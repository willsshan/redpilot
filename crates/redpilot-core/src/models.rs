//! Redmine REST API 数据结构，以及给界面/MCP 使用的视图结构。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamedId {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "is_closed")]
    pub is_closed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: u64,
    pub project: NamedId,
    #[serde(default)]
    pub tracker: Option<NamedId>,
    pub status: NamedId,
    #[serde(default)]
    pub priority: Option<NamedId>,
    #[serde(default)]
    pub author: Option<NamedId>,
    #[serde(default, alias = "assigned_to")]
    pub assigned_to: Option<NamedId>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "start_date")]
    pub start_date: Option<String>,
    #[serde(default, alias = "due_date")]
    pub due_date: Option<String>,
    #[serde(default, alias = "done_ratio")]
    pub done_ratio: Option<u32>,
    #[serde(default, alias = "estimated_hours")]
    pub estimated_hours: Option<f64>,
    #[serde(default, alias = "created_on")]
    pub created_on: String,
    #[serde(default, alias = "updated_on")]
    pub updated_on: String,
    #[serde(default, alias = "closed_on")]
    pub closed_on: Option<String>,
    #[serde(default)]
    pub journals: Vec<Journal>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub relations: Vec<Relation>,
    #[serde(default, alias = "allowed_statuses")]
    pub allowed_statuses: Vec<NamedId>,
    #[serde(default)]
    pub watchers: Vec<NamedId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub id: u64,
    #[serde(default)]
    pub user: Option<NamedId>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default, alias = "created_on")]
    pub created_on: String,
    #[serde(default, alias = "private_notes")]
    pub private_notes: Option<bool>,
    #[serde(default)]
    pub details: Vec<JournalDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalDetail {
    #[serde(default)]
    pub property: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, alias = "old_value")]
    pub old_value: Option<String>,
    #[serde(default, alias = "new_value")]
    pub new_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: u64,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub filesize: u64,
    #[serde(default, alias = "content_type")]
    pub content_type: Option<String>,
    #[serde(default, alias = "content_url")]
    pub content_url: String,
    #[serde(default)]
    pub author: Option<NamedId>,
    #[serde(default, alias = "created_on")]
    pub created_on: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub id: u64,
    #[serde(alias = "issue_id")]
    pub issue_id: u64,
    #[serde(alias = "issue_to_id")]
    pub issue_to_id: u64,
    #[serde(alias = "relation_type")]
    pub relation_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: u64,
    #[serde(default)]
    pub login: String,
    #[serde(default, alias = "firstname")]
    pub firstname: String,
    #[serde(default, alias = "lastname")]
    pub lastname: String,
    #[serde(default)]
    pub mail: Option<String>,
}

impl User {
    pub fn display_name(&self) -> String {
        let full = format!("{}{}", self.lastname, self.firstname);
        if full.trim().is_empty() {
            self.login.clone()
        } else {
            full
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub identifier: String,
}

// ---------------- 视图结构 ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IssueView {
    pub id: u64,
    pub project_id: u64,
    pub project_name: String,
    pub tracker_name: String,
    pub status_id: u64,
    pub status_name: String,
    pub priority_id: u64,
    pub priority_name: String,
    pub author_id: Option<u64>,
    pub author_name: String,
    pub assigned_to_id: Option<u64>,
    pub assigned_to_name: String,
    pub subject: String,
    pub description: String,
    pub done_ratio: u32,
    pub due_date: Option<String>,
    pub created_on: String,
    pub updated_on: String,
    pub in_inbox: bool,
    pub unread: bool,
    pub overdue: bool,
    /// 上次查看之后的最新动态（如“张三 回复：…”），查看后清除
    pub change_hint: Option<String>,
    pub analysis: Analysis,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    /// 生效分类：人工 > AI > 规则
    pub category: Option<String>,
    /// manual | ai | rule
    pub category_source: Option<String>,
    pub rule_category: Option<String>,
    pub module: Option<String>,
    pub urgency: Option<String>,
    pub difficulty: Option<u8>,
    pub est_hours: Option<f64>,
    pub summary: Option<String>,
    pub missing_info: Vec<String>,
    pub follow_up: Option<String>,
    pub suggestion: Option<String>,
    pub ai_analyzed: bool,
    /// 任务在 AI 分析之后又有更新
    pub ai_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub view: IssueView,
    pub issue: Issue,
    pub similar: Vec<SimilarIssue>,
    pub web_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarIssue {
    pub id: u64,
    pub subject: String,
    pub status_name: String,
    pub score: f64,
    /// 本工具记录过的处置说明（历史知识库）
    pub resolution_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct IssueFilter {
    /// 默认只看收件箱（当前仍需我处理的任务）
    pub include_done: bool,
    pub category: Option<String>,
    pub project_id: Option<u64>,
    pub keyword: Option<String>,
    pub unread_only: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub total: usize,
    pub new_ids: Vec<u64>,
    pub updated_ids: Vec<u64>,
    pub removed: usize,
    pub finished_at: String,
    /// 他人在 Redmine 上做的变更：回复、调整优先级/截止日期/状态、转回给我
    pub changes: Vec<IssueChange>,
    /// 今天或明天到期、且今天还没提醒过的任务
    pub due_soon: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IssueChange {
    pub issue_id: u64,
    pub subject: String,
    /// reply | priority | due | status | assigned
    pub kind: String,
    pub actor: String,
    /// 变更人是否为提出人
    pub by_author: bool,
    pub summary: String,
    /// 是否值得打扰（提出人回复、转回给我、调为高优先级）
    pub important: bool,
    pub created_on: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeReport {
    pub analyzed: Vec<u64>,
    pub skipped: Vec<u64>,
    pub excluded: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: i64,
    pub created_at: String,
    pub issue_ids: Vec<u64>,
    pub groups: Vec<PlanGroup>,
    pub order: Vec<u64>,
    pub duplicates: Vec<PlanDuplicate>,
    pub daily_plan: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanGroup {
    pub title: String,
    pub issue_ids: Vec<u64>,
    pub reason: String,
    pub approach: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanDuplicate {
    pub issue_ids: Vec<u64>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveRequest {
    pub issue_id: u64,
    /// 处置说明，会写入 Redmine 备注
    pub notes: String,
    /// 目标状态，留空使用配置里的“处理完成状态”
    #[serde(default)]
    pub status_id: Option<u64>,
    /// 指派回提出人（默认 true）
    #[serde(default = "default_true")]
    pub assign_to_author: bool,
    #[serde(default)]
    pub done_ratio: Option<u32>,
    /// 同时登记工时（小时）
    #[serde(default)]
    pub spent_hours: Option<f64>,
    #[serde(default)]
    pub activity_id: Option<u64>,
    #[serde(default)]
    pub private_notes: bool,
    /// 要上传的附件（本地文件路径），每个任务单独上传一份
    #[serde(default)]
    pub attachments: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResolveOutcome {
    pub issue_id: u64,
    pub ok: bool,
    pub assigned_to: Option<String>,
    pub status: Option<String>,
    /// 已上传的附件文件名
    pub attached: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// 已暂存、等待随回派上传的附件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    pub path: String,
    pub filename: String,
    pub size: u64,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub open_total: usize,
    pub unread: usize,
    pub overdue: usize,
    pub high_priority: usize,
    pub unanalyzed: usize,
    pub by_category: Vec<(String, usize)>,
    pub by_project: Vec<(String, usize)>,
    pub by_priority: Vec<(String, usize)>,
    pub resolved_7d: usize,
    pub resolved_30d: usize,
    pub spent_hours_7d: f64,
    /// 最近 14 天每天回派数量（日期, 数量）
    pub daily_resolved: Vec<(String, usize)>,
    pub ai_tokens_30d: u64,
    pub ai_prompt_tokens_30d: u64,
    pub ai_completion_tokens_30d: u64,
    /// 按设置中的单价估算的 30 天 AI 费用
    pub ai_cost_30d: f64,
    pub ai_currency: String,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionRecord {
    pub issue_id: u64,
    pub subject: String,
    pub notes: String,
    pub status_name: String,
    pub assigned_to_name: String,
    pub spent_hours: Option<f64>,
    pub created_at: String,
    pub via: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端只传需要的字段，其余字段必须有默认值，否则整个请求会失败
    #[test]
    fn filter_accepts_partial_json_from_frontend() {
        for v in [
            serde_json::json!({}),
            serde_json::json!({ "keyword": null }),
            serde_json::json!({ "includeDone": true }),
        ] {
            let f: IssueFilter = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v} 解析失败: {e}"));
            assert!(f.keyword.is_none() && !f.unread_only);
        }
    }
}
