//! 业务服务层：桌面端命令与 MCP 工具都调用这里，保证两边行为一致。

use crate::ai::{AiClient, Message, Usage};
use crate::analysis::{self, Corpus};
use crate::config::{self, AppConfig, Paths, TextFormat};
use crate::db::{self, Db, PrevIssue};
use crate::models::*;
use crate::project;
use crate::redmine::{truncate, IssueCategory, IssueQuery, IssueUpdate, NewIssue, RedmineClient};
use crate::secrets::{SecretKind, SecretStore};
use crate::{coded, mask, prompts};
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Semaphore};

/// 单个附件大小上限（Redmine 自身的限制通常更小，默认 5MB）
const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;
/// 每次同步最多为多少个有更新的任务拉取沟通记录
const CHANGE_DETAIL_LIMIT: usize = 30;
/// 可在界面中预览的附件大小上限
const MAX_PREVIEW_BYTES: u64 = 30 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    SyncStarted,
    SyncFinished { report: SyncReport },
    SyncFailed { message: String },
    AnalysisStarted { count: usize },
    AnalysisFinished { report: AnalyzeReport },
    AnalysisFailed { message: String },
    IssuesChanged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    pub redmine: bool,
    pub ai: bool,
}

pub struct Service {
    pub paths: Paths,
    pub db: Db,
    config: RwLock<AppConfig>,
    secrets: SecretStore,
    secret_cache: RwLock<HashMap<SecretKind, Option<String>>>,
    redmine_client: RwLock<Option<RedmineClient>>,
    me: RwLock<Option<User>>,
    statuses: RwLock<Vec<NamedId>>,
    /// 项目成员缓存：项目 ID → (获取时间, 成员)
    assignees: RwLock<HashMap<u64, (std::time::Instant, Vec<Assignee>)>>,
    /// 项目总览缓存：项目 ID → (获取时间, 总览)
    overviews: RwLock<HashMap<u64, (std::time::Instant, ProjectOverview)>>,
    sync_lock: AsyncMutex<()>,
    events: broadcast::Sender<AppEvent>,
}

fn read<T: Clone>(l: &RwLock<T>) -> T {
    l.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn write<T>(l: &RwLock<T>, v: T) {
    *l.write().unwrap_or_else(|e| e.into_inner()) = v;
}

impl Service {
    pub fn open(paths: Paths) -> Result<Arc<Self>> {
        let cfg = config::load(&paths.config_file())?;
        let db = Db::open(&paths.db_file())?;
        cleanup_old_entries(&paths.staging_dir(), Duration::from_secs(24 * 3600));
        cleanup_old_entries(&paths.attachment_cache_dir(), Duration::from_secs(30 * 24 * 3600));
        let (events, _) = broadcast::channel(64);
        Ok(Arc::new(Self {
            secrets: SecretStore::new(paths.secrets_file()),
            paths,
            db,
            config: RwLock::new(cfg),
            secret_cache: RwLock::new(HashMap::new()),
            redmine_client: RwLock::new(None),
            me: RwLock::new(None),
            statuses: RwLock::new(Vec::new()),
            assignees: RwLock::new(HashMap::new()),
            overviews: RwLock::new(HashMap::new()),
            sync_lock: AsyncMutex::new(()),
            events,
        }))
    }

    pub fn open_default() -> Result<Arc<Self>> {
        Self::open(Paths::default_location()?)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    fn emit(&self, e: AppEvent) {
        let _ = self.events.send(e);
    }

    // ---------------- 配置与密钥 ----------------

    pub fn config(&self) -> AppConfig {
        read(&self.config)
    }

    /// 从磁盘重新加载配置（stdio MCP 进程用来感知桌面端的修改）
    pub fn reload_config(&self) -> Result<()> {
        let cfg = config::load(&self.paths.config_file())?;
        let old = self.config();
        if old.redmine.base_url != cfg.redmine.base_url
            || old.redmine.proxy != cfg.redmine.proxy
            || old.redmine.accept_invalid_certs != cfg.redmine.accept_invalid_certs
        {
            self.reset_redmine_cache();
        }
        write(&self.config, cfg);
        Ok(())
    }

    pub fn save_config(&self, mut cfg: AppConfig) -> Result<()> {
        let old = self.config();
        if cfg.mcp.token.trim().is_empty() {
            cfg.mcp.token = old.mcp.token.clone();
        }
        cfg.ai.batch_size = cfg.ai.batch_size.clamp(1, 20);
        config::save(&self.paths.config_file(), &cfg)?;
        let redmine_changed = old.redmine.base_url != cfg.redmine.base_url
            || old.redmine.proxy != cfg.redmine.proxy
            || old.redmine.accept_invalid_certs != cfg.redmine.accept_invalid_certs;
        let rules_changed = serde_json::to_string(&old.rules)? != serde_json::to_string(&cfg.rules)?;
        write(&self.config, cfg);
        if redmine_changed {
            self.reset_redmine_cache();
        }
        if rules_changed {
            self.reapply_rules()?;
            self.emit(AppEvent::IssuesChanged);
        }
        Ok(())
    }

    pub fn regenerate_mcp_token(&self) -> Result<String> {
        let mut cfg = self.config();
        cfg.mcp.token = config::new_token();
        let token = cfg.mcp.token.clone();
        self.save_config(cfg)?;
        Ok(token)
    }

    fn reset_redmine_cache(&self) {
        write(&self.redmine_client, None);
        write(&self.me, None);
        write(&self.statuses, Vec::new());
        write(&self.assignees, HashMap::new());
        write(&self.overviews, HashMap::new());
    }

    pub fn secret(&self, kind: SecretKind) -> Result<Option<String>> {
        if let Some(v) = self.secret_cache.read().unwrap_or_else(|e| e.into_inner()).get(&kind) {
            return Ok(v.clone());
        }
        let v = self.secrets.get(kind)?;
        self.secret_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind, v.clone());
        Ok(v)
    }

    pub fn set_secret(&self, kind: SecretKind, value: &str) -> Result<()> {
        self.secrets.set(kind, value)?;
        let v = Some(value.trim().to_string()).filter(|s| !s.is_empty());
        self.secret_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind, v);
        if kind == SecretKind::RedmineApiKey {
            self.reset_redmine_cache();
        }
        Ok(())
    }

    pub fn secret_status(&self) -> SecretStatus {
        SecretStatus {
            redmine: self.secret(SecretKind::RedmineApiKey).ok().flatten().is_some(),
            ai: self.secret(SecretKind::AiApiKey).ok().flatten().is_some(),
        }
    }

    // ---------------- 客户端 ----------------

    pub fn redmine(&self) -> Result<RedmineClient> {
        if let Some(c) = read(&self.redmine_client) {
            return Ok(c);
        }
        let key = self.secret(SecretKind::RedmineApiKey)?.unwrap_or_default();
        let client = RedmineClient::new(&self.config().redmine, &key)?;
        write(&self.redmine_client, Some(client.clone()));
        Ok(client)
    }

    fn ai_client(&self, planning: bool) -> Result<AiClient> {
        let cfg = self.config();
        if !cfg.ai.enabled {
            return Err(coded("AI_DISABLED", "AI 功能已在设置中关闭"));
        }
        let key = self.secret(SecretKind::AiApiKey)?.unwrap_or_default();
        if key.is_empty() && cfg.ai.base_url.contains("deepseek.com") {
            return Err(coded("AI_NO_KEY", "尚未配置 DeepSeek API Key"));
        }
        let model = if planning && !cfg.ai.planning_model.trim().is_empty() {
            &cfg.ai.planning_model
        } else {
            &cfg.ai.model
        };
        AiClient::new(&cfg.ai.base_url, &key, model, cfg.ai.json_mode, cfg.ai.temperature, &cfg.redmine.proxy)
    }

    pub fn ai_ready(&self) -> bool {
        self.ai_client(false).is_ok()
    }

    fn record_usage(&self, purpose: &str, model: &str, u: Usage) {
        if let Err(e) = self.db.record_usage(purpose, model, u.prompt_tokens, u.completion_tokens) {
            tracing::warn!("记录 AI 用量失败: {e}");
        }
    }

    fn prep_for_ai(&self, text: &str) -> String {
        if self.config().ai.mask_sensitive {
            mask::mask(text)
        } else {
            text.to_string()
        }
    }

    pub async fn test_redmine(&self) -> Result<User> {
        self.reset_redmine_cache();
        self.me().await
    }

    pub async fn test_ai(&self) -> Result<String> {
        let ai = self.ai_client(false)?;
        let (text, usage) = ai
            .chat(&[Message::user("请只回复：连接成功")], false)
            .await?;
        self.record_usage("test", &ai.model, usage);
        Ok(format!("{}（模型：{}）", text.trim(), ai.model))
    }

    pub async fn me(&self) -> Result<User> {
        if let Some(u) = read(&self.me) {
            return Ok(u);
        }
        let u = self.redmine()?.current_user().await?;
        write(&self.me, Some(u.clone()));
        Ok(u)
    }

    pub async fn statuses(&self) -> Result<Vec<NamedId>> {
        let cached = read(&self.statuses);
        if !cached.is_empty() {
            return Ok(cached);
        }
        let s = self.redmine()?.statuses().await?;
        write(&self.statuses, s.clone());
        Ok(s)
    }

    /// 项目中可指派的成员（缓存 10 分钟）。没有权限查看成员时返回空列表。
    pub async fn assignees(&self, project_id: u64) -> Result<Vec<Assignee>> {
        if let Some((at, list)) = self.assignees.read().unwrap_or_else(|e| e.into_inner()).get(&project_id) {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(list.clone());
            }
        }
        let mut list = match self.redmine()?.memberships(project_id).await {
            Ok(l) => l,
            Err(e) if e.to_string().contains("REDMINE_FORBIDDEN") || e.to_string().contains("REDMINE_NOT_FOUND") => {
                tracing::info!("无法读取项目 {project_id} 的成员: {e}");
                Vec::new()
            }
            Err(e) => return Err(e),
        };
        list.sort_by(|a, b| a.is_group.cmp(&b.is_group).then_with(|| a.name.cmp(&b.name)));
        self.assignees
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project_id, (std::time::Instant::now(), list.clone()));
        Ok(list)
    }

    /// 任务所属项目的可指派成员
    pub async fn assignees_for_issue(&self, issue_id: u64) -> Result<Vec<Assignee>> {
        let project_id = match self.db.get_view(issue_id)? {
            Some(v) => v.project_id,
            None => self.detail(issue_id, false).await?.project.id,
        };
        self.assignees(project_id).await
    }

    pub async fn activities(&self) -> Result<Vec<NamedId>> {
        self.redmine()?.activities().await
    }

    pub async fn projects(&self) -> Result<Vec<Project>> {
        self.redmine()?.projects().await
    }

    // ---------------- 同步 ----------------

    pub async fn sync(self: &Arc<Self>) -> Result<SyncReport> {
        let _guard = self.sync_lock.lock().await;
        self.emit(AppEvent::SyncStarted);
        let res = self.sync_inner().await;
        match &res {
            Ok(r) => self.emit(AppEvent::SyncFinished { report: r.clone() }),
            Err(e) => self.emit(AppEvent::SyncFailed { message: e.to_string() }),
        }
        res
    }

    async fn sync_inner(self: &Arc<Self>) -> Result<SyncReport> {
        let cfg = self.config();
        let rm = self.redmine()?;
        let s = &cfg.sync;
        if !s.assigned_to_me && !s.watched && s.query_id.is_none() {
            return Err(coded("SYNC_SCOPE", "请在设置中至少选择一种同步范围"));
        }
        let project = Some(s.project.trim().to_string()).filter(|p| !p.is_empty());
        let base = IssueQuery {
            project,
            status: "open".into(),
            ..Default::default()
        };
        let mut all: HashMap<u64, Issue> = HashMap::new();
        if s.assigned_to_me {
            let q = IssueQuery { assigned_to_me: true, ..base.clone() };
            all.extend(rm.list_issues(&q).await?.into_iter().map(|i| (i.id, i)));
        }
        if s.watched {
            let q = IssueQuery { watched_by_me: true, ..base.clone() };
            all.extend(rm.list_issues(&q).await?.into_iter().map(|i| (i.id, i)));
        }
        if let Some(qid) = s.query_id {
            let q = IssueQuery { query_id: Some(qid), ..base.clone() };
            all.extend(rm.list_issues(&q).await?.into_iter().map(|i| (i.id, i)));
        }
        let issues: Vec<Issue> = all.into_values().collect();
        let diff = self.db.apply_sync(&issues)?;
        let new_ids = diff.new_ids;
        let updated_ids: Vec<u64> = diff.updated.iter().map(|p| p.id).collect();

        let changed: HashSet<u64> = new_ids.iter().chain(updated_ids.iter()).copied().collect();
        for is in issues.iter().filter(|i| changed.contains(&i.id)) {
            let text = format!("{} {}", is.subject, is.description.as_deref().unwrap_or(""));
            self.db
                .set_rule_category(is.id, analysis::rule_classify(&text, &cfg.rules).as_deref())?;
        }
        let changes = self.detect_changes(&issues, &diff.updated).await;
        if !changes.is_empty() {
            self.db.add_changes(&changes)?;
        }
        let due_soon = self.due_soon(&issues)?;
        let finished_at = db::now_rfc3339();
        self.db.kv_set("last_sync_at", &finished_at)?;

        if s.auto_analyze && !changed.is_empty() && self.ai_ready() {
            let svc = Arc::clone(self);
            let ids: Vec<u64> = changed.iter().copied().collect();
            tokio::spawn(async move {
                if let Err(e) = svc.analyze(Some(ids), false).await {
                    tracing::warn!("自动分析失败: {e}");
                }
            });
        }

        Ok(SyncReport {
            total: issues.len(),
            new_ids,
            updated_ids,
            removed: diff.removed,
            finished_at,
            changes,
            due_soon,
        })
    }

    /// 为有更新的任务拉取沟通记录，找出别人做的变更（回复、调优先级、改截止日期、转回给我）
    async fn detect_changes(self: &Arc<Self>, issues: &[Issue], updated: &[PrevIssue]) -> Vec<IssueChange> {
        if updated.is_empty() {
            return vec![];
        }
        let me = self.me().await.ok().map(|u| u.id);
        let by_id: HashMap<u64, &Issue> = issues.iter().map(|i| (i.id, i)).collect();
        let sem = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for p in updated.iter().take(CHANGE_DETAIL_LIMIT) {
            let svc = Arc::clone(self);
            let sem = Arc::clone(&sem);
            let p = p.clone();
            set.spawn(async move {
                let _permit = sem.acquire().await;
                let r = svc.detail(p.id, false).await;
                (p, r)
            });
        }
        let mut per_issue: Vec<Vec<IssueChange>> = Vec::new();
        while let Some(joined) = set.join_next().await {
            let Ok((prev, res)) = joined else { continue };
            let Some(listed) = by_id.get(&prev.id) else { continue };
            per_issue.push(match res {
                Ok(detail) => changes_from_journals(&prev, &detail, me),
                Err(e) => {
                    tracing::warn!("拉取 #{} 沟通记录失败: {e}", prev.id);
                    changes_from_fields(&prev, listed)
                }
            });
        }
        for prev in updated.iter().skip(CHANGE_DETAIL_LIMIT) {
            if let Some(listed) = by_id.get(&prev.id) {
                per_issue.push(changes_from_fields(prev, listed));
            }
        }
        let mut out = Vec::new();
        for changes in per_issue {
            if let Some(c) = pick_hint(&changes) {
                if let Err(e) = self.db.set_change_hint(c.issue_id, &c.summary) {
                    tracing::warn!("保存变更提示失败: {e}");
                }
            }
            out.extend(changes);
        }
        out.sort_by(|a, b| a.created_on.cmp(&b.created_on));
        out
    }

    /// 今天或明天到期、今天还没提醒过的任务
    fn due_soon(&self, issues: &[Issue]) -> Result<Vec<u64>> {
        let today = chrono::Local::now().date_naive();
        let tomorrow = today + chrono::Duration::days(1);
        let today_s = today.format("%Y-%m-%d").to_string();
        let stored = self.db.kv_get("due_notified")?.unwrap_or_default();
        let mut notified: HashSet<u64> = match stored.split_once(':') {
            Some((day, ids)) if day == today_s => ids.split(',').filter_map(|x| x.parse().ok()).collect(),
            _ => HashSet::new(),
        };
        let mut due: Vec<u64> = issues
            .iter()
            .filter(|i| {
                i.due_date
                    .as_deref()
                    .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .is_some_and(|d| d == today || d == tomorrow)
            })
            .map(|i| i.id)
            .filter(|id| !notified.contains(id))
            .collect();
        due.sort_unstable();
        if !due.is_empty() {
            notified.extend(&due);
            let ids: Vec<String> = notified.iter().map(u64::to_string).collect();
            self.db.kv_set("due_notified", &format!("{today_s}:{}", ids.join(",")))?;
        }
        Ok(due)
    }

    /// 附件内容（带本地缓存）。Redmine 附件内容不会变化，按 ID 缓存即可。
    pub async fn attachment(&self, id: u64) -> Result<Vec<u8>> {
        let dir = self.paths.attachment_cache_dir();
        let file = dir.join(id.to_string());
        if let Ok(bytes) = tokio::fs::read(&file).await {
            return Ok(bytes);
        }
        let bytes = self.redmine()?.download_attachment(id, MAX_PREVIEW_BYTES).await?;
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            tracing::warn!("创建附件缓存目录失败: {e}");
        } else {
            // 先写临时文件再改名，避免并发预览读到写了一半的文件
            let tmp = dir.join(format!("{id}.{}.tmp", uuid::Uuid::new_v4().simple()));
            if tokio::fs::write(&tmp, &bytes).await.is_ok() && tokio::fs::rename(&tmp, &file).await.is_err() {
                let _ = tokio::fs::remove_file(&tmp).await;
            }
        }
        Ok(bytes)
    }

    pub fn recent_changes(&self, limit: usize) -> Result<Vec<IssueChange>> {
        self.db.recent_changes(limit)
    }

    fn reapply_rules(&self) -> Result<()> {
        let rules = self.config().rules;
        for v in self.db.list_views(&IssueFilter { include_done: true, ..Default::default() })? {
            let text = format!("{} {}", v.subject, v.description);
            self.db.set_rule_category(v.id, analysis::rule_classify(&text, &rules).as_deref())?;
        }
        Ok(())
    }

    // ---------------- 查询 ----------------

    pub fn list_issues(&self, f: &IssueFilter) -> Result<Vec<IssueView>> {
        self.db.list_views(f)
    }

    pub fn mark_read(&self, id: u64) -> Result<()> {
        self.db.mark_read(id)
    }

    /// 取任务详情（含沟通记录）。缓存与 Redmine 上的版本一致时直接使用缓存。
    async fn detail(&self, id: u64, refresh: bool) -> Result<Issue> {
        if !refresh {
            if let Some(d) = self.db.cached_detail(id)? {
                return Ok(d);
            }
        }
        let issue = self.redmine()?.get_issue(id).await?;
        self.db.save_detail(&issue)?;
        Ok(issue)
    }

    pub async fn issue_detail(&self, id: u64, refresh: bool) -> Result<IssueDetail> {
        let issue = self.detail(id, refresh).await?;
        let view = match self.db.get_view(id)? {
            Some(v) => v,
            None => view_from_issue(&issue),
        };
        let similar = self.find_similar(id, 5).unwrap_or_default();
        let web_url = self.redmine().map(|r| r.issue_url(id)).unwrap_or_default();
        let children = if issue.children.is_empty() {
            vec![]
        } else {
            self.child_issues(id).await.unwrap_or_else(|e| {
                tracing::warn!("读取 #{id} 的子任务失败: {e}");
                issue
                    .children
                    .iter()
                    .map(|c| ChildIssue {
                        id: c.id,
                        subject: c.subject.clone(),
                        tracker: c.tracker.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
                        ..Default::default()
                    })
                    .collect()
            })
        };
        Ok(IssueDetail { view, issue, similar, web_url, children })
    }

    /// 子任务的当前状态（详情缓存里只有子任务的 ID 和标题）
    async fn child_issues(&self, id: u64) -> Result<Vec<ChildIssue>> {
        let list = self
            .redmine()?
            .list_issues(&IssueQuery { status: "*".into(), extra: vec![("parent_id", id.to_string())], max_results: 100, ..Default::default() })
            .await?;
        let statuses = self.statuses().await.unwrap_or_default();
        let mut out: Vec<ChildIssue> = list.iter().map(|c| child_issue(c, &statuses)).collect();
        out.sort_by_key(|c| c.id);
        Ok(out)
    }

    pub fn find_similar(&self, id: u64, limit: usize) -> Result<Vec<SimilarIssue>> {
        let corpus = self.db.similarity_corpus()?;
        let Some(target) = corpus.iter().position(|c| c.0 == id) else {
            return Ok(vec![]);
        };
        let docs: Vec<String> = corpus
            .iter()
            .map(|(_, s, d, _, n)| {
                format!("{s} {s} {} {}", truncate(d, 300), n.as_deref().map(|n| truncate(n, 200)).unwrap_or_default())
            })
            .collect();
        let c = Corpus::build(&docs);
        Ok(analysis::most_similar(&c, target, limit, 0.18)
            .into_iter()
            .map(|(i, score)| {
                let (id, subject, _, status, notes) = &corpus[i];
                SimilarIssue {
                    id: *id,
                    subject: subject.clone(),
                    status_name: status.clone(),
                    score: (score * 100.0).round() / 100.0,
                    resolution_notes: notes.clone(),
                }
            })
            .collect())
    }

    // ---------------- AI 分析分类 ----------------

    pub async fn analyze(self: &Arc<Self>, ids: Option<Vec<u64>>, force: bool) -> Result<AnalyzeReport> {
        let cfg = self.config();
        let ai = self.ai_client(false)?;
        let views = match &ids {
            Some(ids) => ids.iter().filter_map(|id| self.db.get_view(*id).ok().flatten()).collect::<Vec<_>>(),
            None => self.db.list_views(&IssueFilter::default())?,
        };
        let mut report = AnalyzeReport::default();
        let mut targets = Vec::new();
        for v in views {
            if cfg.ai.excluded_project_ids.contains(&v.project_id) {
                report.excluded.push(v.id);
            } else if v.analysis.ai_analyzed && !v.analysis.ai_stale && !force {
                report.skipped.push(v.id);
            } else {
                targets.push(v.id);
            }
        }
        if targets.is_empty() {
            return Ok(report);
        }
        self.emit(AppEvent::AnalysisStarted { count: targets.len() });

        let res = self.analyze_targets(&ai, &cfg, &targets).await;
        match res {
            Ok(done) => {
                report.analyzed = done;
                self.emit(AppEvent::AnalysisFinished { report: report.clone() });
                self.emit(AppEvent::IssuesChanged);
                Ok(report)
            }
            Err(e) => {
                self.emit(AppEvent::AnalysisFailed { message: e.to_string() });
                Err(e)
            }
        }
    }

    async fn analyze_targets(self: &Arc<Self>, ai: &AiClient, cfg: &AppConfig, targets: &[u64]) -> Result<Vec<u64>> {
        // 1. 并发拉取详情（含沟通记录）
        let sem = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for &id in targets {
            let svc = Arc::clone(self);
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                let _p = sem.acquire().await;
                svc.detail(id, false).await
            });
        }
        let mut issues = Vec::new();
        while let Some(r) = set.join_next().await {
            match r {
                Ok(Ok(i)) => issues.push(i),
                Ok(Err(e)) => tracing::warn!("拉取任务详情失败: {e}"),
                Err(e) => tracing::warn!("任务异常: {e}"),
            }
        }
        if issues.is_empty() {
            return Err(anyhow!("无法获取任务详情，请检查 Redmine 连接"));
        }

        // 2. 分批调用 AI，最多 3 批并发
        let categories = config::categories(cfg);
        let mut system = prompts::classify_system(&categories);
        let corrections = self.db.recent_corrections(12)?;
        if !corrections.is_empty() {
            system.push_str("\n\n本团队过往的人工分类示例（请参考其习惯）：\n");
            for (subject, cat, module) in corrections {
                system.push_str(&format!("- 「{}」→ {}{}\n", truncate(&subject, 40), cat, module.map(|m| format!(" / {m}")).unwrap_or_default()));
            }
        }
        let system = prompts::with_extra(system, &cfg.ai.custom_prompts.classify);
        let sem = Arc::new(Semaphore::new(3));
        let mut set = tokio::task::JoinSet::new();
        for chunk in issues.chunks(cfg.ai.batch_size.max(1)) {
            let body: String = chunk
                .iter()
                .map(|i| self.prep_for_ai(&issue_context(i, 1500, 4)))
                .collect::<Vec<_>>()
                .join("\n\n");
            let msgs = vec![
                Message::system(system.clone()),
                Message::user(format!("请分析以下 {} 个任务：\n\n{body}", chunk.len())),
            ];
            let updated: HashMap<u64, String> = chunk.iter().map(|i| (i.id, i.updated_on.clone())).collect();
            let ai = ai.clone();
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                let _p = sem.acquire().await;
                let r = ai.chat_json::<Value>(&msgs).await;
                (r, updated)
            });
        }
        let mut done = Vec::new();
        let mut last_err = None;
        while let Some(joined) = set.join_next().await {
            let Ok((r, updated)) = joined else { continue };
            match r {
                Ok((v, usage)) => {
                    self.record_usage("analyze", &ai.model, usage);
                    let items = v.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                    for item in items {
                        let Some(id) = item.get("id").and_then(v_u64) else { continue };
                        let Some(updated_on) = updated.get(&id) else { continue };
                        let a = parse_analysis(&item, &categories);
                        self.db.save_ai_analysis(id, updated_on, &a)?;
                        done.push(id);
                    }
                }
                Err(e) => {
                    tracing::warn!("AI 分析批次失败: {e}");
                    last_err = Some(e);
                }
            }
        }
        if done.is_empty() {
            if let Some(e) = last_err {
                return Err(e);
            }
        }
        Ok(done)
    }

    pub fn set_manual_classification(&self, id: u64, category: Option<String>, module: Option<String>) -> Result<()> {
        let c = category.filter(|s| !s.trim().is_empty());
        let m = module.filter(|s| !s.trim().is_empty());
        self.db.set_manual(id, c.as_deref(), m.as_deref())?;
        self.emit(AppEvent::IssuesChanged);
        Ok(())
    }

    // ---------------- AI 合并规划 ----------------

    pub async fn suggest_batches(&self, ids: Option<Vec<u64>>) -> Result<Plan> {
        let cfg = self.config();
        let ai = self.ai_client(true)?;
        let mut views: Vec<IssueView> = match ids {
            Some(ids) => ids.iter().filter_map(|id| self.db.get_view(*id).ok().flatten()).collect(),
            None => self.db.list_views(&IssueFilter::default())?,
        };
        views.retain(|v| !cfg.ai.excluded_project_ids.contains(&v.project_id));
        if views.len() < 2 {
            return Err(coded("PLAN_TOO_FEW", "至少需要 2 个可分析的待办任务才能做合并规划"));
        }
        views.truncate(120);

        // 本地相似度预聚类，作为提示给 AI
        let docs: Vec<String> = views
            .iter()
            .map(|v| {
                format!(
                    "{s} {s} {} {} {}",
                    v.analysis.module.as_deref().unwrap_or(""),
                    v.analysis.summary.as_deref().unwrap_or(""),
                    truncate(&v.description, 200),
                    s = v.subject
                )
            })
            .collect();
        let corpus = Corpus::build(&docs);
        let hints: Vec<String> = analysis::pre_cluster(&corpus, 0.3)
            .into_iter()
            .map(|g| g.iter().map(|&i| format!("#{}", views[i].id)).collect::<Vec<_>>().join(", "))
            .collect();

        let mut body = String::from("待办任务：\n");
        for v in &views {
            let a = &v.analysis;
            body.push_str(&format!(
                "- #{} [{}/{}] 优先级:{} 紧急度:{} 难度:{} 预估:{}h 提出人:{} 截止:{} | {} — {}\n",
                v.id,
                a.category.as_deref().unwrap_or("未分类"),
                a.module.as_deref().unwrap_or("-"),
                v.priority_name,
                a.urgency.as_deref().unwrap_or("-"),
                a.difficulty.map(|d| d.to_string()).unwrap_or("-".into()),
                a.est_hours.map(|h| h.to_string()).unwrap_or("-".into()),
                v.author_name,
                v.due_date.as_deref().unwrap_or("-"),
                v.subject,
                a.summary.clone().unwrap_or_else(|| truncate(&v.description, 120)),
            ));
        }
        if !hints.is_empty() {
            body.push_str("\n本地相似度候选组：\n");
            for h in hints {
                body.push_str(&format!("- {h}\n"));
            }
        }
        body.push_str(&format!("\n今天是 {}。", chrono::Local::now().format("%Y-%m-%d %A")));

        let msgs = vec![
            Message::system(prompts::with_extra(prompts::plan_system(), &cfg.ai.custom_prompts.plan)),
            Message::user(self.prep_for_ai(&body)),
        ];
        let (v, usage) = ai.chat_json::<Value>(&msgs).await?;
        self.record_usage("plan", &ai.model, usage);

        let valid: HashSet<u64> = views.iter().map(|v| v.id).collect();
        let keep = |ids: Vec<u64>| -> Vec<u64> {
            let mut seen = HashSet::new();
            ids.into_iter().filter(|i| valid.contains(i) && seen.insert(*i)).collect()
        };
        let groups = v["groups"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|g| PlanGroup {
                        title: v_str(&g["title"]).unwrap_or_else(|| "可合并处理".into()),
                        issue_ids: keep(v_ids(&g["issueIds"])),
                        reason: v_str(&g["reason"]).unwrap_or_default(),
                        approach: v_str(&g["approach"]).unwrap_or_default(),
                    })
                    .filter(|g| g.issue_ids.len() >= 2)
                    .collect()
            })
            .unwrap_or_default();
        let duplicates = v["duplicates"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|d| PlanDuplicate {
                        issue_ids: keep(v_ids(&d["issueIds"])),
                        reason: v_str(&d["reason"]).unwrap_or_default(),
                    })
                    .filter(|d| d.issue_ids.len() >= 2)
                    .collect()
            })
            .unwrap_or_default();
        let mut order = keep(v_ids(&v["order"]));
        for view in &views {
            if !order.contains(&view.id) {
                order.push(view.id);
            }
        }
        let mut plan = Plan {
            id: 0,
            created_at: db::now_rfc3339(),
            issue_ids: views.iter().map(|v| v.id).collect(),
            groups,
            order,
            duplicates,
            daily_plan: v_str(&v["dailyPlan"]).unwrap_or_default(),
        };
        plan.id = self.db.save_plan(&plan)?;
        Ok(plan)
    }

    pub fn latest_plan(&self) -> Result<Option<Plan>> {
        self.db.latest_plan()
    }

    // ---------------- 处置说明 ----------------

    /// 根据处理要点生成处置说明（不提交）。多个任务时生成一份共用说明。
    pub async fn draft_note(&self, ids: &[u64], key_points: &str) -> Result<String> {
        if ids.is_empty() {
            return Err(anyhow!("请指定任务"));
        }
        let cfg = self.config();
        let ai = self.ai_client(false)?;
        let mut ctx = String::new();
        for &id in ids.iter().take(10) {
            let issue = self.detail(id, false).await?;
            if cfg.ai.excluded_project_ids.contains(&issue.project.id) {
                return Err(coded("AI_EXCLUDED", format!("任务 #{id} 所属项目已设置为不发送给 AI")));
            }
            ctx.push_str(&issue_context(&issue, 800, 2));
            ctx.push_str("\n\n");
        }
        let points = if key_points.trim().is_empty() { "（工程师未填写要点，请根据任务内容写一个待补充的框架）" } else { key_points };
        let msgs = vec![
            Message::system(prompts::with_extra(
                prompts::note_system(cfg.redmine.text_format, &cfg.workflow.note_sections),
                &cfg.ai.custom_prompts.note,
            )),
            Message::user(self.prep_for_ai(&format!(
                "{ctx}工程师的处理要点：\n{points}\n\n{}",
                if ids.len() > 1 { "这些任务是合并处理的，请写一份可通用于每个任务的说明。" } else { "" }
            ))),
        ];
        let (text, usage) = ai.chat(&msgs, false).await?;
        self.record_usage("note", &ai.model, usage);
        Ok(strip_fence(&text))
    }

    // ---------------- 写操作：回派 / 追问 / 备注 / 关联 ----------------

    /// 一键处理完成：写处置说明、改状态、指派回提出人、登记工时
    pub async fn resolve(&self, req: &ResolveRequest, via: &str) -> Result<ResolveOutcome> {
        if req.notes.trim().is_empty() {
            return Err(coded("NOTES_EMPTY", "处置说明不能为空"));
        }
        let cfg = self.config();
        let done_ratio = req.done_ratio.or(cfg.workflow.set_done_ratio.then_some(100));
        let status_id = req.status_id.or(cfg.workflow.resolved_status_id);
        let files = self.check_attachments(&req.attachments, via)?;
        let mut out = self
            .apply_update(
                req.issue_id,
                UpdatePlan {
                    notes: Some(req.notes.clone()),
                    status_id,
                    assign_to_author: req.assign_to_author,
                    assign_to: req.assign_to_id,
                    done_ratio,
                    private_notes: req.private_notes,
                    files,
                },
            )
            .await?;
        if status_id.is_none() {
            out.warnings.push("未配置“处理完成状态”，本次未修改状态（可在设置 → 工作流中配置）".into());
        }
        if let Some(h) = req.spent_hours.filter(|h| *h > 0.0) {
            let activity = req.activity_id.or(cfg.workflow.default_activity_id);
            let comment = req.notes.lines().find(|l| !l.trim().is_empty()).unwrap_or("处理任务");
            if let Err(e) = self.redmine()?.create_time_entry(req.issue_id, h, activity, comment).await {
                out.warnings.push(format!("处置已提交，但工时登记失败：{e}"));
            }
        }
        // 只写备注、不改状态也不改指派时，不算一次回派
        if status_id.is_none() && req.assign_to_id.is_none() && !req.assign_to_author {
            self.emit(AppEvent::IssuesChanged);
            return Ok(out);
        }
        let subject = self.db.get_view(req.issue_id)?.map(|v| v.subject).unwrap_or_default();
        self.db.add_resolution(&ResolutionRecord {
            issue_id: req.issue_id,
            subject,
            notes: req.notes.clone(),
            status_name: out.status.clone().unwrap_or_default(),
            assigned_to_name: out.assigned_to.clone().unwrap_or_default(),
            spent_hours: req.spent_hours,
            created_at: db::now_rfc3339(),
            via: via.to_string(),
        })?;
        self.emit(AppEvent::IssuesChanged);
        Ok(out)
    }

    pub async fn batch_resolve(&self, reqs: &[ResolveRequest], via: &str) -> Vec<ResolveOutcome> {
        let mut outs = Vec::new();
        for r in reqs {
            outs.push(match self.resolve(r, via).await {
                Ok(o) => o,
                Err(e) => ResolveOutcome {
                    issue_id: r.issue_id,
                    ok: false,
                    error: Some(e.to_string()),
                    ..Default::default()
                },
            });
        }
        outs
    }

    /// 信息不足：把追问发给提出人，状态改为“反馈”，并指派给提出人
    pub async fn request_more_info(&self, id: u64, question: &str) -> Result<ResolveOutcome> {
        if question.trim().is_empty() {
            return Err(coded("NOTES_EMPTY", "追问内容不能为空"));
        }
        let status = self.config().workflow.feedback_status_id;
        let plan = UpdatePlan {
            notes: Some(question.to_string()),
            status_id: status,
            assign_to_author: true,
            ..Default::default()
        };
        let mut out = self.apply_update(id, plan).await?;
        if status.is_none() {
            out.warnings.push("未配置“反馈”状态，本次未修改状态".into());
        }
        self.emit(AppEvent::IssuesChanged);
        Ok(out)
    }

    /// 开始处理：状态切换为“进行中”
    pub async fn start_progress(&self, id: u64) -> Result<ResolveOutcome> {
        let Some(status) = self.config().workflow.in_progress_status_id else {
            return Err(coded("NO_STATUS", "未配置“进行中”状态（设置 → 工作流）"));
        };
        self.change_status(id, status, None).await
    }

    /// 只修改状态（看板拖动），可附带一条备注
    pub async fn change_status(&self, id: u64, status_id: u64, notes: Option<String>) -> Result<ResolveOutcome> {
        let plan = UpdatePlan {
            notes: notes.filter(|n| !n.trim().is_empty()),
            status_id: Some(status_id),
            ..Default::default()
        };
        let out = self.apply_update(id, plan).await?;
        self.emit(AppEvent::IssuesChanged);
        Ok(out)
    }

    /// 把前端选择或粘贴的文件暂存到数据目录，回派时再上传（每个任务都要单独上传一份）
    pub fn stage_attachment(&self, filename: &str, bytes: &[u8]) -> Result<StagedFile> {
        if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(coded("UPLOAD_TOO_LARGE", format!("附件不能超过 {} MB", MAX_ATTACHMENT_BYTES / 1024 / 1024)));
        }
        let name = sanitize_filename(filename);
        let dir = self.paths.staging_dir().join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(&name);
        std::fs::write(&path, bytes)?;
        Ok(StagedFile {
            path: path.display().to_string(),
            content_type: guess_content_type(&name).map(str::to_string),
            filename: name,
            size: bytes.len() as u64,
        })
    }

    /// 校验附件路径。桌面端只能用暂存目录里的文件；MCP 调用方不能读取隐藏目录和 RedPilot 数据目录。
    fn check_attachments(&self, paths: &[String], via: &str) -> Result<Vec<PathBuf>> {
        let staging = self.paths.staging_dir().canonicalize().ok();
        let data_root = self.paths.root.canonicalize().unwrap_or_else(|_| self.paths.root.clone());
        let mut out = Vec::new();
        for raw in paths {
            let p = Path::new(raw.trim());
            let real = p
                .canonicalize()
                .map_err(|_| coded("ATTACHMENT_INVALID", format!("找不到附件：{raw}")))?;
            let meta = std::fs::metadata(&real)?;
            if !meta.is_file() {
                return Err(coded("ATTACHMENT_INVALID", format!("不是文件：{raw}")));
            }
            if meta.len() > MAX_ATTACHMENT_BYTES {
                return Err(coded("UPLOAD_TOO_LARGE", format!("附件超过 {} MB：{raw}", MAX_ATTACHMENT_BYTES / 1024 / 1024)));
            }
            let allowed = if via == "app" {
                staging.as_ref().is_some_and(|s| real.starts_with(s))
            } else {
                let hidden = real.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
                !hidden && !real.starts_with(&data_root)
            };
            if !allowed {
                return Err(coded("ATTACHMENT_FORBIDDEN", format!("不允许上传该位置的文件：{raw}（隐藏目录和 RedPilot 数据目录中的文件不能作为附件）")));
            }
            out.push(real);
        }
        Ok(out)
    }

    pub async fn add_note(&self, id: u64, notes: &str, private: bool) -> Result<()> {
        if notes.trim().is_empty() {
            return Err(coded("NOTES_EMPTY", "备注内容不能为空"));
        }
        self.redmine()?
            .update_issue(id, &IssueUpdate { notes: Some(notes.to_string()), private_notes: private, ..Default::default() })
            .await?;
        self.emit(AppEvent::IssuesChanged);
        Ok(())
    }

    pub async fn link_issues(&self, id: u64, others: &[u64], relation_type: &str) -> Result<Vec<String>> {
        let allowed = ["relates", "duplicates", "duplicated", "blocks", "blocked", "precedes", "follows", "copied_to", "copied_from"];
        if !allowed.contains(&relation_type) {
            return Err(coded("RELATION_TYPE", format!("不支持的关联类型：{relation_type}")));
        }
        let rm = self.redmine()?;
        let mut errors = Vec::new();
        for &o in others.iter().filter(|o| **o != id) {
            if let Err(e) = rm.create_relation(id, o, relation_type).await {
                errors.push(format!("#{id} → #{o}: {e}"));
            }
        }
        Ok(errors)
    }

    async fn apply_update(&self, id: u64, plan: UpdatePlan) -> Result<ResolveOutcome> {
        let UpdatePlan { notes, status_id, assign_to_author, assign_to, done_ratio, private_notes, files } = plan;
        let rm = self.redmine()?;
        let issue = rm.get_issue(id).await?;
        let me = self.me().await.ok();
        let mut warnings = Vec::new();

        // Redmine 5.0+ 会返回当前用户可切换的状态，提前校验以给出更清楚的提示
        if let Some(s) = status_id {
            if s != issue.status.id && !issue.allowed_statuses.is_empty() && !issue.allowed_statuses.iter().any(|a| a.id == s) {
                let names: Vec<&str> = issue.allowed_statuses.iter().map(|a| a.name.as_str()).collect();
                return Err(coded(
                    "STATUS_INVALID",
                    format!("工作流不允许从“{}”直接切换到目标状态，当前可选：{}", issue.status.name, names.join("、")),
                ));
            }
        }

        let mut assignee: Option<NamedId> = None;
        if let Some(uid) = assign_to {
            // 优先用 Redmine 按其显示格式返回的名字
            let name = if issue.author.as_ref().is_some_and(|a| a.id == uid) {
                issue.author.as_ref().map(|a| a.name.clone())
            } else {
                self.assignees(issue.project.id)
                    .await
                    .ok()
                    .and_then(|l| l.into_iter().find(|a| a.id == uid).map(|a| a.name))
                    .or_else(|| me.as_ref().filter(|m| m.id == uid).map(|m| m.display_name()))
            };
            assignee = Some(NamedId {
                id: uid,
                name: name.unwrap_or_else(|| format!("用户 #{uid}")),
                is_closed: None,
            });
        } else if assign_to_author {
            match &issue.author {
                None => warnings.push("任务缺少提出人信息，未修改指派".into()),
                Some(a) if is_anonymous(a) => warnings.push("提出人为匿名用户，无法指派，请手动处理".into()),
                Some(a) if me.as_ref().is_some_and(|m| m.id == a.id) => warnings.push("提出人就是你自己，指派保持不变".into()),
                Some(a) => assignee = Some(a.clone()),
            }
        }

        // 先上传附件；上传失败时不修改任务（未挂接的上传会被 Redmine 定期清理）
        let mut uploads = Vec::new();
        for f in &files {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "附件".into());
            let bytes = tokio::fs::read(f).await.with_context(|| format!("读取附件失败: {}", f.display()))?;
            uploads.push(rm.upload(bytes, &name, guess_content_type(&name)).await?);
        }
        let attached: Vec<String> = uploads.iter().map(|u| u.filename.clone()).collect();

        rm.update_issue(
            id,
            &IssueUpdate {
                notes,
                private_notes,
                status_id,
                assigned_to_id: assignee.as_ref().map(|a| a.id),
                done_ratio,
                uploads,
            },
        )
        .await?;

        let statuses = self.statuses().await.unwrap_or_default();
        let target = status_id.and_then(|s| statuses.iter().find(|x| x.id == s));
        // 自己做的修改直接写回本地缓存，下次同步不会把它当成别人的变更
        match rm.get_issue(id).await {
            Ok(fresh) => self.db.apply_own_update(&fresh)?,
            Err(e) => tracing::warn!("刷新 #{id} 失败: {e}"),
        }
        let handed_off = assignee.as_ref().is_some_and(|a| me.as_ref().is_none_or(|m| m.id != a.id));
        if handed_off || target.is_some_and(|t| t.is_closed == Some(true)) {
            self.db.set_out_of_inbox(id)?;
        }
        let status = match status_id {
            Some(s) => Some(target.map(|t| t.name.clone()).unwrap_or_else(|| format!("状态 #{s}"))),
            None => Some(issue.status.name.clone()),
        };
        Ok(ResolveOutcome {
            issue_id: id,
            ok: true,
            assigned_to: assignee.map(|a| a.name).or(issue.assigned_to.map(|a| a.name)),
            status,
            attached,
            warnings,
            error: None,
        })
    }

    // ---------------- 项目总览（项目管理员视角） ----------------

    /// 我参与的项目，担任管理类角色的排在前面；系统管理员可以看到全部项目
    pub async fn my_projects(&self) -> Result<Vec<ProjectAccess>> {
        let rm = self.redmine()?;
        let (admin, mut list) = rm.my_projects().await?;
        for p in &mut list {
            p.is_manager = admin || p.roles.iter().any(|r| project::is_manager_role(r));
        }
        if admin {
            for p in rm.projects().await.unwrap_or_default() {
                if !list.iter().any(|x| x.id == p.id) {
                    list.push(ProjectAccess { id: p.id, name: p.name, roles: vec!["系统管理员".into()], is_manager: true });
                }
            }
        }
        list.sort_by(|a, b| b.is_manager.cmp(&a.is_manager).then_with(|| a.name.cmp(&b.name)));
        Ok(list)
    }

    /// 项目全部未关闭任务、版本进度、近 8 周新增/关闭趋势（缓存 5 分钟）
    pub async fn project_overview(&self, project_id: u64, refresh: bool) -> Result<ProjectOverview> {
        if !refresh {
            if let Some((at, ov)) = self.overviews.read().unwrap_or_else(|e| e.into_inner()).get(&project_id) {
                if at.elapsed() < Duration::from_secs(300) {
                    return Ok(ov.clone());
                }
            }
        }
        let rm = self.redmine()?;
        let today = chrono::Local::now().date_naive();
        let pid = project_id.to_string();
        let open = rm
            .list_issues(&IssueQuery { project: Some(pid.clone()), status: "open".into(), ..Default::default() })
            .await?;
        let truncated = open.len() >= 2000;
        let issues: Vec<ProjectIssue> = open.iter().map(|i| project::to_project_issue(i, today)).collect();

        let mut members = self.assignees(project_id).await.unwrap_or_default();
        if members.is_empty() {
            // 没有权限查看成员时，用当前任务的负责人作为候选
            for i in &issues {
                if let Some(id) = i.assignee_id {
                    if !members.iter().any(|m| m.id == id) {
                        members.push(Assignee { id, name: i.assignee.clone(), is_group: false, roles: vec![] });
                    }
                }
            }
        }

        let mut versions: Vec<_> = rm
            .versions(project_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|v| v.status != "closed")
            .collect();
        versions.sort_by(|a, b| match (&a.due_date, &b.due_date) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        versions.truncate(8);

        // 版本已关闭数量、每周新增/关闭数量，都只需要 total_count，并发查询
        enum Slot {
            Version(usize),
            Created(usize),
            Closed(usize),
        }
        let weeks = project::recent_weeks(today, 8);
        let mut jobs: Vec<(Slot, Vec<(&'static str, String)>)> = Vec::new();
        for (vi, v) in versions.iter().enumerate() {
            jobs.push((
                Slot::Version(vi),
                vec![("project_id", pid.clone()), ("fixed_version_id", v.id.to_string()), ("status_id", "closed".into())],
            ));
        }
        for (wi, (a, b)) in weeks.iter().enumerate() {
            jobs.push((Slot::Created(wi), vec![("project_id", pid.clone()), ("status_id", "*".into()), ("created_on", format!("><{a}|{b}"))]));
            jobs.push((Slot::Closed(wi), vec![("project_id", pid.clone()), ("status_id", "closed".into()), ("closed_on", format!("><{a}|{b}"))]));
        }
        let sem = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for (slot, params) in jobs {
            let rm = rm.clone();
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                let _permit = sem.acquire().await;
                let r = rm.count_issues(&params).await;
                (slot, r)
            });
        }
        let mut closed_by_version = vec![0usize; versions.len()];
        let mut weekly: Vec<WeekStat> = weeks
            .iter()
            .map(|(a, _)| WeekStat { label: a.format("%m-%d").to_string(), created: 0, closed: 0 })
            .collect();
        while let Some(joined) = set.join_next().await {
            let Ok((slot, r)) = joined else { continue };
            let n = r.unwrap_or_else(|e| {
                tracing::warn!("统计任务数量失败: {e}");
                0
            });
            match slot {
                Slot::Version(i) => closed_by_version[i] = n,
                Slot::Created(i) => weekly[i].created = n,
                Slot::Closed(i) => weekly[i].closed = n,
            }
        }
        let versions = versions
            .iter()
            .enumerate()
            .map(|(vi, v)| {
                let open = issues.iter().filter(|i| i.version_id == Some(v.id)).count();
                VersionProgress {
                    id: v.id,
                    name: v.name.clone(),
                    status: v.status.clone(),
                    due_date: v.due_date.clone(),
                    open,
                    closed: closed_by_version[vi],
                    overdue: open > 0
                        && v.due_date
                            .as_deref()
                            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                            .is_some_and(|d| d < today),
                }
            })
            .collect();

        let ov = ProjectOverview {
            project_id,
            fetched_at: db::now_rfc3339(),
            issues,
            truncated,
            members,
            versions,
            weekly,
        };
        self.overviews
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project_id, (std::time::Instant::now(), ov.clone()));
        Ok(ov)
    }

    /// 为待转派任务推荐负责人；`use_ai` 时由 AI 综合判断，否则用本地算法
    pub async fn suggest_assignments(&self, project_id: u64, issue_ids: &[u64], use_ai: bool) -> Result<Vec<AssignSuggestion>> {
        if issue_ids.is_empty() {
            return Ok(vec![]);
        }
        let ov = self.project_overview(project_id, false).await?;
        let rm = self.redmine()?;
        let today = chrono::Local::now().date_naive();
        let mut targets: Vec<ProjectIssue> = Vec::new();
        for id in issue_ids {
            match ov.issues.iter().find(|i| i.id == *id) {
                Some(i) => targets.push(i.clone()),
                None => targets.push(project::to_project_issue(&rm.get_issue(*id).await?, today)),
            }
        }
        let (categories, refs, load) = self.assign_refs(&rm, &ov).await;
        let local = project::local_suggestions(&targets, &refs, &ov.members, &categories, &load);
        if !use_ai {
            return Ok(local);
        }
        self.ai_assign(project_id, &targets, &ov, &categories, &load, local).await
    }

    /// 推荐负责人用到的参考数据：类别、参考任务（谁处理过什么）、每个成员当前的未关闭任务数
    async fn assign_refs(&self, rm: &RedmineClient, ov: &ProjectOverview) -> (Vec<IssueCategory>, Vec<ProjectIssue>, HashMap<u64, usize>) {
        let today = chrono::Local::now().date_naive();
        let categories = rm.issue_categories(ov.project_id).await.unwrap_or_default();
        // 近 120 天关闭、且没有回派给提出人的任务，反映谁处理过什么
        let since = (today - chrono::Duration::days(120)).format("%Y-%m-%d").to_string();
        let closed = rm
            .list_issues(&IssueQuery {
                project: Some(ov.project_id.to_string()),
                status: "closed".into(),
                updated_since: Some(since),
                max_results: 500,
                ..Default::default()
            })
            .await
            .unwrap_or_default();
        let mut refs = ov.issues.clone();
        refs.extend(
            closed
                .iter()
                .map(|i| project::to_project_issue(i, today))
                .filter(|i| i.assignee_id.is_some() && i.assignee_id != i.author_id),
        );
        let mut load: HashMap<u64, usize> = HashMap::new();
        for i in &ov.issues {
            if let Some(a) = i.assignee_id {
                *load.entry(a).or_default() += 1;
            }
        }
        (categories, refs, load)
    }

    async fn ai_assign(
        &self,
        project_id: u64,
        targets: &[ProjectIssue],
        ov: &ProjectOverview,
        categories: &[IssueCategory],
        load: &HashMap<u64, usize>,
        local: Vec<AssignSuggestion>,
    ) -> Result<Vec<AssignSuggestion>> {
        let cfg = self.config();
        if cfg.ai.excluded_project_ids.contains(&project_id) {
            return Err(coded("AI_EXCLUDED", "该项目已设置为不发送给 AI，请使用“按历史推荐”"));
        }
        let ai = self.ai_client(true)?;
        let cands = project::candidates(&ov.members);
        if cands.is_empty() {
            return Ok(local);
        }
        let mut body = members_brief(&cands, ov, load);
        let defaults: Vec<String> = categories
            .iter()
            .filter_map(|c| c.assigned_to.as_ref().map(|a| format!("{} → {}", c.name, a.name)))
            .collect();
        if !defaults.is_empty() {
            body.push_str(&format!("\n类别默认负责人：{}\n", defaults.join("；")));
        }
        body.push_str("\n本地算法的推荐（仅供参考）：\n");
        for s in &local {
            body.push_str(&format!("- #{} → {}（{}）\n", s.issue_id, s.assignee_name, s.reason));
        }
        body.push_str("\n待分配任务：\n");
        for t in targets {
            body.push_str(&format!(
                "### #{} [{} / {} / {}] 优先级：{} 截止：{} 当前负责人：{}\n标题：{}\n描述：{}\n\n",
                t.id,
                t.tracker,
                t.category.as_deref().unwrap_or("无类别"),
                t.version.as_deref().unwrap_or("无版本"),
                t.priority,
                t.due_date.as_deref().unwrap_or("-"),
                if t.assignee.is_empty() { "未指派" } else { &t.assignee },
                t.subject,
                truncate(&t.description, 300)
            ));
        }
        let msgs = vec![
            Message::system(prompts::with_extra(prompts::assign_system(), &cfg.ai.custom_prompts.plan)),
            Message::user(self.prep_for_ai(&body)),
        ];
        let (v, usage) = ai.chat_json::<Value>(&msgs).await?;
        self.record_usage("assign", &ai.model, usage);
        let mut by_issue: HashMap<u64, (u64, String)> = HashMap::new();
        for item in v["assignments"].as_array().cloned().unwrap_or_default() {
            let (Some(iid), Some(aid)) = (v_u64(&item["issueId"]), v_u64(&item["assigneeId"])) else { continue };
            if cands.iter().any(|c| c.id == aid) {
                by_issue.insert(iid, (aid, v_str(&item["reason"]).unwrap_or_default()));
            }
        }
        // AI 没给出有效结果的任务沿用本地推荐
        Ok(local
            .into_iter()
            .map(|s| match by_issue.get(&s.issue_id) {
                Some((aid, reason)) => AssignSuggestion {
                    issue_id: s.issue_id,
                    assignee_id: Some(*aid),
                    assignee_name: cands.iter().find(|c| c.id == *aid).map(|c| c.name.clone()).unwrap_or_default(),
                    reason: if reason.is_empty() { "AI 推荐".into() } else { reason.clone() },
                    source: "ai".into(),
                },
                None => s,
            })
            .collect())
    }

    /// 批量转派：每个任务指派给各自的目标成员，可附带同一条说明（并发 4 个）
    pub async fn bulk_assign(self: &Arc<Self>, items: &[AssignItem], notes: Option<String>) -> Vec<ResolveOutcome> {
        let notes = notes.filter(|n| !n.trim().is_empty());
        let sem = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for (idx, it) in items.iter().cloned().enumerate() {
            let svc = Arc::clone(self);
            let sem = Arc::clone(&sem);
            let notes = notes.clone();
            set.spawn(async move {
                let _permit = sem.acquire().await;
                let plan = UpdatePlan { notes, assign_to: Some(it.assignee_id), ..Default::default() };
                let out = match svc.apply_update(it.issue_id, plan).await {
                    Ok(o) => o,
                    Err(e) => ResolveOutcome { issue_id: it.issue_id, ok: false, error: Some(e.to_string()), ..Default::default() },
                };
                (idx, out)
            });
        }
        let mut outs: Vec<(usize, ResolveOutcome)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            if let Ok(r) = joined {
                outs.push(r);
            }
        }
        outs.sort_by_key(|(i, _)| *i);
        write(&self.overviews, HashMap::new());
        self.emit(AppEvent::IssuesChanged);
        outs.into_iter().map(|(_, o)| o).collect()
    }

    // ---------------- 拆分任务 ----------------

    /// 拆分建议：默认把描述里分条列出的事项按推荐负责人分组；`use_ai` 时由 AI 按分工拆分
    pub async fn suggest_split(&self, issue_id: u64, use_ai: bool) -> Result<SplitSuggestion> {
        let rm = self.redmine()?;
        let parent = rm.get_issue(issue_id).await?;
        let pi = project::to_project_issue(&parent, chrono::Local::now().date_naive());
        let ov = match self.project_overview(parent.project.id, false).await {
            Ok(ov) => ov,
            Err(e) => {
                // 读不了全项目任务时，仍然给出成员列表，可以手动拆分
                tracing::warn!("读取项目 {} 的任务失败: {e}", parent.project.id);
                ProjectOverview {
                    project_id: parent.project.id,
                    fetched_at: db::now_rfc3339(),
                    issues: vec![],
                    truncated: false,
                    members: self.assignees(parent.project.id).await.unwrap_or_default(),
                    versions: vec![],
                    weekly: vec![],
                }
            }
        };
        let (categories, refs, load) = self.assign_refs(&rm, &ov).await;
        let items = project::split_items(parent.description.as_deref().unwrap_or(""));
        let local = if items.len() >= 2 {
            project::local_split(&pi, &items, &refs, &ov.members, &categories, &load)
        } else {
            vec![]
        };
        let mut hint = None;
        let parts = if use_ai {
            let parts = self.ai_split(&parent, &ov, &categories, &refs, &load, &local).await?;
            if parts.len() == 1 {
                hint = Some("AI 认为这个问题由一个人处理即可（理由见下方），如仍需拆分可以手动添加".into());
            }
            parts
        } else {
            if items.len() < 2 {
                hint = Some("描述里没有分条列出的事项，可以点“AI 拆分”，或手动添加子任务".into());
            } else if local.len() == 1 {
                hint = Some("这些条目都推荐给了同一个人；如需分工，可以点“AI 拆分”或手动调整".into());
            }
            local
        };
        let mut load_list: Vec<(u64, usize)> = load.into_iter().collect();
        load_list.sort();
        Ok(SplitSuggestion {
            issue_id,
            subject: parent.subject.clone(),
            description: parent.description.clone().unwrap_or_default(),
            category: parent.category.as_ref().map(|c| c.name.clone()),
            parts,
            hint,
            members: ov.members.clone(),
            categories: categories.iter().map(|c| NamedId { id: c.id, name: c.name.clone(), is_closed: None }).collect(),
            load: load_list,
        })
    }

    async fn ai_split(
        &self,
        parent: &Issue,
        ov: &ProjectOverview,
        categories: &[IssueCategory],
        refs: &[ProjectIssue],
        load: &HashMap<u64, usize>,
        local: &[SplitPart],
    ) -> Result<Vec<SplitPart>> {
        let cfg = self.config();
        if cfg.ai.excluded_project_ids.contains(&parent.project.id) {
            return Err(coded("AI_EXCLUDED", "该项目已设置为不发送给 AI，请按条目拆分或手动添加"));
        }
        let ai = self.ai_client(true)?;
        let cands = project::candidates(&ov.members);
        if cands.is_empty() {
            return Err(coded("NO_MEMBERS", "无法读取项目成员，请手动拆分"));
        }
        let mut body = members_brief(&cands, ov, load);
        if !categories.is_empty() {
            let list: Vec<String> = categories
                .iter()
                .map(|c| match &c.assigned_to {
                    Some(a) => format!("[{}] {}（默认负责人 {}）", c.id, c.name, a.name),
                    None => format!("[{}] {}", c.id, c.name),
                })
                .collect();
            body.push_str(&format!("\n可选类别（categoryId）：{}\n", list.join("；")));
        }
        if !local.is_empty() {
            body.push_str("\n本地算法按条目给出的分工（仅供参考）：\n");
            for p in local {
                body.push_str(&format!("- {} → {}（{}）\n", p.subject, p.assignee_name, p.reason));
            }
        }
        body.push_str(&format!(
            "\n待拆分问题：#{} [{} / {} / {}] 优先级：{} 截止：{}\n标题：{}\n描述：\n{}\n",
            parent.id,
            parent.tracker.as_ref().map(|t| t.name.as_str()).unwrap_or("-"),
            parent.category.as_ref().map(|c| c.name.as_str()).unwrap_or("无类别"),
            parent.fixed_version.as_ref().map(|v| v.name.as_str()).unwrap_or("无版本"),
            parent.priority.as_ref().map(|p| p.name.as_str()).unwrap_or("-"),
            parent.due_date.as_deref().unwrap_or("-"),
            parent.subject,
            truncate(parent.description.as_deref().unwrap_or("（无描述）"), 3000),
        ));
        let notes: Vec<String> = parent
            .journals
            .iter()
            .filter_map(|j| {
                let n = j.notes.as_deref()?.trim();
                (!n.is_empty() && j.private_notes != Some(true))
                    .then(|| format!("- {}：{}", j.user.as_ref().map(|u| u.name.as_str()).unwrap_or("?"), truncate(n, 300)))
            })
            .collect();
        if !notes.is_empty() {
            let start = notes.len().saturating_sub(5);
            body.push_str(&format!("\n后续沟通（最近 5 条）：\n{}\n", notes[start..].join("\n")));
        }
        let msgs = vec![
            Message::system(prompts::with_extra(prompts::split_system(), &cfg.ai.custom_prompts.plan)),
            Message::user(self.prep_for_ai(&body)),
        ];
        let (v, usage) = ai.chat_json::<Value>(&msgs).await?;
        self.record_usage("split", &ai.model, usage);

        let mut parts = Vec::new();
        for item in v["parts"].as_array().cloned().unwrap_or_default().into_iter().take(10) {
            let Some(subject) = v_str(&item["subject"]) else { continue };
            let description = v_str(&item["description"]).unwrap_or_default();
            let category_id = v_u64(&item["categoryId"]).filter(|id| categories.iter().any(|c| c.id == *id));
            let estimated_hours = v_f64(&item["estimatedHours"]).filter(|h| *h > 0.0 && *h < 1000.0);
            let reason = v_str(&item["reason"]).unwrap_or_else(|| "AI 推荐".into());
            let mut part = match v_u64(&item["assigneeId"]).and_then(|id| cands.iter().find(|c| c.id == id)) {
                Some(c) => SplitPart { assignee_id: Some(c.id), assignee_name: c.name.clone(), reason, source: "ai".into(), ..Default::default() },
                None => {
                    // AI 给的人不在成员列表里：这一部分改用本地算法推荐
                    let t = ProjectIssue { subject: subject.clone(), description: description.clone(), ..Default::default() };
                    let s = project::local_suggestions(&[t], refs, &ov.members, categories, load).remove(0);
                    SplitPart {
                        assignee_id: s.assignee_id,
                        assignee_name: s.assignee_name,
                        reason: format!("AI 没有给出有效人选，按历史推荐：{}", s.reason),
                        source: s.source,
                        ..Default::default()
                    }
                }
            };
            part.subject = truncate(&subject, 120);
            part.description = description;
            part.category_id = category_id;
            part.estimated_hours = estimated_hours;
            parts.push(part);
        }
        if parts.is_empty() {
            return Err(coded("AI_EMPTY", "AI 没有给出有效的拆分方案，请重试或手动拆分"));
        }
        Ok(parts)
    }

    /// 创建子任务并分别指派。逐个创建，单个失败不影响其他；可选在原任务添加拆分说明。
    pub async fn split_issue(&self, issue_id: u64, parts: &[SplitPart], add_note: bool) -> Result<SplitOutcome> {
        let parts: Vec<&SplitPart> = parts.iter().filter(|p| !p.subject.trim().is_empty()).collect();
        if parts.is_empty() {
            return Err(coded("SPLIT_EMPTY", "请至少填写一个子任务标题"));
        }
        if parts.len() > 20 {
            return Err(coded("SPLIT_TOO_MANY", "一次最多拆分 20 个子任务"));
        }
        let rm = self.redmine()?;
        let parent = rm.get_issue(issue_id).await?;
        let statuses = self.statuses().await.unwrap_or_default();
        let members = self.assignees(parent.project.id).await.unwrap_or_default();
        let custom: Vec<(u64, Value)> = parent
            .custom_fields
            .iter()
            .filter(|c| !(c.value.is_null() || c.value.as_str() == Some("") || c.value.as_array().is_some_and(|a| a.is_empty())))
            .map(|c| (c.id, c.value.clone()))
            .collect();
        let mut out = SplitOutcome { issue_id, ..Default::default() };
        // 同一批子任务的跟踪类型相同：父任务字段或自定义字段被拒绝一次后，后面的不再带上
        let mut as_subtask = true;
        let mut with_custom = true;
        for p in parts {
            let mut n = NewIssue {
                project_id: parent.project.id,
                tracker_id: parent.tracker.as_ref().map(|t| t.id),
                priority_id: parent.priority.as_ref().map(|t| t.id),
                parent_issue_id: as_subtask.then_some(issue_id),
                subject: p.subject.trim().to_string(),
                description: p.description.trim().to_string(),
                assigned_to_id: p.assignee_id,
                category_id: p.category_id.or(parent.category.as_ref().map(|c| c.id)),
                fixed_version_id: parent.fixed_version.as_ref().map(|v| v.id),
                due_date: parent.due_date.clone(),
                estimated_hours: p.estimated_hours,
                custom_fields: if with_custom { custom.clone() } else { vec![] },
            };
            let mut res = rm.create_issue(&n).await;
            // 有的跟踪类型禁用了“父任务”字段，或沿用的自定义字段值对新任务无效：去掉后重试一次
            if let Err(e) = &res {
                let msg = plain_error(e);
                let parent_bad = n.parent_issue_id.is_some() && (msg.contains("父任务") || msg.to_lowercase().contains("parent"));
                let cf_bad = !n.custom_fields.is_empty() && parent.custom_fields.iter().any(|c| !c.name.is_empty() && msg.contains(&c.name));
                if parent_bad {
                    n.parent_issue_id = None;
                    as_subtask = false;
                    out.warnings.push(format!("无法设为子任务（{msg}），已改为与 #{issue_id} 建立“相关”关联"));
                }
                if cf_bad {
                    n.custom_fields.clear();
                    with_custom = false;
                    out.warnings.push("沿用的自定义字段值对子任务无效，子任务未设置自定义字段".into());
                }
                if parent_bad || cf_bad {
                    res = rm.create_issue(&n).await;
                }
            }
            match res {
                Ok(child) => {
                    if n.parent_issue_id.is_none() {
                        if let Err(e) = rm.create_relation(issue_id, child.id, "relates").await {
                            out.warnings.push(format!("#{} 关联到 #{issue_id} 失败：{}", child.id, plain_error(&e)));
                        }
                    }
                    let mut c = child_issue(&child, &statuses);
                    if c.assignee.is_empty() {
                        if let Some(m) = p.assignee_id.and_then(|id| members.iter().find(|m| m.id == id)) {
                            c.assignee = m.name.clone();
                        }
                    }
                    out.created.push(c);
                }
                Err(e) => out.failed.push(format!("{}：{}", truncate(p.subject.trim(), 40), plain_error(&e))),
            }
        }

        if add_note && !out.created.is_empty() {
            let bullet = match self.config().redmine.text_format {
                TextFormat::Textile => "*",
                TextFormat::Markdown => "-",
            };
            let mut note = format!("已拆分为 {} 个子任务：\n\n", out.created.len());
            for c in &out.created {
                let who = if c.assignee.is_empty() { "未指派" } else { c.assignee.as_str() };
                note.push_str(&format!("{bullet} #{} {}（{who}）\n", c.id, c.subject));
            }
            match rm.update_issue(issue_id, &IssueUpdate { notes: Some(note), ..Default::default() }).await {
                Ok(()) => {
                    if let Ok(fresh) = rm.get_issue(issue_id).await {
                        self.db.apply_own_update(&fresh)?;
                    }
                }
                Err(e) => out.warnings.push(format!("在原任务添加拆分说明失败：{}", plain_error(&e))),
            }
        }
        write(&self.overviews, HashMap::new());
        self.emit(AppEvent::IssuesChanged);
        Ok(out)
    }

    // ---------------- 统计与报告 ----------------

    pub fn stats(&self) -> Result<Stats> {
        let views = self.db.list_views(&IssueFilter::default())?;
        let count_by = |key: &dyn Fn(&IssueView) -> String| -> Vec<(String, usize)> {
            let mut m: HashMap<String, usize> = HashMap::new();
            for v in &views {
                *m.entry(key(v)).or_default() += 1;
            }
            let mut out: Vec<(String, usize)> = m.into_iter().collect();
            out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            out
        };
        let now = chrono::Utc::now();
        let r7 = self.db.resolutions_since(&(now - chrono::Duration::days(7)).to_rfc3339())?;
        let r30 = self.db.resolutions_since(&(now - chrono::Duration::days(30)).to_rfc3339())?;
        let (prompt_tokens, completion_tokens) = self.db.usage_since(&(now - chrono::Duration::days(30)).to_rfc3339())?;
        let ai = self.config().ai;
        Ok(Stats {
            open_total: views.len(),
            unread: views.iter().filter(|v| v.unread).count(),
            overdue: views.iter().filter(|v| v.overdue).count(),
            high_priority: views.iter().filter(|v| is_high_priority(&v.priority_name)).count(),
            unanalyzed: views.iter().filter(|v| !v.analysis.ai_analyzed).count(),
            by_category: count_by(&|v| v.analysis.category.clone().unwrap_or_else(|| "未分类".into())),
            by_project: count_by(&|v| v.project_name.clone()),
            by_priority: count_by(&|v| v.priority_name.clone()),
            resolved_7d: r7.len(),
            resolved_30d: r30.len(),
            spent_hours_7d: r7.iter().filter_map(|r| r.spent_hours).sum(),
            daily_resolved: self.db.daily_resolved(14)?,
            ai_tokens_30d: prompt_tokens + completion_tokens,
            ai_prompt_tokens_30d: prompt_tokens,
            ai_completion_tokens_30d: completion_tokens,
            ai_cost_30d: (prompt_tokens as f64 * ai.price_input + completion_tokens as f64 * ai.price_output) / 1_000_000.0,
            ai_currency: ai.currency,
            last_sync_at: self.db.kv_get("last_sync_at")?,
        })
    }

    pub fn recent_resolutions(&self, days: i64) -> Result<Vec<ResolutionRecord>> {
        self.db
            .resolutions_since(&(chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339())
    }

    /// AI 生成日报 / 周报
    pub async fn work_report(&self, days: i64) -> Result<String> {
        let days = days.clamp(1, 31);
        let records = self.recent_resolutions(days)?;
        if records.is_empty() {
            return Ok(format!("最近 {days} 天没有通过 RedPilot 回派的记录。"));
        }
        let ai = self.ai_client(false)?;
        let mut body = String::new();
        for r in &records {
            body.push_str(&format!(
                "- {} #{} {}（工时 {}h）\n  处置：{}\n",
                &r.created_at[..10.min(r.created_at.len())],
                r.issue_id,
                r.subject,
                r.spent_hours.unwrap_or(0.0),
                truncate(&r.notes.replace('\n', " "), 300)
            ));
        }
        let extra = self.config().ai.custom_prompts.report;
        let msgs = vec![
            Message::system(prompts::with_extra(prompts::report_system(days), &extra)),
            Message::user(self.prep_for_ai(&body)),
        ];
        let (text, usage) = ai.chat(&msgs, false).await?;
        self.record_usage("report", &ai.model, usage);
        Ok(strip_fence(&text))
    }
}

// ---------------- 工具函数 ----------------

/// 给 AI 的成员说明：角色、当前负载、正在负责的任务（体现熟悉的模块）
fn members_brief(cands: &[&Assignee], ov: &ProjectOverview, load: &HashMap<u64, usize>) -> String {
    let mut body = String::from("项目成员：\n");
    for c in cands {
        let own: Vec<String> = ov
            .issues
            .iter()
            .filter(|i| i.assignee_id == Some(c.id))
            .take(6)
            .map(|i| format!("#{} {}", i.id, truncate(&i.subject, 30)))
            .collect();
        body.push_str(&format!(
            "- [{}] {}（{}）当前未关闭 {} 个；正在负责：{}\n",
            c.id,
            c.name,
            if c.roles.is_empty() { "角色未知".into() } else { c.roles.join("、") },
            load.get(&c.id).copied().unwrap_or(0),
            if own.is_empty() { "无".into() } else { own.join("；") }
        ));
    }
    body
}

/// 给用户看的错误信息，不带错误码
fn plain_error(e: &anyhow::Error) -> String {
    match e.downcast_ref::<crate::CodedError>() {
        Some(c) => c.message.clone(),
        None => e.to_string(),
    }
}

fn child_issue(is: &Issue, statuses: &[NamedId]) -> ChildIssue {
    let closed = is
        .status
        .is_closed
        .or_else(|| statuses.iter().find(|s| s.id == is.status.id).and_then(|s| s.is_closed))
        .unwrap_or(false);
    ChildIssue {
        id: is.id,
        subject: is.subject.clone(),
        tracker: is.tracker.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
        status: is.status.name.clone(),
        closed,
        assignee: is.assigned_to.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        done_ratio: is.done_ratio.unwrap_or(0),
    }
}

#[derive(Default)]
struct UpdatePlan {
    notes: Option<String>,
    status_id: Option<u64>,
    assign_to_author: bool,
    /// 指派给指定成员，优先于 assign_to_author
    assign_to: Option<u64>,
    done_ratio: Option<u32>,
    private_notes: bool,
    files: Vec<PathBuf>,
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 根据上次同步之后新增的沟通记录，找出别人做的变更（忽略自己的操作）
fn changes_from_journals(prev: &PrevIssue, is: &Issue, me: Option<u64>) -> Vec<IssueChange> {
    let Some(since) = parse_time(&prev.updated_on) else { return vec![] };
    let author_id = is.author.as_ref().map(|a| a.id);
    let me_s = me.map(|m| m.to_string());
    let mut out = Vec::new();
    for j in &is.journals {
        if !parse_time(&j.created_on).is_some_and(|t| t > since) {
            continue;
        }
        let Some(user) = &j.user else { continue };
        if Some(user.id) == me {
            continue;
        }
        let by_author = Some(user.id) == author_id;
        let actor = user.name.clone();
        let change = |kind: &str, summary: String, important: bool| IssueChange {
            issue_id: is.id,
            subject: is.subject.clone(),
            kind: kind.into(),
            actor: actor.clone(),
            by_author,
            summary,
            important,
            created_on: j.created_on.clone(),
        };
        if let Some(n) = j.notes.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            let who = if by_author { format!("提出人 {actor}") } else { actor.clone() };
            out.push(change("reply", format!("{who} 回复：{}", truncate(&one_line(n), 60)), by_author));
        }
        for d in j.details.iter().filter(|d| d.property == "attr") {
            let new = d.new_value.as_deref();
            match d.name.as_str() {
                "assigned_to_id" if me_s.is_some() && new == me_s.as_deref() => {
                    out.push(change("assigned", format!("{actor} 把任务转给了你"), true));
                }
                "priority_id" => {
                    let name = is.priority.as_ref().filter(|p| new == Some(p.id.to_string().as_str())).map(|p| p.name.clone());
                    let high = name.as_deref().is_some_and(is_high_priority);
                    out.push(change("priority", format!("{actor} 把优先级调整为 {}", name.unwrap_or_else(|| "新等级".into())), high));
                }
                "due_date" => {
                    out.push(change("due", format!("{actor} 把截止日期改为 {}", new.unwrap_or("无")), false));
                }
                "status_id" => {
                    let name = if new == Some(is.status.id.to_string().as_str()) { is.status.name.clone() } else { "新状态".into() };
                    out.push(change("status", format!("{actor} 把状态改为 {name}"), false));
                }
                _ => {}
            }
        }
    }
    out
}

/// 拿不到沟通记录时，只根据字段对比给出最关键的变化
fn changes_from_fields(prev: &PrevIssue, is: &Issue) -> Vec<IssueChange> {
    let base = |kind: &str, summary: String, important: bool| IssueChange {
        issue_id: is.id,
        subject: is.subject.clone(),
        kind: kind.into(),
        summary,
        important,
        created_on: is.updated_on.clone(),
        ..Default::default()
    };
    let mut out = Vec::new();
    if !prev.in_inbox {
        out.push(base("assigned", "任务重新回到了你的待办".into(), true));
    }
    if let Some(p) = is.priority.as_ref().filter(|p| p.id != prev.priority_id) {
        out.push(base("priority", format!("优先级调整为 {}", p.name), is_high_priority(&p.name)));
    }
    if is.due_date != prev.due_date {
        out.push(base("due", format!("截止日期改为 {}", is.due_date.as_deref().unwrap_or("无")), false));
    }
    out
}

/// 选出最值得在列表里提示的一条：提出人回复 > 转回给我 > 调为高优先级 > 其他回复 > 最新一条
fn pick_hint(changes: &[IssueChange]) -> Option<&IssueChange> {
    let rank = |c: &IssueChange| match c.kind.as_str() {
        "reply" if c.by_author => 5,
        "assigned" => 4,
        "priority" if c.important => 3,
        "reply" => 2,
        _ => 1,
    };
    changes.iter().enumerate().max_by_key(|(i, c)| (rank(c), *i)).map(|(_, c)| c)
}

fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || r#"<>:"|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim().to_string();
    if cleaned.is_empty() {
        return "附件".into();
    }
    if cleaned.chars().count() > 120 {
        let (stem, ext) = match cleaned.rsplit_once('.') {
            Some((s, e)) if e.len() <= 10 => (s.to_string(), format!(".{e}")),
            _ => (cleaned.clone(), String::new()),
        };
        return format!("{}{ext}", stem.chars().take(110).collect::<String>());
    }
    cleaned
}

fn guess_content_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp4" => "video/mp4",
        _ => return None,
    })
}

/// 删除目录下修改时间早于 max_age 的文件或子目录（暂存附件、附件缓存）
fn cleanup_old_entries(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - max_age;
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
        if old {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

pub fn is_high_priority(name: &str) -> bool {
    let n = name.to_lowercase();
    db::HIGH_PRIORITY_WORDS.iter().any(|w| n.contains(w))
}

fn is_anonymous(a: &NamedId) -> bool {
    let n = a.name.to_lowercase();
    n == "anonymous" || n.contains("匿名")
}

fn issue_context(is: &Issue, desc_limit: usize, notes_limit: usize) -> String {
    let name = |n: &Option<NamedId>| n.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| "-".into());
    let mut s = format!(
        "### 任务 #{}\n项目：{} ｜ 跟踪：{} ｜ 优先级：{} ｜ 状态：{} ｜ 提出人：{}",
        is.id,
        is.project.name,
        name(&is.tracker),
        name(&is.priority),
        is.status.name,
        name(&is.author)
    );
    if let Some(d) = &is.due_date {
        s.push_str(&format!(" ｜ 截止：{d}"));
    }
    s.push_str(&format!(
        "\n标题：{}\n描述：{}\n",
        is.subject,
        truncate(is.description.as_deref().unwrap_or("（无）").trim(), desc_limit)
    ));
    let mut notes: Vec<_> = is
        .journals
        .iter()
        .filter(|j| j.private_notes != Some(true) && j.notes.as_deref().is_some_and(|n| !n.trim().is_empty()))
        .rev()
        .take(notes_limit)
        .collect();
    notes.reverse();
    if !notes.is_empty() {
        s.push_str("最近沟通：\n");
        for j in notes {
            s.push_str(&format!(
                "- {}（{}）：{}\n",
                name(&j.user),
                &j.created_on[..10.min(j.created_on.len())],
                truncate(j.notes.as_deref().unwrap_or("").trim(), 300)
            ));
        }
    }
    s
}

fn view_from_issue(is: &Issue) -> IssueView {
    IssueView {
        id: is.id,
        project_id: is.project.id,
        project_name: is.project.name.clone(),
        tracker_name: is.tracker.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
        status_id: is.status.id,
        status_name: is.status.name.clone(),
        priority_id: is.priority.as_ref().map(|p| p.id).unwrap_or(0),
        priority_name: is.priority.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
        author_id: is.author.as_ref().map(|a| a.id),
        author_name: is.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        assigned_to_id: is.assigned_to.as_ref().map(|a| a.id),
        assigned_to_name: is.assigned_to.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
        subject: is.subject.clone(),
        description: is.description.clone().unwrap_or_default(),
        done_ratio: is.done_ratio.unwrap_or(0),
        due_date: is.due_date.clone(),
        created_on: is.created_on.clone(),
        updated_on: is.updated_on.clone(),
        ..Default::default()
    }
}

fn parse_analysis(item: &Value, categories: &[String]) -> Analysis {
    let category = v_str(&item["category"]).map(|c| {
        categories
            .iter()
            .find(|k| **k == c || c.contains(k.as_str()))
            .cloned()
            .unwrap_or_else(|| "其他".into())
    });
    let urgency = v_str(&item["urgency"]).map(|u| match u.as_str() {
        s if s.contains('高') || s.eq_ignore_ascii_case("high") => "高".to_string(),
        s if s.contains('低') || s.eq_ignore_ascii_case("low") => "低".to_string(),
        _ => "中".to_string(),
    });
    Analysis {
        category,
        module: v_str(&item["module"]),
        urgency,
        difficulty: v_f64(&item["difficulty"]).map(|d| d.round().clamp(1.0, 5.0) as u8),
        est_hours: v_f64(&item["estHours"]).map(|h| (h * 10.0).round() / 10.0),
        summary: v_str(&item["summary"]),
        missing_info: item["missingInfo"]
            .as_array()
            .map(|a| a.iter().filter_map(v_str).collect())
            .unwrap_or_default(),
        follow_up: v_str(&item["followUp"]),
        suggestion: v_str(&item["suggestion"]),
        ..Default::default()
    }
}

fn v_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok()))
}

fn v_str(v: &Value) -> Option<String> {
    v.as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn v_f64(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| {
        v.as_str()
            .and_then(|s| s.trim().trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok())
    })
}

fn v_ids(v: &Value) -> Vec<u64> {
    v.as_array().map(|a| a.iter().filter_map(v_u64).collect()).unwrap_or_default()
}

fn strip_fence(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|(_, r)| r).unwrap_or(rest);
        return rest.trim_end().trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lenient_ai_output() {
        let cats = vec!["缺陷".to_string(), "需求".to_string(), "其他".to_string()];
        let item = serde_json::json!({
            "id": "#12", "category": "缺陷（Bug）", "urgency": "High",
            "difficulty": "3", "estHours": "2.5h", "missingInfo": ["复现步骤"]
        });
        let a = parse_analysis(&item, &cats);
        assert_eq!(v_u64(&item["id"]), Some(12));
        assert_eq!(a.category.as_deref(), Some("缺陷"));
        assert_eq!(a.urgency.as_deref(), Some("高"));
        assert_eq!(a.difficulty, Some(3));
        assert_eq!(a.est_hours, Some(2.5));
        assert_eq!(a.missing_info, vec!["复现步骤".to_string()]);
    }

    #[test]
    fn strips_code_fence() {
        assert_eq!(strip_fence("```markdown\n**已处理**\n```"), "**已处理**");
    }

    #[test]
    fn sanitizes_filenames() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\a\\截图:1.png"), "截图_1.png");
        assert_eq!(sanitize_filename(".env"), "env");
        assert_eq!(sanitize_filename("  "), "附件");
    }

    #[test]
    fn staged_attachments_are_confined() {
        let root = std::env::temp_dir().join(format!("redpilot-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).unwrap();
        let svc = Service::open(Paths { root: root.clone() }).unwrap();
        let f = svc.stage_attachment("../../截图 1.png", b"png").unwrap();
        assert_eq!(f.filename, "截图 1.png");
        assert_eq!(f.content_type.as_deref(), Some("image/png"));
        assert!(Path::new(&f.path).starts_with(root.join("staging")));
        assert_eq!(svc.check_attachments(&[f.path.clone()], "app").unwrap().len(), 1);
        // 桌面端只能提交暂存目录里的文件
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "x").unwrap();
        let err = svc.check_attachments(&[outside.display().to_string()], "app").unwrap_err();
        assert!(err.to_string().contains("ATTACHMENT_FORBIDDEN"));
        // MCP 不能提交数据目录里的文件（包括暂存目录）
        let err = svc.check_attachments(&[f.path], "mcp").unwrap_err();
        assert!(err.to_string().contains("ATTACHMENT_FORBIDDEN"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_changes_from_journals() {
        let issue: Issue = serde_json::from_value(serde_json::json!({
            "id": 7, "project": {"id": 1, "name": "P"}, "status": {"id": 2, "name": "进行中"},
            "priority": {"id": 4, "name": "紧急"}, "author": {"id": 11, "name": "张三"}, "subject": "S",
            "created_on": "2026-11-01T00:00:00Z", "updated_on": "2026-11-02T03:00:00Z",
            "journals": [
                {"id": 1, "user": {"id": 11, "name": "张三"}, "notes": "旧回复", "created_on": "2026-11-01T01:00:00Z", "details": []},
                {"id": 2, "user": {"id": 5, "name": "我"}, "notes": "我的备注", "created_on": "2026-11-02T01:00:00Z", "details": []},
                {"id": 3, "user": {"id": 11, "name": "张三"}, "notes": "补充了截图\n请查看", "created_on": "2026-11-02T02:00:00Z",
                 "details": [{"property": "attr", "name": "assigned_to_id", "old_value": "11", "new_value": "5"}]},
                {"id": 4, "user": {"id": 1, "name": "管理员"}, "notes": "", "created_on": "2026-11-02T03:00:00Z",
                 "details": [{"property": "attr", "name": "priority_id", "old_value": "2", "new_value": "4"}]}
            ]
        }))
        .unwrap();
        let prev = PrevIssue {
            id: 7,
            updated_on: "2026-11-01T12:00:00Z".into(),
            in_inbox: false,
            status_id: 2,
            priority_id: 2,
            due_date: None,
        };
        let cs = changes_from_journals(&prev, &issue, Some(5));
        let kinds: Vec<&str> = cs.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, vec!["reply", "assigned", "priority"]);
        assert_eq!(cs[0].summary, "提出人 张三 回复：补充了截图 请查看");
        assert!(cs.iter().all(|c| c.important));
        assert_eq!(pick_hint(&cs).unwrap().kind, "reply");
    }
}
