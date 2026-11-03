//! 前端调用的 Tauri 命令，均为对 `redpilot_core::Service` 的薄封装。

use redpilot_core::mcp::{self, HttpHandle};
use redpilot_core::models::*;
use redpilot_core::secrets::SecretKind;
use redpilot_core::service::SecretStatus;
use redpilot_core::{AppConfig, Service};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

type CmdResult<T> = Result<T, String>;

fn e(err: anyhow::Error) -> String {
    format!("{err:#}")
}

#[derive(Default)]
struct McpRuntime {
    handle: Option<HttpHandle>,
    error: Option<String>,
}

pub struct AppState {
    pub svc: Arc<Service>,
    mcp: Mutex<McpRuntime>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    enabled: bool,
    running: bool,
    port: u16,
    url: String,
    token: String,
    allow_write: bool,
    error: Option<String>,
    /// 当前可执行文件路径，用于生成 stdio 方式的客户端配置
    exe_path: String,
}

impl AppState {
    pub fn new(svc: Arc<Service>) -> Self {
        Self { svc, mcp: Mutex::new(McpRuntime::default()) }
    }

    fn status(&self, rt: &McpRuntime) -> McpStatus {
        let cfg = self.svc.config().mcp;
        let port = rt.handle.as_ref().map(|h| h.port).unwrap_or(cfg.port);
        McpStatus {
            enabled: cfg.enabled,
            running: rt.handle.is_some(),
            port,
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: cfg.token,
            allow_write: cfg.allow_write,
            error: rt.error.clone(),
            exe_path: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
        }
    }

    /// 按当前配置（重新）启动 HTTP MCP 服务
    pub async fn restart_mcp(&self) -> McpStatus {
        let mut rt = self.mcp.lock().await;
        if let Some(h) = rt.handle.take() {
            h.stop();
            drop(h);
        }
        rt.error = None;
        if self.svc.config().mcp.enabled {
            // 旧监听端口释放需要一点时间，重试几次
            for attempt in 0..6 {
                match mcp::start_http(Arc::clone(&self.svc)).await {
                    Ok(h) => {
                        rt.handle = Some(h);
                        rt.error = None;
                        break;
                    }
                    Err(err) => {
                        rt.error = Some(format!("{err:#}"));
                        if attempt < 5 {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    }
                }
            }
        }
        self.status(&rt)
    }
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.svc.config()
}

#[tauri::command]
pub async fn save_config(state: State<'_, AppState>, config: AppConfig) -> CmdResult<()> {
    let old = state.svc.config().mcp;
    state.svc.save_config(config).map_err(e)?;
    let new = state.svc.config().mcp;
    if old.enabled != new.enabled || old.port != new.port || old.token != new.token {
        state.restart_mcp().await;
    }
    Ok(())
}

#[tauri::command]
pub fn set_secret(state: State<'_, AppState>, kind: SecretKind, value: String) -> CmdResult<()> {
    state.svc.set_secret(kind, &value).map_err(e)
}

#[tauri::command]
pub fn secret_status(state: State<'_, AppState>) -> SecretStatus {
    state.svc.secret_status()
}

#[tauri::command]
pub async fn test_redmine(state: State<'_, AppState>) -> CmdResult<User> {
    state.svc.test_redmine().await.map_err(e)
}

#[tauri::command]
pub async fn test_ai(state: State<'_, AppState>) -> CmdResult<String> {
    state.svc.test_ai().await.map_err(e)
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> CmdResult<SyncReport> {
    state.svc.sync().await.map_err(e)
}

#[tauri::command]
pub fn list_issues(state: State<'_, AppState>, filter: IssueFilter) -> CmdResult<Vec<IssueView>> {
    state.svc.list_issues(&filter).map_err(e)
}

#[tauri::command]
pub async fn issue_detail(state: State<'_, AppState>, id: u64, refresh: bool) -> CmdResult<IssueDetail> {
    state.svc.issue_detail(id, refresh).await.map_err(e)
}

#[tauri::command]
pub fn mark_read(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    state.svc.mark_read(id).map_err(e)
}

#[tauri::command]
pub async fn analyze(state: State<'_, AppState>, ids: Option<Vec<u64>>, force: bool) -> CmdResult<AnalyzeReport> {
    state.svc.analyze(ids, force).await.map_err(e)
}

#[tauri::command]
pub fn set_classification(
    state: State<'_, AppState>,
    id: u64,
    category: Option<String>,
    module: Option<String>,
) -> CmdResult<()> {
    state.svc.set_manual_classification(id, category, module).map_err(e)
}

#[tauri::command]
pub async fn suggest_batches(state: State<'_, AppState>, ids: Option<Vec<u64>>) -> CmdResult<Plan> {
    state.svc.suggest_batches(ids).await.map_err(e)
}

#[tauri::command]
pub fn latest_plan(state: State<'_, AppState>) -> CmdResult<Option<Plan>> {
    state.svc.latest_plan().map_err(e)
}

#[tauri::command]
pub async fn draft_note(state: State<'_, AppState>, ids: Vec<u64>, key_points: String) -> CmdResult<String> {
    state.svc.draft_note(&ids, &key_points).await.map_err(e)
}

#[tauri::command]
pub async fn resolve_issue(state: State<'_, AppState>, req: ResolveRequest) -> CmdResult<ResolveOutcome> {
    state.svc.resolve(&req, "app").await.map_err(e)
}

#[tauri::command]
pub async fn batch_resolve(state: State<'_, AppState>, reqs: Vec<ResolveRequest>) -> CmdResult<Vec<ResolveOutcome>> {
    Ok(state.svc.batch_resolve(&reqs, "app").await)
}

#[tauri::command]
pub async fn request_more_info(state: State<'_, AppState>, id: u64, question: String) -> CmdResult<ResolveOutcome> {
    state.svc.request_more_info(id, &question).await.map_err(e)
}

#[tauri::command]
pub async fn start_progress(state: State<'_, AppState>, id: u64) -> CmdResult<ResolveOutcome> {
    state.svc.start_progress(id).await.map_err(e)
}

#[tauri::command]
pub async fn change_status(
    state: State<'_, AppState>,
    id: u64,
    status_id: u64,
    notes: Option<String>,
) -> CmdResult<ResolveOutcome> {
    state.svc.change_status(id, status_id, notes).await.map_err(e)
}

/// 前端以原始二进制发送文件内容，文件名放在 `x-filename` 头里（URL 编码）
#[tauri::command]
pub fn stage_attachment(state: State<'_, AppState>, request: Request<'_>) -> CmdResult<StagedFile> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("附件数据格式错误".into());
    };
    let name = request
        .headers()
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .map(|v| percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string())
        .unwrap_or_default();
    state.svc.stage_attachment(&name, bytes).map_err(e)
}

/// 附件原始内容，前端转成 Blob 显示图片
#[tauri::command]
pub async fn attachment_bytes(state: State<'_, AppState>, id: u64) -> CmdResult<tauri::ipc::Response> {
    let bytes = state.svc.attachment(id).await.map_err(e)?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn assignees(state: State<'_, AppState>, project_id: u64) -> CmdResult<Vec<Assignee>> {
    state.svc.assignees(project_id).await.map_err(e)
}

#[tauri::command]
pub async fn my_projects(state: State<'_, AppState>) -> CmdResult<Vec<ProjectAccess>> {
    state.svc.my_projects().await.map_err(e)
}

#[tauri::command]
pub async fn project_overview(state: State<'_, AppState>, project_id: u64, refresh: bool) -> CmdResult<ProjectOverview> {
    state.svc.project_overview(project_id, refresh).await.map_err(e)
}

#[tauri::command]
pub async fn suggest_assignments(
    state: State<'_, AppState>,
    project_id: u64,
    issue_ids: Vec<u64>,
    use_ai: bool,
) -> CmdResult<Vec<AssignSuggestion>> {
    state.svc.suggest_assignments(project_id, &issue_ids, use_ai).await.map_err(e)
}

#[tauri::command]
pub async fn bulk_assign(state: State<'_, AppState>, items: Vec<AssignItem>, notes: Option<String>) -> CmdResult<Vec<ResolveOutcome>> {
    Ok(state.svc.bulk_assign(&items, notes).await)
}

#[tauri::command]
pub async fn suggest_split(state: State<'_, AppState>, issue_id: u64, use_ai: bool) -> CmdResult<SplitSuggestion> {
    state.svc.suggest_split(issue_id, use_ai).await.map_err(e)
}

#[tauri::command]
pub async fn split_issue(state: State<'_, AppState>, issue_id: u64, parts: Vec<SplitPart>, add_note: bool) -> CmdResult<SplitOutcome> {
    state.svc.split_issue(issue_id, &parts, add_note).await.map_err(e)
}

#[tauri::command]
pub fn recent_changes(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<IssueChange>> {
    state.svc.recent_changes(limit).map_err(e)
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> CmdResult<bool> {
    app.autolaunch().is_enabled().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn link_issues(
    state: State<'_, AppState>,
    id: u64,
    others: Vec<u64>,
    relation_type: String,
) -> CmdResult<Vec<String>> {
    state.svc.link_issues(id, &others, &relation_type).await.map_err(e)
}

#[tauri::command]
pub async fn statuses(state: State<'_, AppState>) -> CmdResult<Vec<NamedId>> {
    state.svc.statuses().await.map_err(e)
}

#[tauri::command]
pub async fn activities(state: State<'_, AppState>) -> CmdResult<Vec<NamedId>> {
    state.svc.activities().await.map_err(e)
}

#[tauri::command]
pub fn stats(state: State<'_, AppState>) -> CmdResult<Stats> {
    state.svc.stats().map_err(e)
}

#[tauri::command]
pub async fn work_report(state: State<'_, AppState>, days: i64) -> CmdResult<String> {
    state.svc.work_report(days).await.map_err(e)
}

#[tauri::command]
pub fn recent_resolutions(state: State<'_, AppState>, days: i64) -> CmdResult<Vec<ResolutionRecord>> {
    state.svc.recent_resolutions(days).map_err(e)
}

#[tauri::command]
pub async fn mcp_status(state: State<'_, AppState>) -> CmdResult<McpStatus> {
    let rt = state.mcp.lock().await;
    Ok(state.status(&rt))
}

#[tauri::command]
pub async fn restart_mcp(state: State<'_, AppState>) -> CmdResult<McpStatus> {
    Ok(state.restart_mcp().await)
}

#[tauri::command]
pub async fn regenerate_mcp_token(state: State<'_, AppState>) -> CmdResult<McpStatus> {
    state.svc.regenerate_mcp_token().map_err(e)?;
    Ok(state.restart_mcp().await)
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> CmdResult<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只能打开 http/https 链接".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|err| err.to_string())
}
