//! MCP 服务：把 RedPilot 的能力暴露给其他 AI（Claude、Cursor、Cherry Studio 等）。
//!
//! - Streamable HTTP：桌面端运行时监听 `http://127.0.0.1:<port>/mcp`，需 Bearer Token；
//! - stdio：`redpilot --mcp-stdio` 或 `redpilot-mcp`，由 MCP 客户端直接拉起。
//!
//! 写操作（回派、追问、备注、改状态、建关联）默认关闭，需在设置中打开“允许外部 AI 写操作”。

use crate::models::{AssignItem, IssueFilter, IssueView, ResolveRequest};
use crate::service::Service;
use anyhow::Result;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

type ToolResult = std::result::Result<String, String>;

fn to_json<T: serde::Serialize>(v: &T) -> ToolResult {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

fn err(e: anyhow::Error) -> String {
    e.to_string()
}

/// 精简的任务摘要，节省调用方 AI 的上下文
fn brief(v: &IssueView, base_url: &str) -> Value {
    let a = &v.analysis;
    json!({
        "id": v.id,
        "subject": v.subject,
        "project": v.project_name,
        "tracker": v.tracker_name,
        "status": v.status_name,
        "priority": v.priority_name,
        "author": v.author_name,
        "assignedTo": v.assigned_to_name,
        "dueDate": v.due_date,
        "overdue": v.overdue,
        "unread": v.unread,
        "changeHint": v.change_hint,
        "updatedOn": v.updated_on,
        "category": a.category,
        "module": a.module,
        "urgency": a.urgency,
        "difficulty": a.difficulty,
        "estHours": a.est_hours,
        "summary": a.summary,
        "missingInfo": a.missing_info,
        "url": format!("{base_url}/issues/{}", v.id),
    })
}

// ---------------- 参数 ----------------

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ListIssuesParams {
    /// 按分类过滤，如：缺陷、需求、咨询、数据处理、权限配置、环境问题、其他
    pub category: Option<String>,
    /// 关键词（匹配标题/描述/提出人/摘要），也可以是 #任务号
    pub keyword: Option<String>,
    /// 按项目 ID 过滤
    pub project_id: Option<u64>,
    /// 是否包含已不在我名下的历史任务，默认 false
    pub include_done: bool,
    /// 只看有新变化未读的任务
    pub unread_only: bool,
    /// 最多返回条数，默认 50
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueIdParams {
    /// Redmine 任务号
    pub issue_id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetIssueParams {
    /// Redmine 任务号
    pub issue_id: u64,
    /// 强制从 Redmine 重新拉取，默认 false（缓存与服务器一致时使用缓存）
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct AnalyzeParams {
    /// 要分析的任务号；留空分析收件箱中全部未分析/有更新的任务
    pub issue_ids: Option<Vec<u64>>,
    /// 已分析过也重新分析
    pub force: bool,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct BatchParams {
    /// 参与规划的任务号；留空使用收件箱全部任务
    pub issue_ids: Option<Vec<u64>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SimilarParams {
    pub issue_id: u64,
    /// 默认 5
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftNoteParams {
    /// 一个或多个（合并处理的）任务号
    pub issue_ids: Vec<u64>,
    /// 处理要点，例如：“原因：缓存未刷新；处理：已清理缓存并加了失效机制”
    pub key_points: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveParams {
    pub issue_id: u64,
    /// 处置说明（会作为备注写入 Redmine，提出人可见）
    pub notes: String,
    /// 目标状态 ID；留空使用用户配置的“处理完成状态”。可用 get_workflow_options 查询
    #[serde(default)]
    pub status_id: Option<u64>,
    /// 是否指派回提出人，默认 true
    #[serde(default)]
    pub assign_to_author: Option<bool>,
    /// 改为指派给指定成员（用户或组 ID，可用 list_assignees 查询）；设置后忽略 assign_to_author
    #[serde(default)]
    pub assign_to_id: Option<u64>,
    /// 完成度（0-100）；留空按用户配置（默认 100）
    #[serde(default)]
    pub done_ratio: Option<u32>,
    /// 同时登记的工时（小时）
    #[serde(default)]
    pub spent_hours: Option<f64>,
    /// 随处置说明上传的附件：本机文件的绝对路径（如截图、日志）。隐藏目录中的文件不能上传
    #[serde(default)]
    pub attachments: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchResolveParams {
    /// 合并处理的任务号
    pub issue_ids: Vec<u64>,
    /// 共用的处置说明
    pub notes: String,
    #[serde(default)]
    pub status_id: Option<u64>,
    /// 每个任务登记的工时（小时）
    #[serde(default)]
    pub spent_hours_each: Option<f64>,
    /// 全部指派给指定成员；留空则各自指派回提出人
    #[serde(default)]
    pub assign_to_id: Option<u64>,
    /// 附件（本机文件绝对路径），会分别上传到每个任务
    #[serde(default)]
    pub attachments: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusParams {
    pub issue_id: u64,
    /// 目标状态 ID，可用 get_workflow_options 查询
    pub status_id: u64,
    /// 同时写入的备注（可选）
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverviewParams {
    pub project_id: u64,
    /// 最多返回多少个任务明细，默认 50
    #[serde(default)]
    pub limit: Option<usize>,
    /// 只看未指派的任务
    #[serde(default)]
    pub unassigned_only: bool,
    /// 只看已逾期的任务
    #[serde(default)]
    pub overdue_only: bool,
    /// 只看某个成员负责的任务
    #[serde(default)]
    pub assignee_id: Option<u64>,
    /// 忽略 5 分钟缓存，重新从 Redmine 读取
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestAssignParams {
    pub project_id: u64,
    pub issue_ids: Vec<u64>,
    /// 用 AI 综合判断（默认 false：用本地算法，按类别默认负责人、相似任务和负载推荐）
    #[serde(default)]
    pub use_ai: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BulkAssignParams {
    /// 每个任务指派给谁
    pub assignments: Vec<BulkAssignItem>,
    /// 写入每个任务的统一说明（可选），被指派人会收到通知
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BulkAssignItem {
    pub issue_id: u64,
    pub assignee_id: u64,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ChangesParams {
    /// 默认 20
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoreInfoParams {
    pub issue_id: u64,
    /// 发给提出人的追问内容
    pub question: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteParams {
    pub issue_id: u64,
    pub notes: String,
    /// 私有备注（仅有权限的成员可见），默认 false
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkParams {
    pub issue_id: u64,
    pub related_issue_ids: Vec<u64>,
    /// relates（默认）| duplicates | blocks | precedes | follows ...
    #[serde(default)]
    pub relation_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyParams {
    pub issue_id: u64,
    pub category: Option<String>,
    pub module: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ReportParams {
    /// 统计最近多少天，默认 7（周报），1 为日报
    pub days: Option<i64>,
}

// ---------------- 服务 ----------------

#[derive(Clone)]
pub struct RedPilotMcp {
    svc: Arc<Service>,
    /// stdio 模式下每次调用前重读配置，以感知桌面端的修改
    reload_config: bool,
    tool_router: ToolRouter<Self>,
}

impl RedPilotMcp {
    fn prepare(&self) {
        if self.reload_config {
            if let Err(e) = self.svc.reload_config() {
                tracing::warn!("重新加载配置失败: {e}");
            }
        }
    }

    fn ensure_write(&self) -> std::result::Result<(), String> {
        self.prepare();
        if self.svc.config().mcp.allow_write {
            Ok(())
        } else {
            Err("写操作未授权：请让用户在 RedPilot → 设置 → MCP 中开启“允许外部 AI 执行写操作”。在此之前你可以先用 draft_resolution_note 起草说明，交给用户确认。".into())
        }
    }

    fn base_url(&self) -> String {
        self.svc.config().redmine.base_url.trim_end_matches('/').to_string()
    }
}

#[tool_router]
impl RedPilotMcp {
    pub fn new(svc: Arc<Service>, reload_config: bool) -> Self {
        Self {
            svc,
            reload_config,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "获取工作概览：待办数、未读、逾期、高优先级、分类分布、最近回派量，以及是否允许写操作。建议首先调用。")]
    async fn get_overview(&self) -> ToolResult {
        self.prepare();
        let stats = self.svc.stats().map_err(err)?;
        let cfg = self.svc.config();
        to_json(&json!({
            "stats": stats,
            "redmineUrl": cfg.redmine.base_url,
            "writeAllowed": cfg.mcp.allow_write,
            "aiReady": self.svc.ai_ready(),
        }))
    }

    #[tool(description = "立即从 Redmine 同步我的待办任务（指派给我/关注的/保存的查询），返回新增与更新的任务号。")]
    async fn sync_issues(&self) -> ToolResult {
        self.prepare();
        to_json(&self.svc.sync().await.map_err(err)?)
    }

    #[tool(description = "列出本地缓存的待办任务（含 AI 分类、模块、紧急度、摘要）。先 sync_issues 可获取最新数据。")]
    async fn list_issues(&self, Parameters(p): Parameters<ListIssuesParams>) -> ToolResult {
        self.prepare();
        let f = IssueFilter {
            include_done: p.include_done,
            category: p.category,
            project_id: p.project_id,
            keyword: p.keyword,
            unread_only: p.unread_only,
            limit: Some(p.limit.unwrap_or(50).min(200)),
        };
        let base = self.base_url();
        let list: Vec<Value> = self.svc.list_issues(&f).map_err(err)?.iter().map(|v| brief(v, &base)).collect();
        to_json(&json!({ "count": list.len(), "issues": list }))
    }

    #[tool(description = "获取单个任务详情：描述、完整沟通记录、附件列表、AI 分析（排查建议、缺失信息、追问话术）和相似历史任务（含当时的处置说明）。")]
    async fn get_issue(&self, Parameters(p): Parameters<GetIssueParams>) -> ToolResult {
        self.prepare();
        let d = self.svc.issue_detail(p.issue_id, p.refresh).await.map_err(err)?;
        let journals: Vec<Value> = d
            .issue
            .journals
            .iter()
            .filter(|j| j.notes.as_deref().is_some_and(|n| !n.trim().is_empty()))
            .map(|j| json!({ "user": j.user.as_ref().map(|u| &u.name), "createdOn": j.created_on, "private": j.private_notes, "notes": j.notes }))
            .collect();
        let attachments: Vec<Value> = d
            .issue
            .attachments
            .iter()
            .map(|a| json!({ "filename": a.filename, "size": a.filesize, "url": a.content_url }))
            .collect();
        to_json(&json!({
            "issue": brief(&d.view, &self.base_url()),
            "description": d.issue.description,
            "analysis": d.view.analysis,
            "journals": journals,
            "attachments": attachments,
            "allowedStatuses": d.issue.allowed_statuses,
            "relations": d.issue.relations,
            "similarIssues": d.similar,
        }))
    }

    #[tool(description = "用 AI 对任务做分类与分析（分类、模块、紧急度、难度、预估工时、一句话摘要、缺失信息、追问话术、排查思路），结果写入本地缓存。")]
    async fn analyze_issues(&self, Parameters(p): Parameters<AnalyzeParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.analyze(p.issue_ids, p.force).await.map_err(err)?)
    }

    #[tool(description = "AI 合并规划：找出可以一起处理的任务组（附理由与思路）、疑似重复任务、建议处理顺序和今日工作计划。")]
    async fn suggest_batches(&self, Parameters(p): Parameters<BatchParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.suggest_batches(p.issue_ids).await.map_err(err)?)
    }

    #[tool(description = "获取最近一次生成的合并规划结果（不调用 AI）。")]
    async fn get_latest_plan(&self) -> ToolResult {
        self.prepare();
        to_json(&self.svc.latest_plan().map_err(err)?)
    }

    #[tool(description = "在本地知识库中检索与指定任务相似的历史任务，返回相似度和当时的处置说明，便于复用经验。")]
    async fn find_similar_issues(&self, Parameters(p): Parameters<SimilarParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.find_similar(p.issue_id, p.limit.unwrap_or(5).min(20)).map_err(err)?)
    }

    #[tool(description = "获取可用的任务状态、工时活动类型，以及用户配置的“处理完成/进行中/反馈”默认状态。")]
    async fn get_workflow_options(&self) -> ToolResult {
        self.prepare();
        let statuses = self.svc.statuses().await.map_err(err)?;
        let activities = self.svc.activities().await.unwrap_or_default();
        let wf = self.svc.config().workflow;
        to_json(&json!({
            "statuses": statuses,
            "activities": activities,
            "defaults": {
                "resolvedStatusId": wf.resolved_status_id,
                "inProgressStatusId": wf.in_progress_status_id,
                "feedbackStatusId": wf.feedback_status_id,
                "setDoneRatio100": wf.set_done_ratio,
                "defaultActivityId": wf.default_activity_id,
            }
        }))
    }

    #[tool(description = "列出任务所属项目中可以指派的成员（用户和组），用于把任务转给其他人处理。")]
    async fn list_assignees(&self, Parameters(p): Parameters<IssueIdParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.assignees_for_issue(p.issue_id).await.map_err(err)?)
    }

    #[tool(description = "列出我参与的项目及我的角色，标记出我担任管理角色的项目（项目经理视角的入口）。")]
    async fn list_my_projects(&self) -> ToolResult {
        self.prepare();
        to_json(&self.svc.my_projects().await.map_err(err)?)
    }

    #[tool(description = "项目全局视角：全部未关闭任务的统计（未指派、逾期、停滞、各成员负载）、版本进度、近 8 周新增与关闭趋势，以及按条件筛选的任务明细。")]
    async fn get_project_overview(&self, Parameters(p): Parameters<OverviewParams>) -> ToolResult {
        self.prepare();
        let ov = self.svc.project_overview(p.project_id, p.refresh).await.map_err(err)?;
        let mut by_assignee: Vec<(String, usize, usize)> = Vec::new();
        for i in &ov.issues {
            let name = if i.assignee.is_empty() { "（未指派）".to_string() } else { i.assignee.clone() };
            match by_assignee.iter_mut().find(|x| x.0 == name) {
                Some(x) => {
                    x.1 += 1;
                    x.2 += i.overdue as usize;
                }
                None => by_assignee.push((name, 1, i.overdue as usize)),
            }
        }
        by_assignee.sort_by(|a, b| b.1.cmp(&a.1));
        let picked: Vec<Value> = ov
            .issues
            .iter()
            .filter(|i| !p.unassigned_only || i.assignee_id.is_none())
            .filter(|i| !p.overdue_only || i.overdue)
            .filter(|i| p.assignee_id.is_none() || i.assignee_id == p.assignee_id)
            .take(p.limit.unwrap_or(50).min(500))
            .map(|i| {
                json!({
                    "id": i.id, "subject": i.subject, "tracker": i.tracker, "status": i.status, "priority": i.priority,
                    "assignee": i.assignee, "assigneeId": i.assignee_id, "author": i.author, "category": i.category,
                    "version": i.version, "dueDate": i.due_date, "overdue": i.overdue, "staleDays": i.stale_days,
                    "url": format!("{}/issues/{}", self.base_url(), i.id),
                })
            })
            .collect();
        to_json(&json!({
            "projectId": ov.project_id,
            "fetchedAt": ov.fetched_at,
            "openTotal": ov.issues.len(),
            "truncated": ov.truncated,
            "unassigned": ov.issues.iter().filter(|i| i.assignee_id.is_none()).count(),
            "overdue": ov.issues.iter().filter(|i| i.overdue).count(),
            "stale14d": ov.issues.iter().filter(|i| i.stale_days >= 14).count(),
            "byAssignee": by_assignee.iter().map(|(n, open, overdue)| json!({ "assignee": n, "open": open, "overdue": overdue })).collect::<Vec<_>>(),
            "versions": ov.versions,
            "weekly": ov.weekly,
            "members": ov.members,
            "issues": picked,
        }))
    }

    #[tool(description = "为一批任务推荐负责人（每个任务给出建议人选和理由）。默认用本地算法（类别默认负责人、相似任务由谁负责、负载），use_ai=true 时由 AI 综合判断。只给建议，不修改 Redmine。")]
    async fn suggest_assignments(&self, Parameters(p): Parameters<SuggestAssignParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.suggest_assignments(p.project_id, &p.issue_ids, p.use_ai).await.map_err(err)?)
    }

    #[tool(description = "【写操作】批量转派：把每个任务指派给指定成员（可以各不相同），可附带统一说明。建议先用 suggest_assignments 生成方案并让用户确认。")]
    async fn bulk_assign(&self, Parameters(p): Parameters<BulkAssignParams>) -> ToolResult {
        self.ensure_write()?;
        let items: Vec<AssignItem> = p
            .assignments
            .iter()
            .map(|a| AssignItem { issue_id: a.issue_id, assignee_id: a.assignee_id })
            .collect();
        to_json(&self.svc.bulk_assign(&items, p.notes).await)
    }

    #[tool(description = "根据处理要点起草规范的处置说明（问题原因/处理方法/处理结果/影响范围/验证建议），只生成不提交。多个任务时生成一份共用说明。")]
    async fn draft_resolution_note(&self, Parameters(p): Parameters<DraftNoteParams>) -> ToolResult {
        self.prepare();
        self.svc.draft_note(&p.issue_ids, &p.key_points).await.map_err(err)
    }

    #[tool(description = "【写操作】处理并指派：写入处置说明、切换到完成状态、默认指派回提出人（也可用 assign_to_id 转给其他成员），可同时登记工时和上传附件。提交前请确保说明内容已经用户确认。")]
    async fn resolve_issue(&self, Parameters(p): Parameters<ResolveParams>) -> ToolResult {
        self.ensure_write()?;
        let req = ResolveRequest {
            issue_id: p.issue_id,
            notes: p.notes,
            status_id: p.status_id,
            assign_to_author: p.assign_to_author.unwrap_or(true),
            assign_to_id: p.assign_to_id,
            done_ratio: p.done_ratio,
            spent_hours: p.spent_hours,
            activity_id: None,
            private_notes: false,
            attachments: p.attachments.unwrap_or_default(),
        };
        to_json(&self.svc.resolve(&req, "mcp").await.map_err(err)?)
    }

    #[tool(description = "【写操作】批量回派一组合并处理的任务：每个任务写入同一份处置说明并指派回各自的提出人。")]
    async fn batch_resolve(&self, Parameters(p): Parameters<BatchResolveParams>) -> ToolResult {
        self.ensure_write()?;
        let reqs: Vec<ResolveRequest> = p
            .issue_ids
            .iter()
            .map(|&id| ResolveRequest {
                issue_id: id,
                notes: p.notes.clone(),
                status_id: p.status_id,
                assign_to_author: true,
                assign_to_id: p.assign_to_id,
                done_ratio: None,
                spent_hours: p.spent_hours_each,
                activity_id: None,
                private_notes: false,
                attachments: p.attachments.clone().unwrap_or_default(),
            })
            .collect();
        to_json(&self.svc.batch_resolve(&reqs, "mcp").await)
    }

    #[tool(description = "【写操作】信息不足时向提出人追问：写入追问内容，状态改为“反馈”，并指派给提出人。")]
    async fn request_more_info(&self, Parameters(p): Parameters<MoreInfoParams>) -> ToolResult {
        self.ensure_write()?;
        to_json(&self.svc.request_more_info(p.issue_id, &p.question).await.map_err(err)?)
    }

    #[tool(description = "【写操作】把任务状态切换为用户配置的“进行中”。")]
    async fn start_progress(&self, Parameters(p): Parameters<IssueIdParams>) -> ToolResult {
        self.ensure_write()?;
        to_json(&self.svc.start_progress(p.issue_id).await.map_err(err)?)
    }

    #[tool(description = "【写操作】修改任务状态（不改指派），可附带备注。工作流不允许时会返回当前可选的状态。")]
    async fn update_status(&self, Parameters(p): Parameters<StatusParams>) -> ToolResult {
        self.ensure_write()?;
        to_json(&self.svc.change_status(p.issue_id, p.status_id, p.notes).await.map_err(err)?)
    }

    #[tool(description = "查看最近同步到的他人变更：提出人回复、任务被转回给我、优先级/截止日期/状态被调整。适合回答“有什么新动态”。")]
    async fn get_recent_changes(&self, Parameters(p): Parameters<ChangesParams>) -> ToolResult {
        self.prepare();
        to_json(&self.svc.recent_changes(p.limit.unwrap_or(20).min(100)).map_err(err)?)
    }

    #[tool(description = "【写操作】给任务添加一条备注（不改状态和指派）。")]
    async fn add_note(&self, Parameters(p): Parameters<NoteParams>) -> ToolResult {
        self.ensure_write()?;
        self.svc.add_note(p.issue_id, &p.notes, p.private).await.map_err(err)?;
        Ok("已添加备注".into())
    }

    #[tool(description = "【写操作】在 Redmine 上建立任务关联，例如把合并处理或重复的任务互相关联。")]
    async fn link_issues(&self, Parameters(p): Parameters<LinkParams>) -> ToolResult {
        self.ensure_write()?;
        let rt = p.relation_type.unwrap_or_else(|| "relates".into());
        let errors = self.svc.link_issues(p.issue_id, &p.related_issue_ids, &rt).await.map_err(err)?;
        to_json(&json!({ "ok": errors.is_empty(), "errors": errors }))
    }

    #[tool(description = "人工修正任务的分类/模块（只保存在本地，不改 Redmine）。修正记录会作为样例提升后续 AI 分类准确度。")]
    async fn set_classification(&self, Parameters(p): Parameters<ClassifyParams>) -> ToolResult {
        self.prepare();
        self.svc
            .set_manual_classification(p.issue_id, p.category, p.module)
            .map_err(err)?;
        Ok("已保存".into())
    }

    #[tool(description = "根据最近通过 RedPilot 回派的处置记录，用 AI 生成日报/周报（Markdown）。")]
    async fn work_report(&self, Parameters(p): Parameters<ReportParams>) -> ToolResult {
        self.prepare();
        self.svc.work_report(p.days.unwrap_or(7)).await.map_err(err)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RedPilotMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("redpilot", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "RedPilot 是用户的 Redmine 工单助手。典型流程：get_overview → sync_issues → list_issues / suggest_batches → \
                 get_issue 查看详情 → draft_resolution_note 起草说明 → 用户确认后 resolve_issue 回派给提出人。\
                 写操作会直接修改 Redmine 并通知相关人员，执行前务必让用户确认内容。",
            )
    }
}

// ---------------- 传输层 ----------------

pub struct HttpHandle {
    pub port: u16,
    ct: CancellationToken,
}

impl HttpHandle {
    pub fn stop(&self) {
        self.ct.cancel();
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}

async fn bearer_auth(State(token): State<Arc<String>>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if token.is_empty() {
        return next.run(req).await;
    }
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t.trim() == token.as_str());
    if ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
    }
}

/// 在 127.0.0.1 上启动 Streamable HTTP MCP 服务
pub async fn start_http(svc: Arc<Service>) -> Result<HttpHandle> {
    let cfg = svc.config().mcp;
    let ct = CancellationToken::new();
    let server_cfg = StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token());
    let factory_svc = Arc::clone(&svc);
    let mcp: StreamableHttpService<RedPilotMcp, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(RedPilotMcp::new(Arc::clone(&factory_svc), false)),
        Default::default(),
        server_cfg,
    );
    let router = axum::Router::new()
        .nest_service("/mcp", mcp)
        .layer(axum::middleware::from_fn_with_state(Arc::new(cfg.token.clone()), bearer_auth))
        .route("/health", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port))
        .await
        .map_err(|e| anyhow::anyhow!("MCP 端口 {} 无法监听（可能被占用）: {e}", cfg.port))?;
    let port = listener.local_addr()?.port();
    let shutdown = ct.clone();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await
        {
            tracing::error!("MCP HTTP 服务异常退出: {e}");
        }
    });
    tracing::info!("MCP 服务已启动: http://127.0.0.1:{port}/mcp");
    Ok(HttpHandle { port, ct })
}

/// 以 stdio 方式运行 MCP 服务（阻塞直到客户端断开）
pub async fn serve_stdio(svc: Arc<Service>) -> Result<()> {
    let running = RedPilotMcp::new(svc, true).serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
