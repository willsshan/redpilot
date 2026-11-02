//! 业务服务层：桌面端命令与 MCP 工具都调用这里，保证两边行为一致。

use crate::ai::{AiClient, Message, Usage};
use crate::analysis::{self, Corpus};
use crate::config::{self, AppConfig, Paths};
use crate::db::{self, Db};
use crate::models::*;
use crate::redmine::{truncate, IssueQuery, IssueUpdate, RedmineClient};
use crate::secrets::{SecretKind, SecretStore};
use crate::{coded, mask, prompts};
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Semaphore};

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
        let (new_ids, updated_ids, removed) = self.db.apply_sync(&issues)?;

        let changed: HashSet<u64> = new_ids.iter().chain(updated_ids.iter()).copied().collect();
        for is in issues.iter().filter(|i| changed.contains(&i.id)) {
            let text = format!("{} {}", is.subject, is.description.as_deref().unwrap_or(""));
            self.db
                .set_rule_category(is.id, analysis::rule_classify(&text, &cfg.rules).as_deref())?;
        }
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
            removed,
            finished_at,
        })
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
        Ok(IssueDetail { view, issue, similar, web_url })
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

        let msgs = vec![Message::system(prompts::plan_system()), Message::user(self.prep_for_ai(&body))];
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
            Message::system(prompts::note_system(cfg.redmine.text_format, &cfg.workflow.note_sections)),
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
        let mut out = self
            .apply_update(req.issue_id, &req.notes, status_id, req.assign_to_author, done_ratio, req.private_notes)
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
        let mut out = self.apply_update(id, question, status, true, None, false).await?;
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
        self.redmine()?
            .update_issue(id, &IssueUpdate { status_id: Some(status), ..Default::default() })
            .await?;
        let _ = self.detail(id, true).await;
        self.emit(AppEvent::IssuesChanged);
        Ok(ResolveOutcome {
            issue_id: id,
            ok: true,
            status: self.status_name(status).await,
            ..Default::default()
        })
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

    async fn status_name(&self, id: u64) -> Option<String> {
        self.statuses().await.ok()?.into_iter().find(|s| s.id == id).map(|s| s.name)
    }

    async fn apply_update(
        &self,
        id: u64,
        notes: &str,
        status_id: Option<u64>,
        assign_to_author: bool,
        done_ratio: Option<u32>,
        private_notes: bool,
    ) -> Result<ResolveOutcome> {
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
        if assign_to_author {
            match &issue.author {
                None => warnings.push("任务缺少提出人信息，未修改指派".into()),
                Some(a) if is_anonymous(a) => warnings.push("提出人为匿名用户，无法指派，请手动处理".into()),
                Some(a) if me.as_ref().is_some_and(|m| m.id == a.id) => warnings.push("提出人就是你自己，指派保持不变".into()),
                Some(a) => assignee = Some(a.clone()),
            }
        }

        rm.update_issue(
            id,
            &IssueUpdate {
                notes: Some(notes.to_string()),
                private_notes,
                status_id,
                assigned_to_id: assignee.as_ref().map(|a| a.id),
                done_ratio,
            },
        )
        .await?;

        if assignee.is_some() {
            self.db.set_out_of_inbox(id)?;
        }
        let _ = self.detail(id, true).await;
        let status = match status_id {
            Some(s) => self.status_name(s).await.or(Some(format!("状态 #{s}"))),
            None => Some(issue.status.name.clone()),
        };
        Ok(ResolveOutcome {
            issue_id: id,
            ok: true,
            assigned_to: assignee.map(|a| a.name).or(issue.assigned_to.map(|a| a.name)),
            status,
            warnings,
            error: None,
        })
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
            ai_tokens_30d: self.db.tokens_since(&(now - chrono::Duration::days(30)).to_rfc3339())?,
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
        let msgs = vec![Message::system(prompts::report_system(days)), Message::user(self.prep_for_ai(&body))];
        let (text, usage) = ai.chat(&msgs, false).await?;
        self.record_usage("report", &ai.model, usage);
        Ok(strip_fence(&text))
    }
}

// ---------------- 工具函数 ----------------

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
}
