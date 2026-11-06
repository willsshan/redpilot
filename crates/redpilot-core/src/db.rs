//! 本地 SQLite 缓存（WAL 模式，桌面端与 stdio MCP 进程可同时访问）。

use crate::models::{Analysis, Issue, IssueChange, IssueFilter, IssueView, Plan, ResolutionRecord};
use anyhow::Result;
use chrono::{Local, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS issues (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL,
    project_name TEXT NOT NULL,
    tracker_name TEXT NOT NULL DEFAULT '',
    status_id INTEGER NOT NULL,
    status_name TEXT NOT NULL,
    priority_id INTEGER NOT NULL DEFAULT 0,
    priority_name TEXT NOT NULL DEFAULT '',
    author_id INTEGER,
    author_name TEXT NOT NULL DEFAULT '',
    assigned_to_id INTEGER,
    assigned_to_name TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    done_ratio INTEGER NOT NULL DEFAULT 0,
    due_date TEXT,
    created_on TEXT NOT NULL,
    updated_on TEXT NOT NULL,
    in_inbox INTEGER NOT NULL DEFAULT 1,
    read_updated_on TEXT,
    first_seen_at TEXT NOT NULL,
    synced_at TEXT NOT NULL,
    detail_json TEXT,
    detail_updated_on TEXT
);
CREATE INDEX IF NOT EXISTS idx_issues_inbox ON issues(in_inbox, updated_on);

CREATE TABLE IF NOT EXISTS analyses (
    issue_id INTEGER PRIMARY KEY,
    rule_category TEXT,
    ai_category TEXT,
    ai_module TEXT,
    urgency TEXT,
    difficulty INTEGER,
    est_hours REAL,
    summary TEXT,
    missing_info TEXT,
    follow_up TEXT,
    suggestion TEXT,
    ai_issue_updated_on TEXT,
    ai_updated_at TEXT,
    manual_category TEXT,
    manual_module TEXT
);

CREATE TABLE IF NOT EXISTS plans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    result_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS resolutions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_id INTEGER NOT NULL,
    subject TEXT NOT NULL,
    notes TEXT NOT NULL,
    status_name TEXT NOT NULL DEFAULT '',
    assigned_to_name TEXT NOT NULL DEFAULT '',
    spent_hours REAL,
    created_at TEXT NOT NULL,
    via TEXT NOT NULL DEFAULT 'app'
);
CREATE INDEX IF NOT EXISTS idx_resolutions_created ON resolutions(created_at);

CREATE TABLE IF NOT EXISTS ai_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    purpose TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS issue_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL DEFAULT '',
    by_author INTEGER NOT NULL DEFAULT 0,
    important INTEGER NOT NULL DEFAULT 0,
    summary TEXT NOT NULL,
    created_on TEXT NOT NULL,
    detected_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_changes_detected ON issue_changes(detected_at);

-- 分派台：留给自己处理、已拆分等不再出现在分派台的任务（任务离开收件箱后清除）
CREATE TABLE IF NOT EXISTS dispatch_marks (
    issue_id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- 派单记录：派给了谁、是否采用了推荐，用于后续跟踪和统计
CREATE TABLE IF NOT EXISTS dispatches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_id INTEGER NOT NULL,
    subject TEXT NOT NULL,
    mode TEXT NOT NULL,
    assignee_ids TEXT NOT NULL,
    assignee_names TEXT NOT NULL,
    child_ids TEXT NOT NULL DEFAULT '',
    due_date TEXT,
    suggested_id INTEGER,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_dispatches_created ON dispatches(created_at);

-- AI 给出的派单建议，任务更新后失效
CREATE TABLE IF NOT EXISTS dispatch_ai (
    issue_id INTEGER PRIMARY KEY,
    issue_updated_on TEXT NOT NULL,
    result_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
"#;

/// 旧版本数据库缺少的列
const MIGRATIONS: [(&str, &str, &str); 3] = [
    ("issues", "change_hint", "TEXT"),
    ("issues", "category_id", "INTEGER"),
    ("issues", "category_name", "TEXT"),
];

/// 同步前任务在本地的状态，用于判断别人改了什么
#[derive(Debug, Clone)]
pub struct PrevIssue {
    pub id: u64,
    pub updated_on: String,
    pub in_inbox: bool,
    pub status_id: u64,
    pub priority_id: u64,
    pub due_date: Option<String>,
}

#[derive(Debug, Default)]
pub struct SyncDiff {
    pub new_ids: Vec<u64>,
    pub updated: Vec<PrevIssue>,
    pub removed: usize,
}

const VIEW_COLUMNS: &str = "i.id, i.project_id, i.project_name, i.tracker_name, i.status_id, i.status_name,
    i.priority_id, i.priority_name, i.author_id, i.author_name, i.assigned_to_id, i.assigned_to_name,
    i.subject, i.description, i.done_ratio, i.due_date, i.created_on, i.updated_on, i.in_inbox,
    i.read_updated_on,
    a.rule_category, a.ai_category, a.ai_module, a.urgency, a.difficulty, a.est_hours, a.summary,
    a.missing_info, a.follow_up, a.suggestion, a.ai_issue_updated_on, a.manual_category, a.manual_module,
    i.change_hint, i.category_id, i.category_name";

/// 优先级名称里带这些字的视为高优先级
pub const HIGH_PRIORITY_WORDS: [&str; 6] = ["高", "紧急", "立刻", "urgent", "high", "immediate"];

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        for (table, column, ty) in MIGRATIONS {
            let exists: bool = conn
                .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
                .exists([column])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ty}"))?;
            }
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO kv(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 写入一批同步结果，返回新任务和有更新任务的旧状态。不在本次结果中的任务移出收件箱。
    pub fn apply_sync(&self, issues: &[Issue]) -> Result<SyncDiff> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now_rfc3339();
        let mut diff = SyncDiff::default();
        for is in issues {
            let prev: Option<PrevIssue> = tx
                .query_row(
                    "SELECT updated_on, in_inbox, status_id, priority_id, due_date FROM issues WHERE id = ?1",
                    [is.id],
                    |r| {
                        Ok(PrevIssue {
                            id: is.id,
                            updated_on: r.get(0)?,
                            in_inbox: r.get::<_, i64>(1)? == 1,
                            status_id: r.get(2)?,
                            priority_id: r.get(3)?,
                            due_date: r.get(4)?,
                        })
                    },
                )
                .optional()?;
            match prev {
                None => diff.new_ids.push(is.id),
                Some(p) if p.updated_on != is.updated_on || !p.in_inbox => diff.updated.push(p),
                _ => {}
            }
            tx.execute(
                "INSERT INTO issues (id, project_id, project_name, tracker_name, status_id, status_name,
                    priority_id, priority_name, author_id, author_name, assigned_to_id, assigned_to_name,
                    subject, description, done_ratio, due_date, created_on, updated_on, in_inbox,
                    first_seen_at, synced_at, category_id, category_name)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,1,?19,?19,?20,?21)
                 ON CONFLICT(id) DO UPDATE SET
                    project_id=excluded.project_id, project_name=excluded.project_name,
                    tracker_name=excluded.tracker_name, status_id=excluded.status_id,
                    status_name=excluded.status_name, priority_id=excluded.priority_id,
                    priority_name=excluded.priority_name, author_id=excluded.author_id,
                    author_name=excluded.author_name, assigned_to_id=excluded.assigned_to_id,
                    assigned_to_name=excluded.assigned_to_name, subject=excluded.subject,
                    description=excluded.description, done_ratio=excluded.done_ratio,
                    due_date=excluded.due_date, created_on=excluded.created_on,
                    updated_on=excluded.updated_on, in_inbox=1, synced_at=excluded.synced_at,
                    category_id=excluded.category_id, category_name=excluded.category_name",
                params![
                    is.id,
                    is.project.id,
                    is.project.name,
                    is.tracker.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
                    is.status.id,
                    is.status.name,
                    is.priority.as_ref().map(|p| p.id).unwrap_or(0),
                    is.priority.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                    is.author.as_ref().map(|a| a.id),
                    is.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                    is.assigned_to.as_ref().map(|a| a.id),
                    is.assigned_to.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                    is.subject,
                    is.description.clone().unwrap_or_default(),
                    is.done_ratio.unwrap_or(0),
                    is.due_date,
                    is.created_on,
                    is.updated_on,
                    now,
                    is.category.as_ref().map(|c| c.id),
                    is.category.as_ref().map(|c| c.name.clone()),
                ],
            )?;
        }
        let ids: Vec<String> = issues.iter().map(|i| i.id.to_string()).collect();
        diff.removed = if ids.is_empty() {
            tx.execute("UPDATE issues SET in_inbox = 0 WHERE in_inbox = 1", [])?
        } else {
            tx.execute(
                &format!("UPDATE issues SET in_inbox = 0 WHERE in_inbox = 1 AND id NOT IN ({})", ids.join(",")),
                [],
            )?
        };
        tx.commit()?;
        Ok(diff)
    }

    /// 自己通过 RedPilot 修改任务后，用最新数据更新本地缓存并标记已读。
    /// 这样下次同步不会把自己的修改当成新变化，AI 分析也不会因此被标记为过期。
    pub fn apply_own_update(&self, is: &Issue) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let old: Option<String> = tx
            .query_row("SELECT updated_on FROM issues WHERE id = ?1", [is.id], |r| r.get(0))
            .optional()?;
        let Some(old) = old else { return Ok(()) };
        tx.execute(
            "UPDATE issues SET status_id = ?2, status_name = ?3, assigned_to_id = ?4, assigned_to_name = ?5,
                done_ratio = ?6, updated_on = ?7, read_updated_on = ?7, detail_json = ?8, detail_updated_on = ?7,
                due_date = ?9, category_id = ?10, category_name = ?11
             WHERE id = ?1",
            params![
                is.id,
                is.status.id,
                is.status.name,
                is.assigned_to.as_ref().map(|a| a.id),
                is.assigned_to.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                is.done_ratio.unwrap_or(0),
                is.updated_on,
                serde_json::to_string(is)?,
                is.due_date,
                is.category.as_ref().map(|c| c.id),
                is.category.as_ref().map(|c| c.name.clone()),
            ],
        )?;
        tx.execute(
            "UPDATE analyses SET ai_issue_updated_on = ?2 WHERE issue_id = ?1 AND ai_issue_updated_on = ?3",
            params![is.id, is.updated_on, old],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_out_of_inbox(&self, id: u64) -> Result<()> {
        self.lock().execute("UPDATE issues SET in_inbox = 0 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn mark_read(&self, id: u64) -> Result<()> {
        self.lock()
            .execute("UPDATE issues SET read_updated_on = updated_on, change_hint = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_change_hint(&self, id: u64, hint: &str) -> Result<()> {
        self.lock()
            .execute("UPDATE issues SET change_hint = ?2 WHERE id = ?1", params![id, hint])?;
        Ok(())
    }

    pub fn add_changes(&self, changes: &[IssueChange]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now_rfc3339();
        for c in changes {
            tx.execute(
                "INSERT INTO issue_changes(issue_id, kind, actor, by_author, important, summary, created_on, detected_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![c.issue_id, c.kind, c.actor, c.by_author, c.important, c.summary, c.created_on, now],
            )?;
        }
        // 只保留最近 90 天
        let cutoff = (Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        tx.execute("DELETE FROM issue_changes WHERE detected_at < ?1", [cutoff])?;
        tx.commit()?;
        Ok(())
    }

    pub fn recent_changes(&self, limit: usize) -> Result<Vec<IssueChange>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT c.issue_id, COALESCE(i.subject, ''), c.kind, c.actor, c.by_author, c.important, c.summary, c.created_on
             FROM issue_changes c LEFT JOIN issues i ON i.id = c.issue_id
             ORDER BY c.created_on DESC, c.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(IssueChange {
                issue_id: r.get(0)?,
                subject: r.get(1)?,
                kind: r.get(2)?,
                actor: r.get(3)?,
                by_author: r.get(4)?,
                important: r.get(5)?,
                summary: r.get(6)?,
                created_on: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn save_detail(&self, issue: &Issue) -> Result<()> {
        self.lock().execute(
            "UPDATE issues SET detail_json = ?2, detail_updated_on = ?3 WHERE id = ?1",
            params![issue.id, serde_json::to_string(issue)?, issue.updated_on],
        )?;
        Ok(())
    }

    /// 取缓存的详情（含备注），仅当与列表中的 updated_on 一致时有效
    pub fn cached_detail(&self, id: u64) -> Result<Option<Issue>> {
        let row: Option<(Option<String>, Option<String>, String)> = self
            .lock()
            .query_row(
                "SELECT detail_json, detail_updated_on, updated_on FROM issues WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((Some(json), Some(du), u)) if du == u => serde_json::from_str(&json).ok(),
            _ => None,
        })
    }

    pub fn list_views(&self, f: &IssueFilter) -> Result<Vec<IssueView>> {
        let mut sql = format!(
            "SELECT {VIEW_COLUMNS} FROM issues i LEFT JOIN analyses a ON a.issue_id = i.id WHERE 1=1"
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !f.include_done {
            sql.push_str(" AND i.in_inbox = 1");
        }
        if let Some(p) = f.project_id {
            sql.push_str(" AND i.project_id = ?");
            args.push(Box::new(p));
        }
        if let Some(c) = f.category.as_ref().filter(|c| !c.is_empty()) {
            sql.push_str(" AND COALESCE(a.manual_category, a.ai_category, a.rule_category, '其他') = ?");
            args.push(Box::new(c.clone()));
        }
        if let Some(k) = f.keyword.as_ref().filter(|k| !k.trim().is_empty()) {
            let k = k.trim();
            if let Some(id) = k.trim_start_matches('#').parse::<u64>().ok() {
                sql.push_str(" AND i.id = ?");
                args.push(Box::new(id));
            } else {
                sql.push_str(" AND (i.subject LIKE ? OR i.description LIKE ? OR i.author_name LIKE ? OR a.summary LIKE ?)");
                let like = format!("%{k}%");
                for _ in 0..4 {
                    args.push(Box::new(like.clone()));
                }
            }
        }
        if f.unread_only {
            sql.push_str(" AND (i.read_updated_on IS NULL OR i.read_updated_on != i.updated_on)");
        }
        sql.push_str(" ORDER BY i.priority_id DESC, i.updated_on DESC");
        if let Some(l) = f.limit {
            sql.push_str(&format!(" LIMIT {l}"));
        }
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())), row_to_view)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_view(&self, id: u64) -> Result<Option<IssueView>> {
        let sql = format!("SELECT {VIEW_COLUMNS} FROM issues i LEFT JOIN analyses a ON a.issue_id = i.id WHERE i.id = ?1");
        Ok(self.lock().query_row(&sql, [id], row_to_view).optional()?)
    }

    pub fn set_rule_category(&self, id: u64, category: Option<&str>) -> Result<()> {
        self.lock().execute(
            "INSERT INTO analyses(issue_id, rule_category) VALUES(?1, ?2)
             ON CONFLICT(issue_id) DO UPDATE SET rule_category = excluded.rule_category",
            params![id, category],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_ai_analysis(&self, id: u64, issue_updated_on: &str, a: &Analysis) -> Result<()> {
        self.lock().execute(
            "INSERT INTO analyses(issue_id, ai_category, ai_module, urgency, difficulty, est_hours, summary,
                missing_info, follow_up, suggestion, ai_issue_updated_on, ai_updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(issue_id) DO UPDATE SET
                ai_category=excluded.ai_category, ai_module=excluded.ai_module, urgency=excluded.urgency,
                difficulty=excluded.difficulty, est_hours=excluded.est_hours, summary=excluded.summary,
                missing_info=excluded.missing_info, follow_up=excluded.follow_up,
                suggestion=excluded.suggestion, ai_issue_updated_on=excluded.ai_issue_updated_on,
                ai_updated_at=excluded.ai_updated_at",
            params![
                id,
                a.category,
                a.module,
                a.urgency,
                a.difficulty,
                a.est_hours,
                a.summary,
                serde_json::to_string(&a.missing_info)?,
                a.follow_up,
                a.suggestion,
                issue_updated_on,
                now_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn set_manual(&self, id: u64, category: Option<&str>, module: Option<&str>) -> Result<()> {
        self.lock().execute(
            "INSERT INTO analyses(issue_id, manual_category, manual_module) VALUES(?1, ?2, ?3)
             ON CONFLICT(issue_id) DO UPDATE SET manual_category = excluded.manual_category,
                manual_module = excluded.manual_module",
            params![id, category, module],
        )?;
        Ok(())
    }

    /// 最近的人工修正，作为 few-shot 样例让 AI 学习本团队的分类习惯
    pub fn recent_corrections(&self, limit: usize) -> Result<Vec<(String, String, Option<String>)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT i.subject, a.manual_category, a.manual_module FROM analyses a JOIN issues i ON i.id = a.issue_id
             WHERE a.manual_category IS NOT NULL ORDER BY i.updated_on DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn save_plan(&self, plan: &Plan) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO plans(created_at, result_json) VALUES(?1, ?2)",
            params![plan.created_at, serde_json::to_string(plan)?],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn latest_plan(&self) -> Result<Option<Plan>> {
        let row: Option<(i64, String)> = self
            .lock()
            .query_row("SELECT id, result_json FROM plans ORDER BY id DESC LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row.and_then(|(id, j)| {
            let mut p: Plan = serde_json::from_str(&j).ok()?;
            p.id = id;
            Some(p)
        }))
    }

    pub fn add_resolution(&self, r: &ResolutionRecord) -> Result<()> {
        self.lock().execute(
            "INSERT INTO resolutions(issue_id, subject, notes, status_name, assigned_to_name, spent_hours, created_at, via)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![r.issue_id, r.subject, r.notes, r.status_name, r.assigned_to_name, r.spent_hours, r.created_at, r.via],
        )?;
        Ok(())
    }

    pub fn resolutions_since(&self, since: &str) -> Result<Vec<ResolutionRecord>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT issue_id, subject, notes, status_name, assigned_to_name, spent_hours, created_at, via
             FROM resolutions WHERE created_at >= ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([since], |r| {
            Ok(ResolutionRecord {
                issue_id: r.get(0)?,
                subject: r.get(1)?,
                notes: r.get(2)?,
                status_name: r.get(3)?,
                assigned_to_name: r.get(4)?,
                spent_hours: r.get(5)?,
                created_at: r.get(6)?,
                via: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 相似度检索语料：全部已缓存任务（标题 + 描述）以及最近一次处置说明
    pub fn similarity_corpus(&self) -> Result<Vec<(u64, String, String, String, Option<String>)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT i.id, i.subject, i.description, i.status_name,
                (SELECT notes FROM resolutions r WHERE r.issue_id = i.id ORDER BY r.id DESC LIMIT 1)
             FROM issues i ORDER BY i.updated_on DESC LIMIT 3000",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn record_usage(&self, purpose: &str, model: &str, prompt: u64, completion: u64) -> Result<()> {
        self.lock().execute(
            "INSERT INTO ai_usage(created_at, purpose, model, prompt_tokens, completion_tokens) VALUES(?1,?2,?3,?4,?5)",
            params![now_rfc3339(), purpose, model, prompt as i64, completion as i64],
        )?;
        Ok(())
    }

    /// 某时间之后的 AI 用量：(输入 token, 输出 token)
    pub fn usage_since(&self, since: &str) -> Result<(u64, u64)> {
        let (p, c): (i64, i64) = self.lock().query_row(
            "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0) FROM ai_usage WHERE created_at >= ?1",
            [since],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((p as u64, c as u64))
    }

    /// 最近 N 天每天的回派数量（按本地日期）
    pub fn daily_resolved(&self, days: i64) -> Result<Vec<(String, usize)>> {
        let since = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let records = self.resolutions_since(&since)?;
        let today = Local::now().date_naive();
        let mut out = Vec::new();
        for d in (0..days).rev() {
            let day = today - chrono::Duration::days(d);
            let key = day.format("%m-%d").to_string();
            let count = records
                .iter()
                .filter(|r| {
                    chrono::DateTime::parse_from_rfc3339(&r.created_at)
                        .map(|t| t.with_timezone(&Local).date_naive() == day)
                        .unwrap_or(false)
                })
                .count();
            out.push((key, count));
        }
        Ok(out)
    }

    // ---------------- 分派台 ----------------

    pub fn set_dispatch_mark(&self, id: u64, kind: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO dispatch_marks(issue_id, kind, created_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(issue_id) DO UPDATE SET kind = excluded.kind, created_at = excluded.created_at",
            params![id, kind, now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn clear_dispatch_mark(&self, id: u64) -> Result<()> {
        self.lock().execute("DELETE FROM dispatch_marks WHERE issue_id = ?1", [id])?;
        Ok(())
    }

    /// 不再出现在分派台的任务。已离开收件箱的任务清掉标记，再次回到我这里时重新进入分派台。
    pub fn dispatch_marks(&self) -> Result<std::collections::HashMap<u64, String>> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM dispatch_marks WHERE issue_id IN (SELECT id FROM issues WHERE in_inbox = 0)",
            [],
        )?;
        let mut stmt = conn.prepare("SELECT issue_id, kind FROM dispatch_marks")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_dispatch(
        &self,
        issue_id: u64,
        subject: &str,
        mode: &str,
        assignees: &[(u64, String)],
        child_ids: &[u64],
        due_date: Option<&str>,
        suggested_id: Option<u64>,
    ) -> Result<()> {
        let ids: Vec<String> = assignees.iter().map(|a| a.0.to_string()).collect();
        let names: Vec<&str> = assignees.iter().map(|a| a.1.as_str()).collect();
        let children: Vec<String> = child_ids.iter().map(u64::to_string).collect();
        self.lock().execute(
            "INSERT INTO dispatches(issue_id, subject, mode, assignee_ids, assignee_names, child_ids, due_date, suggested_id, created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![issue_id, subject, mode, ids.join(","), names.join("、"), children.join(","), due_date, suggested_id, now_rfc3339()],
        )?;
        Ok(())
    }

    /// 最近派单次数：成员 ID → 次数
    pub fn dispatch_counts(&self, days: i64) -> Result<std::collections::HashMap<u64, usize>> {
        let since = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT assignee_ids FROM dispatches WHERE created_at >= ?1")?;
        let rows = stmt.query_map([since], |r| r.get::<_, String>(0))?;
        let mut out = std::collections::HashMap::new();
        for ids in rows {
            for id in ids?.split(',').filter_map(|x| x.parse::<u64>().ok()) {
                *out.entry(id).or_default() += 1;
            }
        }
        Ok(out)
    }

    pub fn save_dispatch_ai(&self, id: u64, issue_updated_on: &str, json: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO dispatch_ai(issue_id, issue_updated_on, result_json, created_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(issue_id) DO UPDATE SET issue_updated_on = excluded.issue_updated_on,
                result_json = excluded.result_json, created_at = excluded.created_at",
            params![id, issue_updated_on, json, now_rfc3339()],
        )?;
        Ok(())
    }

    /// AI 派单建议：任务 ID → (生成时任务的 updated_on, JSON)
    pub fn dispatch_ai(&self, ids: &[u64]) -> Result<std::collections::HashMap<u64, (String, String)>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let list: Vec<String> = ids.iter().map(u64::to_string).collect();
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT issue_id, issue_updated_on, result_json FROM dispatch_ai WHERE issue_id IN ({})",
            list.join(",")
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, u64>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2)?))))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

fn row_to_view(r: &Row) -> rusqlite::Result<IssueView> {
    let updated_on: String = r.get(17)?;
    let read_updated_on: Option<String> = r.get(19)?;
    let rule_category: Option<String> = r.get(20)?;
    let ai_category: Option<String> = r.get(21)?;
    let ai_module: Option<String> = r.get(22)?;
    let ai_issue_updated_on: Option<String> = r.get(30)?;
    let manual_category: Option<String> = r.get(31)?;
    let manual_module: Option<String> = r.get(32)?;
    let missing: Option<String> = r.get(27)?;
    let due_date: Option<String> = r.get(15)?;

    let (category, source) = if let Some(c) = &manual_category {
        (Some(c.clone()), Some("manual"))
    } else if let Some(c) = &ai_category {
        (Some(c.clone()), Some("ai"))
    } else if let Some(c) = &rule_category {
        (Some(c.clone()), Some("rule"))
    } else {
        (None, None)
    };
    let overdue = due_date
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d < Local::now().date_naive())
        .unwrap_or(false);

    Ok(IssueView {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        tracker_name: r.get(3)?,
        status_id: r.get(4)?,
        status_name: r.get(5)?,
        priority_id: r.get(6)?,
        priority_name: r.get(7)?,
        author_id: r.get(8)?,
        author_name: r.get(9)?,
        assigned_to_id: r.get(10)?,
        assigned_to_name: r.get(11)?,
        subject: r.get(12)?,
        description: r.get(13)?,
        done_ratio: r.get(14)?,
        due_date,
        created_on: r.get(16)?,
        in_inbox: r.get::<_, i64>(18)? == 1,
        unread: read_updated_on.as_deref() != Some(updated_on.as_str()),
        overdue,
        change_hint: r.get(33)?,
        redmine_category_id: r.get(34)?,
        redmine_category: r.get(35)?,
        analysis: Analysis {
            category,
            category_source: source.map(str::to_string),
            rule_category,
            module: manual_module.or(ai_module),
            urgency: r.get(23)?,
            difficulty: r.get(24)?,
            est_hours: r.get(25)?,
            summary: r.get(26)?,
            missing_info: missing.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or_default(),
            follow_up: r.get(28)?,
            suggestion: r.get(29)?,
            ai_analyzed: ai_issue_updated_on.is_some(),
            ai_stale: ai_issue_updated_on.as_deref().is_some_and(|u| u != updated_on),
        },
        updated_on,
    })
}
