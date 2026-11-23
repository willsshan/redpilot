//! 内置 AI 对话助手：通过 OpenAI Function Calling 直接查询和操作 Redmine。
//!
//! - 读操作（查项目、查项目问题、查详情、查状态/成员、查分派台、同步）在对话轮次中自动执行；
//! - 写操作（关闭问题、改状态、回派、指派、加备注、追问、新建任务、建立关联）默认拦截为待确认操作卡片，
//!   由用户在对话框里一键确认后再执行；用户也可在对话框开启「自动执行写操作」跳过确认。
//! - 遵循「不发送给 AI 的项目」与「发送前脱敏」设置。

use crate::ai::{ChatMessage, ToolCall};
use crate::models::{AssignItem, IssueFilter, ResolveRequest};
use crate::redmine::{truncate, IssueQuery, NewIssue};
use crate::service::Service;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

const MAX_TOOL_ROUNDS: usize = 8;
const MAX_HISTORY_MESSAGES: usize = 20;
const MAX_TOOL_RESULT_CHARS: usize = 12000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryItem {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub messages: Vec<ChatHistoryItem>,
    #[serde(default)]
    pub auto_execute_writes: bool,
    #[serde(default)]
    pub recent_project_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolStep {
    pub tool: String,
    pub label: String,
    pub summary: String,
    pub ok: bool,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAction {
    pub id: String,
    pub tool: String,
    pub title: String,
    pub details: Vec<String>,
    pub args: Value,
    #[serde(default)]
    pub project_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub content: String,
    pub steps: Vec<ChatToolStep>,
    pub pending_actions: Vec<PendingAction>,
    pub touched_project_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionExecutionResult {
    pub id: String,
    pub tool: String,
    pub title: String,
    pub ok: bool,
    pub summary: String,
    pub project_ids: Vec<u64>,
}

fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "close_issues"
            | "update_issues_status"
            | "resolve_issue"
            | "assign_issues"
            | "add_note"
            | "request_more_info"
            | "create_issue"
            | "link_issues"
    )
}

fn tool_label(name: &str) -> &'static str {
    match name {
        "list_projects" => "查看项目列表",
        "get_project_issues" => "查询项目问题",
        "list_my_issues" => "查询我的待办",
        "get_issue" => "查看任务详情",
        "list_statuses" => "查询可用状态",
        "list_project_members" => "查询项目成员",
        "get_dispatch_queue" => "查看分派台",
        "get_overview_stats" => "查看工作统计",
        "sync_issues" => "同步 Redmine 任务",
        "close_issues" => "关闭问题",
        "update_issues_status" => "修改问题状态",
        "resolve_issue" => "处理完成并回派",
        "assign_issues" => "指派负责人",
        "add_note" => "添加备注",
        "request_more_info" => "追问提出人",
        "create_issue" => "新建问题",
        "link_issues" => "建立问题关联",
        _ => "调用工具",
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "list_projects",
            "列出当前用户可访问的所有 Redmine 项目（含项目 ID、名称、角色、是否管理员、是否最近使用）。用户提到项目名称时应先调用此工具确认 project_id。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool_def(
            "get_project_issues",
            "查询指定 Redmine 项目下的问题列表（不限于指派给我的任务）。支持按状态（open/closed/all）、关键词、负责人、是否未指派、是否逾期筛选。",
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "integer", "description": "项目 ID（先通过 list_projects 获取）" },
                    "status": { "type": "string", "enum": ["open", "closed", "all"], "description": "状态范围：open（默认，未关闭）、closed（已关闭）、all（全部）" },
                    "keyword": { "type": "string", "description": "可选关键词，按标题/描述/提出人/负责人/分类过滤" },
                    "assignee": { "type": "string", "description": "可选负责人姓名（模糊匹配）" },
                    "unassigned_only": { "type": "boolean", "description": "仅看未指派的问题" },
                    "overdue_only": { "type": "boolean", "description": "仅看已逾期的问题" },
                    "limit": { "type": "integer", "description": "最多返回条数，默认 30，最大 100" },
                    "refresh": { "type": "boolean", "description": "是否跳过缓存强制刷新，默认 false" }
                },
                "required": ["project_id"]
            }),
        ),
        tool_def(
            "list_my_issues",
            "查询我收件箱/本地缓存的待办任务（指派给我、我关注的或自定义查询同步的任务），含 AI 分类和摘要。",
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "integer", "description": "可选，按项目 ID 筛选" },
                    "keyword": { "type": "string", "description": "可选关键词或 #任务号" },
                    "category": { "type": "string", "description": "可选分类，如 缺陷、需求、咨询" },
                    "unread_only": { "type": "boolean", "description": "仅看未读" },
                    "include_done": { "type": "boolean", "description": "是否包含已不在收件箱的历史任务" },
                    "limit": { "type": "integer", "description": "最多返回条数，默认 30" }
                }
            }),
        ),
        tool_def(
            "get_issue",
            "获取单个任务的完整详情：标题、描述、当前状态、可切换的目标状态（allowedStatuses）、提出人、负责人、沟通记录和子任务。",
            json!({
                "type": "object",
                "properties": {
                    "issue_id": { "type": "integer", "description": "Redmine 任务号" },
                    "refresh": { "type": "boolean", "description": "是否从 Redmine 强制拉取最新详情，默认 true" }
                },
                "required": ["issue_id"]
            }),
        ),
        tool_def(
            "list_statuses",
            "获取 Redmine 全部任务状态（含 ID、名称、isClosed 是否为关闭状态）及用户工作流默认状态配置。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool_def(
            "list_project_members",
            "列出指定项目中可指派的成员（用户和组 ID、名称、角色）。指派/转派前先调用此工具确认成员 ID。",
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "integer", "description": "项目 ID" }
                },
                "required": ["project_id"]
            }),
        ),
        tool_def(
            "get_dispatch_queue",
            "查看分派台待分派任务及推荐负责人（可按 project_id 过滤）。",
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "integer", "description": "可选，只看指定项目的待分派任务" }
                }
            }),
        ),
        tool_def(
            "get_overview_stats",
            "获取当前待办概览统计：待办数、未读、逾期、高优先级、近 7 天回派数、各项目待办分布。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool_def(
            "sync_issues",
            "立即从 Redmine 同步最新的待办任务到本地缓存。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        // ---------------- 写操作 ----------------
        tool_def(
            "close_issues",
            "【写操作】关闭一个或多个 Redmine 问题。未指定 status_id 时自动选用 Redmine 的关闭状态（isClosed=true），可附带关闭说明。",
            json!({
                "type": "object",
                "properties": {
                    "issue_ids": { "type": "array", "items": { "type": "integer" }, "description": "要关闭的任务号列表" },
                    "status_id": { "type": "integer", "description": "可选，指定关闭状态 ID；不填则自动选用默认关闭状态" },
                    "notes": { "type": "string", "description": "可选，关闭时写入的备注说明" },
                    "done_ratio": { "type": "integer", "description": "可选，完成度 0-100（默认 100）" }
                },
                "required": ["issue_ids"]
            }),
        ),
        tool_def(
            "update_issues_status",
            "【写操作】修改一个或多个问题的状态（不改指派），可附带备注。调用前请先用 list_statuses 或 get_issue 确认 status_id。",
            json!({
                "type": "object",
                "properties": {
                    "issue_ids": { "type": "array", "items": { "type": "integer" }, "description": "任务号列表" },
                    "status_id": { "type": "integer", "description": "目标状态 ID" },
                    "notes": { "type": "string", "description": "可选备注" }
                },
                "required": ["issue_ids", "status_id"]
            }),
        ),
        tool_def(
            "resolve_issue",
            "【写操作】处理完成并回派问题：写入处置说明、切换到完成状态、默认指派回提出人（也可用 assign_to_id 指定成员），可选登记工时。",
            json!({
                "type": "object",
                "properties": {
                    "issue_id": { "type": "integer", "description": "任务号" },
                    "notes": { "type": "string", "description": "处置说明（必填）" },
                    "status_id": { "type": "integer", "description": "可选目标状态 ID，不填使用默认完成状态" },
                    "assign_to_author": { "type": "boolean", "description": "是否指派回提出人，默认 true" },
                    "assign_to_id": { "type": "integer", "description": "可选，改为指派给指定成员 ID" },
                    "spent_hours": { "type": "number", "description": "可选，登记工时（小时）" }
                },
                "required": ["issue_id", "notes"]
            }),
        ),
        tool_def(
            "assign_issues",
            "【写操作】将一个或多个问题指派给指定负责人，可附带统一说明。调用前请先用 list_project_members 确认成员 ID。",
            json!({
                "type": "object",
                "properties": {
                    "issue_ids": { "type": "array", "items": { "type": "integer" }, "description": "要指派的任务号列表" },
                    "assignee_id": { "type": "integer", "description": "目标负责人（成员或组）ID" },
                    "notes": { "type": "string", "description": "可选指派说明" }
                },
                "required": ["issue_ids", "assignee_id"]
            }),
        ),
        tool_def(
            "add_note",
            "【写操作】给指定问题添加一条备注（不改状态和指派）。",
            json!({
                "type": "object",
                "properties": {
                    "issue_id": { "type": "integer", "description": "任务号" },
                    "notes": { "type": "string", "description": "备注内容" },
                    "private": { "type": "boolean", "description": "是否为私有备注，默认 false" }
                },
                "required": ["issue_id", "notes"]
            }),
        ),
        tool_def(
            "request_more_info",
            "【写操作】向提出人追问补充信息：写入追问内容、状态改为“反馈”并指派回提出人。",
            json!({
                "type": "object",
                "properties": {
                    "issue_id": { "type": "integer", "description": "任务号" },
                    "question": { "type": "string", "description": "追问内容" }
                },
                "required": ["issue_id", "question"]
            }),
        ),
        tool_def(
            "create_issue",
            "【写操作】在指定项目下新建一个 Redmine 问题。",
            json!({
                "type": "object",
                "properties": {
                    "project_id": { "type": "integer", "description": "所属项目 ID" },
                    "subject": { "type": "string", "description": "问题标题" },
                    "description": { "type": "string", "description": "问题描述（可选）" },
                    "assigned_to_id": { "type": "integer", "description": "可选负责人 ID" },
                    "due_date": { "type": "string", "description": "可选截止日期 YYYY-MM-DD" },
                    "parent_issue_id": { "type": "integer", "description": "可选父任务号" }
                },
                "required": ["project_id", "subject"]
            }),
        ),
        tool_def(
            "link_issues",
            "【写操作】在 Redmine 上建立任务关联（relates / duplicates / blocks 等）。",
            json!({
                "type": "object",
                "properties": {
                    "issue_id": { "type": "integer", "description": "主任务号" },
                    "related_issue_ids": { "type": "array", "items": { "type": "integer" }, "description": "关联的目标任务号列表" },
                    "relation_type": { "type": "string", "description": "关联类型：relates（默认）、duplicates、blocks 等" }
                },
                "required": ["issue_id", "related_issue_ids"]
            }),
        ),
    ]
}

fn tool_def(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

fn build_system_prompt(today: &str, me: Option<&str>, auto_write: bool) -> String {
    let who = me.map(|m| format!("当前登录用户：{m}。")).unwrap_or_default();
    let write_rule = if auto_write {
        "当前已开启「自动执行写操作」：调用写工具会立即修改 Redmine，执行后请如实汇报成功和失败的明细。"
    } else {
        "当前处于「写操作需确认」安全模式：你仍应正常调用写工具，系统会自动拦截并生成操作确认卡片交由用户点击确认。收到 pending_confirmation 结果后，请简洁说明即将执行的操作并提醒用户点击卡片上的「确认执行」，切勿声称已经修改完成，也不要重复调用同一个写工具。"
    };
    format!(
        "你是 RedPilot 内置的 Redmine 智能对话助手。你可以直接调用工具帮助用户查询、分析和操作 Redmine 项目与问题。\n\
        今天日期：{today}。{who}\n\n\
        工作准则：\n\
        1. 用户提到项目名称时，先调用 `list_projects` 找到准确的 `project_id`，不要猜测项目 ID、成员 ID 或状态 ID。\n\
        2. 查看某项目的问题用 `get_project_issues`（可筛选 open/closed/all、关键词、负责人、逾期、未指派）；查看用户自己的收件箱待办用 `list_my_issues`。\n\
        3. 关闭问题优先用 `close_issues`（系统会自动匹配 Redmine 的关闭状态）；若用户要求改为特定状态，先用 `list_statuses` 确认状态 ID 再调用 `update_issues_status`。\n\
        4. 如果用户指令指代模糊（例如要关闭“某个项目的问题”但未明确哪几条，或匹配到多条），先查出候选问题列出清单供用户确认，不要盲目关闭全部问题。\n\
        5. {write_rule}\n\
        6. 回复使用简洁清晰的中文 Markdown，提到具体问题时统一写成 `#任务号`（如 `#1234`），前端会自动支持点击跳转。绝不编造工具未返回的数据。"
    )
}

/// 发起一轮 AI 对话：自动执行读工具；写工具按设置拦截为确认卡片或直接执行。
pub async fn chat(svc: &Arc<Service>, req: ChatRequest) -> Result<ChatReply> {
    let ai = svc.ai_client_for_chat()?;
    let today = crate::local_today();
    let me_name = svc.me().await.ok().map(|u| u.display_name());
    let mut msgs = vec![ChatMessage::system(build_system_prompt(
        &today,
        me_name.as_deref(),
        req.auto_execute_writes,
    ))];

    let start = req.messages.len().saturating_sub(MAX_HISTORY_MESSAGES);
    for m in &req.messages[start..] {
        let text = m.content.trim();
        if text.is_empty() {
            continue;
        }
        match m.role.as_str() {
            "user" => msgs.push(ChatMessage::user(text)),
            "assistant" => msgs.push(ChatMessage::assistant(text)),
            _ => {}
        }
    }
    if msgs.len() == 1 {
        return Err(anyhow!("请输入要咨询或操作的内容"));
    }

    let tools = tool_definitions();
    let mut steps: Vec<ChatToolStep> = Vec::new();
    let mut pending_actions: Vec<PendingAction> = Vec::new();
    let mut touched_projects: Vec<u64> = Vec::new();

    for round in 0..MAX_TOOL_ROUNDS {
        // 如果已经收集了待确认写操作，最后补一轮纯文本总结（不传 tools，防止模型重复调用）
        let active_tools: &[Value] = if !pending_actions.is_empty() || round == MAX_TOOL_ROUNDS - 1 {
            &[]
        } else {
            &tools
        };
        let reply = ai.chat_with_tools(&msgs, active_tools).await?;
        svc.record_usage_for("chat", &ai.model, reply.usage);

        if reply.tool_calls.is_empty() {
            return Ok(ChatReply {
                content: reply.content.trim().to_string(),
                steps,
                pending_actions,
                touched_project_ids: dedup_u64(touched_projects),
            });
        }

        let content_opt = if reply.content.trim().is_empty() {
            None
        } else {
            Some(reply.content.clone())
        };
        msgs.push(ChatMessage::assistant_tool_calls(content_opt, reply.tool_calls.clone()));

        for call in reply.tool_calls {
            let name = call.function.name.as_str();
            let args = parse_args(&call.function.arguments);
            if is_write_tool(name) && !req.auto_execute_writes {
                match build_pending_action(svc, &call, &args).await {
                    Ok(action) => {
                        touched_projects.extend(&action.project_ids);
                        let summary = action.title.clone();
                        pending_actions.push(action);
                        msgs.push(ChatMessage::tool_result(
                            call.id,
                            json!({
                                "status": "pending_confirmation",
                                "action": summary,
                                "message": "已生成待用户确认的操作卡片，等待用户点击「确认执行」。请简洁向用户说明待执行的操作并提示确认，不要重复调用写工具。"
                            })
                            .to_string(),
                        ));
                    }
                    Err(e) => {
                        steps.push(ChatToolStep {
                            tool: name.to_string(),
                            label: tool_label(name).to_string(),
                            summary: format!("参数准备失败：{e}"),
                            ok: false,
                            write: true,
                        });
                        msgs.push(ChatMessage::tool_result(
                            call.id,
                            json!({ "error": format!("{e:#}") }).to_string(),
                        ));
                    }
                }
                continue;
            }

            let write = is_write_tool(name);
            let outcome = if write {
                execute_write_tool(svc, name, &args).await
            } else {
                execute_read_tool(svc, name, &args, &req.recent_project_ids).await
            };
            match outcome {
                Ok(res) => {
                    touched_projects.extend(&res.project_ids);
                    steps.push(ChatToolStep {
                        tool: name.to_string(),
                        label: tool_label(name).to_string(),
                        summary: res.summary,
                        ok: res.ok,
                        write,
                    });
                    msgs.push(ChatMessage::tool_result(call.id, truncate_tool_json(&res.payload)));
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    steps.push(ChatToolStep {
                        tool: name.to_string(),
                        label: tool_label(name).to_string(),
                        summary: truncate(&msg, 120),
                        ok: false,
                        write,
                    });
                    msgs.push(ChatMessage::tool_result(call.id, json!({ "error": msg }).to_string()));
                }
            }
        }
    }

    Ok(ChatReply {
        content: if pending_actions.is_empty() {
            "已完成上述查询，如需继续操作请告诉我。".into()
        } else {
            "已准备好以下操作，请核对后点击确认执行。".into()
        },
        steps,
        pending_actions,
        touched_project_ids: dedup_u64(touched_projects),
    })
}

/// 执行用户在对话框确认卡片上点击「确认执行」的写操作
pub async fn execute_actions(svc: &Arc<Service>, actions: Vec<PendingAction>) -> Vec<ActionExecutionResult> {
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        if !is_write_tool(&action.tool) {
            results.push(ActionExecutionResult {
                id: action.id,
                tool: action.tool,
                title: action.title,
                ok: false,
                summary: "不支持的操作类型".into(),
                project_ids: action.project_ids,
            });
            continue;
        }
        match execute_write_tool(svc, &action.tool, &action.args).await {
            Ok(res) => {
                let mut pids = action.project_ids;
                pids.extend(res.project_ids);
                results.push(ActionExecutionResult {
                    id: action.id,
                    tool: action.tool,
                    title: action.title,
                    ok: res.ok,
                    summary: res.summary,
                    project_ids: dedup_u64(pids),
                });
            }
            Err(e) => {
                results.push(ActionExecutionResult {
                    id: action.id,
                    tool: action.tool,
                    title: action.title,
                    ok: false,
                    summary: format!("{e:#}"),
                    project_ids: action.project_ids,
                });
            }
        }
    }
    results
}

struct ToolOutcome {
    payload: Value,
    summary: String,
    ok: bool,
    project_ids: Vec<u64>,
}

fn parse_args(raw: &str) -> Value {
    let t = raw.trim();
    if t.is_empty() {
        return json!({});
    }
    serde_json::from_str(t).unwrap_or_else(|_| json!({}))
}

fn truncate_tool_json(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "{}".into());
    if s.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return s;
    }
    truncate(&s, MAX_TOOL_RESULT_CHARS)
}

fn dedup_u64(ids: Vec<u64>) -> Vec<u64> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| *id > 0 && seen.insert(*id)).collect()
}

fn is_project_excluded(svc: &Service, project_id: u64) -> bool {
    svc.config().ai.excluded_project_ids.contains(&project_id)
}

fn get_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok()))
    })
}

fn get_u64_list(args: &Value, key: &str) -> Vec<u64> {
    let mut out = Vec::new();
    if let Some(arr) = args.get(key).and_then(Value::as_array) {
        for v in arr {
            if let Some(id) = v
                .as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok()))
            {
                if id > 0 && !out.contains(&id) {
                    out.push(id);
                }
            }
        }
    }
    out
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

// ---------------- 读工具执行 ----------------

async fn execute_read_tool(svc: &Arc<Service>, name: &str, args: &Value, recent_ids: &[u64]) -> Result<ToolOutcome> {
    match name {
        "list_projects" => {
            let mut projects = svc.my_projects().await?;
            // 按最近使用的项目优先排序
            if !recent_ids.is_empty() {
                projects.sort_by_key(|p| recent_ids.iter().position(|&id| id == p.id).unwrap_or(usize::MAX));
            }
            let recent_set: HashSet<u64> = recent_ids.iter().take(5).copied().collect();
            let excluded: HashSet<u64> = svc.config().ai.excluded_project_ids.into_iter().collect();
            let list: Vec<Value> = projects
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "name": p.name,
                        "isManager": p.is_manager,
                        "roles": p.roles,
                        "recent": recent_set.contains(&p.id),
                        "aiExcluded": excluded.contains(&p.id),
                    })
                })
                .collect();
            Ok(ToolOutcome {
                summary: format!("共 {} 个项目", list.len()),
                payload: json!({ "count": list.len(), "projects": list }),
                ok: true,
                project_ids: Vec::new(),
            })
        }
        "get_project_issues" => {
            let pid = get_u64(args, "project_id").ok_or_else(|| anyhow!("缺少 project_id"))?;
            if is_project_excluded(svc, pid) {
                return Err(anyhow!("该项目已在设置中标记为「不发送给 AI」，无法向 AI 展示其问题内容"));
            }
            let status = get_str(args, "status").unwrap_or("open");
            let keyword = get_str(args, "keyword").map(|s| s.to_lowercase());
            let assignee_q = get_str(args, "assignee").map(|s| s.to_lowercase());
            let unassigned_only = args.get("unassigned_only").and_then(Value::as_bool).unwrap_or(false);
            let overdue_only = args.get("overdue_only").and_then(Value::as_bool).unwrap_or(false);
            let limit = get_u64(args, "limit").unwrap_or(30).clamp(1, 100) as usize;
            let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(false);

            let (total, issues_json) = if status == "open" {
                let ov = svc.project_overview(pid, refresh).await?;
                let filtered: Vec<_> = ov
                    .issues
                    .iter()
                    .filter(|i| !unassigned_only || i.assignee_id.is_none())
                    .filter(|i| !overdue_only || i.overdue)
                    .filter(|i| match &assignee_q {
                        Some(q) => i.assignee.to_lowercase().contains(q),
                        None => true,
                    })
                    .filter(|i| match &keyword {
                        Some(k) => {
                            format!(
                                "#{} {} {} {} {} {}",
                                i.id,
                                i.subject,
                                i.description,
                                i.author,
                                i.assignee,
                                i.category.as_deref().unwrap_or("")
                            )
                            .to_lowercase()
                            .contains(k)
                        }
                        None => true,
                    })
                    .collect();
                let total = filtered.len();
                let items: Vec<Value> = filtered
                    .into_iter()
                    .take(limit)
                    .map(|i| {
                        json!({
                            "id": i.id,
                            "subject": svc.mask_for_ai(&i.subject),
                            "tracker": i.tracker,
                            "status": i.status,
                            "priority": i.priority,
                            "author": i.author,
                            "assignee": i.assignee,
                            "assigneeId": i.assignee_id,
                            "category": i.category,
                            "version": i.version,
                            "dueDate": i.due_date,
                            "overdue": i.overdue,
                            "updatedOn": i.updated_on,
                        })
                    })
                    .collect();
                (total, items)
            } else {
                let rm = svc.redmine()?;
                let raw = rm
                    .list_issues(&IssueQuery {
                        project: Some(pid.to_string()),
                        status: if status == "closed" { "closed".into() } else { "*".into() },
                        max_results: 300,
                        ..Default::default()
                    })
                    .await?;
                let today = chrono::Local::now().date_naive();
                let filtered: Vec<_> = raw
                    .iter()
                    .map(|is| crate::project::to_project_issue(is, today))
                    .filter(|i| !unassigned_only || i.assignee_id.is_none())
                    .filter(|i| !overdue_only || i.overdue)
                    .filter(|i| match &assignee_q {
                        Some(q) => i.assignee.to_lowercase().contains(q),
                        None => true,
                    })
                    .filter(|i| match &keyword {
                        Some(k) => format!("#{} {} {} {} {}", i.id, i.subject, i.description, i.author, i.assignee)
                            .to_lowercase()
                            .contains(k),
                        None => true,
                    })
                    .collect();
                let total = filtered.len();
                let items: Vec<Value> = filtered
                    .into_iter()
                    .take(limit)
                    .map(|i| {
                        json!({
                            "id": i.id,
                            "subject": svc.mask_for_ai(&i.subject),
                            "tracker": i.tracker,
                            "status": i.status,
                            "priority": i.priority,
                            "author": i.author,
                            "assignee": i.assignee,
                            "assigneeId": i.assignee_id,
                            "dueDate": i.due_date,
                            "overdue": i.overdue,
                            "updatedOn": i.updated_on,
                        })
                    })
                    .collect();
                (total, items)
            };

            Ok(ToolOutcome {
                summary: format!("找到 {total} 个问题（返回 {} 条）", issues_json.len()),
                payload: json!({
                    "projectId": pid,
                    "statusScope": status,
                    "totalMatched": total,
                    "returned": issues_json.len(),
                    "issues": issues_json,
                }),
                ok: true,
                project_ids: vec![pid],
            })
        }
        "list_my_issues" => {
            let pid = get_u64(args, "project_id");
            if let Some(p) = pid {
                if is_project_excluded(svc, p) {
                    return Err(anyhow!("该项目已在设置中标记为「不发送给 AI」"));
                }
            }
            let excluded: HashSet<u64> = svc.config().ai.excluded_project_ids.into_iter().collect();
            let f = IssueFilter {
                include_done: args.get("include_done").and_then(Value::as_bool).unwrap_or(false),
                category: get_str(args, "category").map(str::to_string),
                project_id: pid,
                keyword: get_str(args, "keyword").map(str::to_string),
                unread_only: args.get("unread_only").and_then(Value::as_bool).unwrap_or(false),
                limit: Some(get_u64(args, "limit").unwrap_or(30).clamp(1, 100) as usize),
            };
            let list = svc.list_issues(&f)?;
            let mut skipped = 0usize;
            let items: Vec<Value> = list
                .iter()
                .filter(|v| {
                    if excluded.contains(&v.project_id) {
                        skipped += 1;
                        false
                    } else {
                        true
                    }
                })
                .map(|v| {
                    json!({
                        "id": v.id,
                        "projectId": v.project_id,
                        "project": v.project_name,
                        "subject": svc.mask_for_ai(&v.subject),
                        "status": v.status_name,
                        "priority": v.priority_name,
                        "author": v.author_name,
                        "assignedTo": v.assigned_to_name,
                        "dueDate": v.due_date,
                        "overdue": v.overdue,
                        "unread": v.unread,
                        "category": v.analysis.category,
                        "urgency": v.analysis.urgency,
                        "summary": v.analysis.summary.as_deref().map(|s| svc.mask_for_ai(s)),
                        "updatedOn": v.updated_on,
                    })
                })
                .collect();
            Ok(ToolOutcome {
                summary: format!("返回 {} 个待办", items.len()),
                payload: json!({ "count": items.len(), "excludedCount": skipped, "issues": items }),
                ok: true,
                project_ids: pid.into_iter().collect(),
            })
        }
        "get_issue" => {
            let id = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(true);
            let d = svc.issue_detail(id, refresh).await?;
            if is_project_excluded(svc, d.issue.project.id) {
                return Err(anyhow!("#{id} 所属项目已在设置中标记为「不发送给 AI」"));
            }
            let journals: Vec<Value> = d
                .issue
                .journals
                .iter()
                .filter(|j| j.notes.as_deref().is_some_and(|n| !n.trim().is_empty()))
                .rev()
                .take(10)
                .map(|j| {
                    json!({
                        "user": j.user.as_ref().map(|u| &u.name),
                        "createdOn": j.created_on,
                        "private": j.private_notes,
                        "notes": svc.mask_for_ai(&truncate(j.notes.as_deref().unwrap_or(""), 400)),
                    })
                })
                .collect();
            Ok(ToolOutcome {
                summary: format!("#{id} {}", truncate(&d.issue.subject, 30)),
                payload: json!({
                    "id": d.issue.id,
                    "projectId": d.issue.project.id,
                    "project": d.issue.project.name,
                    "subject": svc.mask_for_ai(&d.issue.subject),
                    "description": svc.mask_for_ai(&truncate(d.issue.description.as_deref().unwrap_or(""), 1500)),
                    "status": d.issue.status,
                    "priority": d.issue.priority.as_ref().map(|p| &p.name),
                    "author": d.issue.author.as_ref().map(|a| &a.name),
                    "assignedTo": d.issue.assigned_to.as_ref().map(|a| json!({ "id": a.id, "name": a.name })),
                    "dueDate": d.issue.due_date,
                    "doneRatio": d.issue.done_ratio,
                    "allowedStatuses": d.issue.allowed_statuses,
                    "children": d.children,
                    "recentJournals": journals,
                }),
                ok: true,
                project_ids: vec![d.issue.project.id],
            })
        }
        "list_statuses" => {
            let statuses = svc.statuses().await?;
            let wf = svc.config().workflow;
            Ok(ToolOutcome {
                summary: format!("共 {} 个状态", statuses.len()),
                payload: json!({
                    "statuses": statuses,
                    "defaults": {
                        "resolvedStatusId": wf.resolved_status_id,
                        "inProgressStatusId": wf.in_progress_status_id,
                        "feedbackStatusId": wf.feedback_status_id,
                    }
                }),
                ok: true,
                project_ids: Vec::new(),
            })
        }
        "list_project_members" => {
            let pid = get_u64(args, "project_id").ok_or_else(|| anyhow!("缺少 project_id"))?;
            let members = svc.assignees(pid).await?;
            Ok(ToolOutcome {
                summary: format!("共 {} 位成员", members.len()),
                payload: json!({ "projectId": pid, "members": members }),
                ok: true,
                project_ids: vec![pid],
            })
        }
        "get_dispatch_queue" => {
            let pid = get_u64(args, "project_id");
            if let Some(p) = pid {
                if is_project_excluded(svc, p) {
                    return Err(anyhow!("该项目已在设置中标记为「不发送给 AI」"));
                }
            }
            let excluded: HashSet<u64> = svc.config().ai.excluded_project_ids.into_iter().collect();
            let mut q = svc.dispatch_queue().await?;
            if let Some(p) = pid {
                q.items.retain(|i| i.issue.project_id == p);
                q.projects.retain(|x| x.id == p);
            }
            q.items.retain(|i| !excluded.contains(&i.issue.project_id));
            let items: Vec<Value> = q
                .items
                .iter()
                .take(40)
                .map(|i| {
                    json!({
                        "id": i.issue.id,
                        "projectId": i.issue.project_id,
                        "project": i.issue.project_name,
                        "subject": svc.mask_for_ai(&i.issue.subject),
                        "priority": i.issue.priority_name,
                        "author": i.issue.author_name,
                        "assignedTo": i.issue.assigned_to_name,
                        "suggestion": i.suggestion.as_ref().map(|s| json!({
                            "assigneeId": s.assignee_id, "assigneeName": s.assignee_name, "reason": s.reason
                        })),
                    })
                })
                .collect();
            Ok(ToolOutcome {
                summary: format!("{} 个待分派任务", q.items.len()),
                payload: json!({ "total": q.items.len(), "items": items }),
                ok: true,
                project_ids: pid.into_iter().collect(),
            })
        }
        "get_overview_stats" => {
            let stats = svc.stats()?;
            Ok(ToolOutcome {
                summary: format!("待办 {}，逾期 {}", stats.open_total, stats.overdue),
                payload: serde_json::to_value(&stats)?,
                ok: true,
                project_ids: Vec::new(),
            })
        }
        "sync_issues" => {
            let report = svc.sync().await?;
            Ok(ToolOutcome {
                summary: format!("共 {} 个待办（新增 {}）", report.total, report.new_ids.len()),
                payload: serde_json::to_value(&report)?,
                ok: true,
                project_ids: Vec::new(),
            })
        }
        _ => Err(anyhow!("未知工具：{name}")),
    }
}

// ---------------- 待确认写操作预览构造 ----------------

async fn issue_brief_label(svc: &Service, id: u64) -> (String, Option<u64>) {
    if let Ok(Some(v)) = svc.db.get_view(id) {
        return (format!("#{id} {}（{}）", truncate(&v.subject, 40), v.project_name), Some(v.project_id));
    }
    if let Ok(rm) = svc.redmine() {
        if let Ok(is) = rm.get_issue(id).await {
            return (format!("#{id} {}（{}）", truncate(&is.subject, 40), is.project.name), Some(is.project.id));
        }
    }
    (format!("#{id}"), None)
}

async fn status_name_of(svc: &Service, id: u64) -> String {
    svc.statuses()
        .await
        .ok()
        .and_then(|xs| xs.into_iter().find(|s| s.id == id).map(|s| s.name))
        .unwrap_or_else(|| format!("状态 #{id}"))
}

async fn build_pending_action(svc: &Arc<Service>, call: &ToolCall, args: &Value) -> Result<PendingAction> {
    let id = if call.id.trim().is_empty() {
        uuid::Uuid::new_v4().simple().to_string()
    } else {
        call.id.clone()
    };
    let name = call.function.name.as_str();
    let mut details = Vec::new();
    let mut project_ids = Vec::new();

    let title = match name {
        "close_issues" => {
            let ids = get_u64_list(args, "issue_ids");
            if ids.is_empty() {
                return Err(anyhow!("未指定要关闭的任务号"));
            }
            let sid = match get_u64(args, "status_id") {
                Some(s) => s,
                None => svc.default_closed_status_id().await?,
            };
            let sname = status_name_of(svc, sid).await;
            for &iid in ids.iter().take(12) {
                let (label, pid) = issue_brief_label(svc, iid).await;
                if let Some(p) = pid {
                    project_ids.push(p);
                }
                details.push(label);
            }
            if ids.len() > 12 {
                details.push(format!("…以及其余 {} 个问题", ids.len() - 12));
            }
            details.push(format!("目标状态：{sname}"));
            if let Some(notes) = get_str(args, "notes") {
                details.push(format!("备注：{}", truncate(notes, 100)));
            }
            format!("关闭 {} 个问题", ids.len())
        }
        "update_issues_status" => {
            let ids = get_u64_list(args, "issue_ids");
            let sid = get_u64(args, "status_id").ok_or_else(|| anyhow!("缺少 status_id"))?;
            if ids.is_empty() {
                return Err(anyhow!("未指定要修改的任务号"));
            }
            let sname = status_name_of(svc, sid).await;
            for &iid in ids.iter().take(12) {
                let (label, pid) = issue_brief_label(svc, iid).await;
                if let Some(p) = pid {
                    project_ids.push(p);
                }
                details.push(label);
            }
            if ids.len() > 12 {
                details.push(format!("…以及其余 {} 个问题", ids.len() - 12));
            }
            details.push(format!("目标状态：{sname}"));
            if let Some(notes) = get_str(args, "notes") {
                details.push(format!("备注：{}", truncate(notes, 100)));
            }
            format!("将 {} 个问题改为「{sname}」", ids.len())
        }
        "resolve_issue" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let notes = get_str(args, "notes").ok_or_else(|| anyhow!("缺少处置说明 notes"))?;
            let (label, pid) = issue_brief_label(svc, iid).await;
            if let Some(p) = pid {
                project_ids.push(p);
            }
            details.push(label);
            if let Some(sid) = get_u64(args, "status_id").or(svc.config().workflow.resolved_status_id) {
                details.push(format!("目标状态：{}", status_name_of(svc, sid).await));
            }
            if let Some(uid) = get_u64(args, "assign_to_id") {
                let who = resolve_member_name(svc, pid, uid).await;
                details.push(format!("指派给：{who}"));
            } else if args.get("assign_to_author").and_then(Value::as_bool).unwrap_or(true) {
                details.push("指派给：提出人".into());
            }
            details.push(format!("处置说明：{}", truncate(notes, 120)));
            format!("处理完成并回派 #{iid}")
        }
        "assign_issues" => {
            let ids = get_u64_list(args, "issue_ids");
            let uid = get_u64(args, "assignee_id").ok_or_else(|| anyhow!("缺少 assignee_id"))?;
            if ids.is_empty() {
                return Err(anyhow!("未指定要指派的任务号"));
            }
            let mut first_pid = None;
            for &iid in ids.iter().take(12) {
                let (label, pid) = issue_brief_label(svc, iid).await;
                if let Some(p) = pid {
                    first_pid.get_or_insert(p);
                    project_ids.push(p);
                }
                details.push(label);
            }
            if ids.len() > 12 {
                details.push(format!("…以及其余 {} 个问题", ids.len() - 12));
            }
            let who = resolve_member_name(svc, first_pid, uid).await;
            details.push(format!("指派给：{who}"));
            if let Some(notes) = get_str(args, "notes") {
                details.push(format!("说明：{}", truncate(notes, 100)));
            }
            format!("指派 {} 个问题给 {who}", ids.len())
        }
        "add_note" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let notes = get_str(args, "notes").ok_or_else(|| anyhow!("备注内容不能为空"))?;
            let private = args.get("private").and_then(Value::as_bool).unwrap_or(false);
            let (label, pid) = issue_brief_label(svc, iid).await;
            if let Some(p) = pid {
                project_ids.push(p);
            }
            details.push(label);
            details.push(format!("{}：{}", if private { "私有备注" } else { "备注" }, truncate(notes, 120)));
            format!("给 #{iid} 添加备注")
        }
        "request_more_info" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let question = get_str(args, "question").ok_or_else(|| anyhow!("追问内容不能为空"))?;
            let (label, pid) = issue_brief_label(svc, iid).await;
            if let Some(p) = pid {
                project_ids.push(p);
            }
            details.push(label);
            details.push(format!("追问内容：{}", truncate(question, 120)));
            format!("向 #{iid} 提出人追问")
        }
        "create_issue" => {
            let pid = get_u64(args, "project_id").ok_or_else(|| anyhow!("缺少 project_id"))?;
            let subject = get_str(args, "subject").ok_or_else(|| anyhow!("缺少标题 subject"))?;
            project_ids.push(pid);
            let pname = svc
                .my_projects()
                .await
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == pid).map(|p| p.name))
                .unwrap_or_else(|| format!("项目 #{pid}"));
            details.push(format!("所属项目：{pname}"));
            details.push(format!("标题：{subject}"));
            if let Some(uid) = get_u64(args, "assigned_to_id") {
                details.push(format!("负责人：{}", resolve_member_name(svc, Some(pid), uid).await));
            }
            if let Some(due) = get_str(args, "due_date") {
                details.push(format!("截止日期：{due}"));
            }
            if let Some(desc) = get_str(args, "description") {
                details.push(format!("描述：{}", truncate(desc, 120)));
            }
            format!("在「{pname}」新建问题")
        }
        "link_issues" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let others = get_u64_list(args, "related_issue_ids");
            if others.is_empty() {
                return Err(anyhow!("缺少关联任务号 related_issue_ids"));
            }
            let rt = get_str(args, "relation_type").unwrap_or("relates");
            let (label, pid) = issue_brief_label(svc, iid).await;
            if let Some(p) = pid {
                project_ids.push(p);
            }
            details.push(label);
            let targets: Vec<String> = others.iter().map(|x| format!("#{x}")).collect();
            details.push(format!("建立 {rt} 关联 → {}", targets.join("、")));
            format!("为 #{iid} 建立 {} 条关联", others.len())
        }
        _ => return Err(anyhow!("未知写操作：{name}")),
    };

    Ok(PendingAction {
        id,
        tool: name.to_string(),
        title,
        details,
        args: args.clone(),
        project_ids: dedup_u64(project_ids),
    })
}

async fn resolve_member_name(svc: &Service, project_id: Option<u64>, uid: u64) -> String {
    if let Some(pid) = project_id {
        if let Ok(members) = svc.assignees(pid).await {
            if let Some(m) = members.into_iter().find(|m| m.id == uid) {
                return m.name;
            }
        }
    }
    format!("成员 #{uid}")
}

// ---------------- 写工具实际执行 ----------------

async fn execute_write_tool(svc: &Arc<Service>, name: &str, args: &Value) -> Result<ToolOutcome> {
    match name {
        "close_issues" => {
            let ids = get_u64_list(args, "issue_ids");
            if ids.is_empty() {
                return Err(anyhow!("未指定要关闭的任务号"));
            }
            let sid = get_u64(args, "status_id");
            let notes = get_str(args, "notes").map(str::to_string);
            let done_ratio = get_u64(args, "done_ratio").map(|v| v.min(100) as u32);
            let mut ok_ids = Vec::new();
            let mut errs = Vec::new();
            let mut pids = Vec::new();
            for iid in ids {
                if let Ok(Some(v)) = svc.db.get_view(iid) {
                    pids.push(v.project_id);
                }
                match svc.close_issue(iid, sid, notes.clone(), done_ratio).await {
                    Ok(out) => ok_ids.push(json!({ "issueId": iid, "status": out.status })),
                    Err(e) => errs.push(format!("#{iid}: {e:#}")),
                }
            }
            let summary = if errs.is_empty() {
                format!("已关闭 {} 个问题", ok_ids.len())
            } else {
                format!("成功关闭 {} 个，{} 个失败（{}）", ok_ids.len(), errs.len(), errs.join("；"))
            };
            Ok(ToolOutcome {
                ok: errs.is_empty(),
                summary,
                payload: json!({ "closed": ok_ids, "errors": errs }),
                project_ids: dedup_u64(pids),
            })
        }
        "update_issues_status" => {
            let ids = get_u64_list(args, "issue_ids");
            let sid = get_u64(args, "status_id").ok_or_else(|| anyhow!("缺少 status_id"))?;
            if ids.is_empty() {
                return Err(anyhow!("未指定要修改的任务号"));
            }
            let notes = get_str(args, "notes").map(str::to_string);
            let mut ok_list = Vec::new();
            let mut errs = Vec::new();
            let mut pids = Vec::new();
            for iid in ids {
                if let Ok(Some(v)) = svc.db.get_view(iid) {
                    pids.push(v.project_id);
                }
                match svc.change_status(iid, sid, notes.clone()).await {
                    Ok(out) => ok_list.push(json!({ "issueId": iid, "status": out.status })),
                    Err(e) => errs.push(format!("#{iid}: {e:#}")),
                }
            }
            let sname = status_name_of(svc, sid).await;
            let summary = if errs.is_empty() {
                format!("已将 {} 个问题改为「{sname}」", ok_list.len())
            } else {
                format!("成功 {} 个，失败 {} 个（{}）", ok_list.len(), errs.len(), errs.join("；"))
            };
            Ok(ToolOutcome {
                ok: errs.is_empty(),
                summary,
                payload: json!({ "updated": ok_list, "errors": errs }),
                project_ids: dedup_u64(pids),
            })
        }
        "resolve_issue" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let notes = get_str(args, "notes").ok_or_else(|| anyhow!("缺少处置说明 notes"))?.to_string();
            let pid = svc.db.get_view(iid).ok().flatten().map(|v| v.project_id);
            let req = ResolveRequest {
                issue_id: iid,
                notes,
                status_id: get_u64(args, "status_id"),
                assign_to_author: args.get("assign_to_author").and_then(Value::as_bool).unwrap_or(true),
                assign_to_id: get_u64(args, "assign_to_id"),
                done_ratio: None,
                spent_hours: args.get("spent_hours").and_then(Value::as_f64),
                activity_id: None,
                private_notes: false,
                attachments: Vec::new(),
            };
            let out = svc.resolve(&req, "chat").await?;
            svc.invalidate_overviews();
            Ok(ToolOutcome {
                ok: out.ok,
                summary: format!(
                    "#{iid} 已处理（状态：{}，指派：{}）",
                    out.status.as_deref().unwrap_or("—"),
                    out.assigned_to.as_deref().unwrap_or("—")
                ),
                payload: serde_json::to_value(&out)?,
                project_ids: pid.into_iter().collect(),
            })
        }
        "assign_issues" => {
            let ids = get_u64_list(args, "issue_ids");
            let uid = get_u64(args, "assignee_id").ok_or_else(|| anyhow!("缺少 assignee_id"))?;
            if ids.is_empty() {
                return Err(anyhow!("未指定任务号"));
            }
            let notes = get_str(args, "notes").map(str::to_string);
            let pids: Vec<u64> = ids
                .iter()
                .filter_map(|&iid| svc.db.get_view(iid).ok().flatten().map(|v| v.project_id))
                .collect();
            let items: Vec<AssignItem> = ids.into_iter().map(|issue_id| AssignItem { issue_id, assignee_id: uid }).collect();
            let outs = svc.bulk_assign(&items, notes).await;
            svc.invalidate_overviews();
            let ok_cnt = outs.iter().filter(|o| o.ok).count();
            let errs: Vec<String> = outs
                .iter()
                .filter(|o| !o.ok)
                .map(|o| format!("#{}: {}", o.issue_id, o.error.as_deref().unwrap_or("失败")))
                .collect();
            let summary = if errs.is_empty() {
                format!("已指派 {ok_cnt} 个问题")
            } else {
                format!("成功 {ok_cnt} 个，失败 {} 个（{}）", errs.len(), errs.join("；"))
            };
            Ok(ToolOutcome {
                ok: errs.is_empty(),
                summary,
                payload: serde_json::to_value(&outs)?,
                project_ids: dedup_u64(pids),
            })
        }
        "add_note" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let notes = get_str(args, "notes").ok_or_else(|| anyhow!("备注内容不能为空"))?;
            let private = args.get("private").and_then(Value::as_bool).unwrap_or(false);
            let pid = svc.db.get_view(iid).ok().flatten().map(|v| v.project_id);
            svc.add_note(iid, notes, private).await?;
            Ok(ToolOutcome {
                ok: true,
                summary: format!("已给 #{iid} 添加备注"),
                payload: json!({ "ok": true, "issueId": iid }),
                project_ids: pid.into_iter().collect(),
            })
        }
        "request_more_info" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let question = get_str(args, "question").ok_or_else(|| anyhow!("追问内容不能为空"))?;
            let pid = svc.db.get_view(iid).ok().flatten().map(|v| v.project_id);
            let out = svc.request_more_info(iid, question).await?;
            svc.invalidate_overviews();
            Ok(ToolOutcome {
                ok: out.ok,
                summary: format!("已向 #{iid} 提出人发送追问"),
                payload: serde_json::to_value(&out)?,
                project_ids: pid.into_iter().collect(),
            })
        }
        "create_issue" => {
            let pid = get_u64(args, "project_id").ok_or_else(|| anyhow!("缺少 project_id"))?;
            let subject = get_str(args, "subject").ok_or_else(|| anyhow!("缺少标题 subject"))?.to_string();
            let created = svc
                .create_issue(&NewIssue {
                    project_id: pid,
                    subject,
                    description: get_str(args, "description").unwrap_or("").to_string(),
                    assigned_to_id: get_u64(args, "assigned_to_id"),
                    due_date: get_str(args, "due_date").map(str::to_string),
                    parent_issue_id: get_u64(args, "parent_issue_id"),
                    ..Default::default()
                })
                .await?;
            Ok(ToolOutcome {
                ok: true,
                summary: format!("已新建 #{} {}", created.id, truncate(&created.subject, 30)),
                payload: json!({ "ok": true, "id": created.id, "subject": created.subject, "projectId": created.project.id }),
                project_ids: vec![pid],
            })
        }
        "link_issues" => {
            let iid = get_u64(args, "issue_id").ok_or_else(|| anyhow!("缺少 issue_id"))?;
            let others = get_u64_list(args, "related_issue_ids");
            if others.is_empty() {
                return Err(anyhow!("缺少 related_issue_ids"));
            }
            let rt = get_str(args, "relation_type").unwrap_or("relates");
            let pid = svc.db.get_view(iid).ok().flatten().map(|v| v.project_id);
            let errors = svc.link_issues(iid, &others, rt).await?;
            let summary = if errors.is_empty() {
                format!("已为 #{iid} 建立 {} 条关联", others.len())
            } else {
                format!("部分关联失败：{}", errors.join("；"))
            };
            Ok(ToolOutcome {
                ok: errors.is_empty(),
                summary,
                payload: json!({ "ok": errors.is_empty(), "errors": errors }),
                project_ids: pid.into_iter().collect(),
            })
        }
        _ => Err(anyhow!("未知写操作：{name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issue_ids_flexibly() {
        let args = json!({ "issue_ids": [101, "#102", "103", 101, 0, "abc"] });
        assert_eq!(get_u64_list(&args, "issue_ids"), vec![101, 102, 103]);
        assert_eq!(get_u64(&json!({ "issue_id": "#456" }), "issue_id"), Some(456));
    }

    #[test]
    fn distinguishes_read_and_write_tools() {
        let defs = tool_definitions();
        assert!(defs.len() >= 15);
        assert!(is_write_tool("close_issues"));
        assert!(is_write_tool("create_issue"));
        assert!(!is_write_tool("get_project_issues"));
        assert!(!is_write_tool("list_projects"));
    }
}
