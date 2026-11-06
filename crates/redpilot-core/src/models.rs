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
    /// Redmine 的“类别”
    #[serde(default)]
    pub category: Option<NamedId>,
    /// 目标版本（里程碑）
    #[serde(default, alias = "fixed_version")]
    pub fixed_version: Option<NamedId>,
    #[serde(default, alias = "custom_fields")]
    pub custom_fields: Vec<CustomFieldValue>,
    /// 子任务（`include=children`，只有 ID、跟踪和标题）
    #[serde(default)]
    pub children: Vec<ChildRef>,
    /// 父任务（列表接口也会返回）
    #[serde(default)]
    pub parent: Option<ParentRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentRef {
    pub id: u64,
}

/// 自定义字段的值；拆分子任务时沿用父任务的值，避免必填字段校验失败
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldValue {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRef {
    pub id: u64,
    #[serde(default)]
    pub tracker: Option<NamedId>,
    #[serde(default)]
    pub subject: String,
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
    /// Redmine 上的类别（区别于本地的分类 `analysis.category`）
    pub redmine_category_id: Option<u64>,
    pub redmine_category: Option<String>,
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
    /// 子任务的当前状态与负责人
    pub children: Vec<ChildIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChildIssue {
    pub id: u64,
    pub subject: String,
    pub tracker: String,
    pub status: String,
    pub closed: bool,
    pub assignee: String,
    pub done_ratio: u32,
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
    /// 指派回提出人（默认 true）；设置了 assign_to_id 时以 assign_to_id 为准
    #[serde(default = "default_true")]
    pub assign_to_author: bool,
    /// 指派给指定的项目成员（用户或组 ID）
    #[serde(default)]
    pub assign_to_id: Option<u64>,
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

/// 我参与的项目及我在其中的角色
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAccess {
    pub id: u64,
    pub name: String,
    pub roles: Vec<String>,
    /// 担任管理类角色（或是系统管理员）
    pub is_manager: bool,
}

/// 项目视角下的任务（不限于指派给我的）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIssue {
    pub id: u64,
    pub project_name: String,
    pub subject: String,
    pub description: String,
    pub tracker: String,
    pub status: String,
    pub priority_id: u64,
    pub priority: String,
    pub author_id: Option<u64>,
    pub author: String,
    pub assignee_id: Option<u64>,
    pub assignee: String,
    pub category_id: Option<u64>,
    pub category: Option<String>,
    pub version_id: Option<u64>,
    pub version: Option<String>,
    pub done_ratio: u32,
    pub due_date: Option<String>,
    pub created_on: String,
    pub updated_on: String,
    /// 父任务 ID
    pub parent_id: Option<u64>,
    pub overdue: bool,
    /// 距上次更新的天数
    pub stale_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionProgress {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub due_date: Option<String>,
    pub open: usize,
    pub closed: usize,
    /// 已过截止日期但还有未关闭任务
    pub overdue: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekStat {
    /// 周一的日期，如 10-27
    pub label: String,
    pub created: usize,
    pub closed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverview {
    pub project_id: u64,
    pub fetched_at: String,
    /// 全部未关闭任务（含子项目）
    pub issues: Vec<ProjectIssue>,
    /// 未关闭任务超过 2000 个时只取了前 2000 个
    pub truncated: bool,
    pub members: Vec<Assignee>,
    pub versions: Vec<VersionProgress>,
    /// 近 8 周新增与关闭数量，从早到晚
    pub weekly: Vec<WeekStat>,
}

/// 转派建议
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignSuggestion {
    pub issue_id: u64,
    pub assignee_id: Option<u64>,
    pub assignee_name: String,
    pub reason: String,
    /// ai | category | history | workload
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignItem {
    pub issue_id: u64,
    pub assignee_id: u64,
}

/// 拆分出的一个子任务。提交时只用到前五个字段，其余字段是推荐结果附带的说明
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SplitPart {
    pub subject: String,
    pub description: String,
    pub assignee_id: Option<u64>,
    /// 为空时沿用原任务的类别
    pub category_id: Option<u64>,
    pub estimated_hours: Option<f64>,
    pub assignee_name: String,
    pub reason: String,
    /// ai | category | history | workload | role
    pub source: String,
    /// 分工方向（前端、后端……），用于“前端等后端”的先后关系
    pub role: String,
}

/// 拆分建议，同时带上拆分界面需要的成员、类别和负载
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SplitSuggestion {
    pub issue_id: u64,
    pub subject: String,
    pub description: String,
    pub category: Option<String>,
    pub parts: Vec<SplitPart>,
    /// 无法给出建议或需要提醒时的说明
    pub hint: Option<String>,
    pub members: Vec<Assignee>,
    pub categories: Vec<NamedId>,
    /// (成员 ID, 当前负责的未关闭任务数)
    pub load: Vec<(u64, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SplitOutcome {
    pub issue_id: u64,
    pub created: Vec<ChildIssue>,
    /// 未能创建的子任务：“标题：原因”
    pub failed: Vec<String>,
    pub warnings: Vec<String>,
}

/// 可指派的项目成员
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignee {
    pub id: u64,
    pub name: String,
    pub is_group: bool,
    pub roles: Vec<String>,
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
    /// 分派台中等待分派的任务数
    pub dispatch_pending: usize,
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

// ---------------- 分派台 ----------------

/// 分派台里的一个任务，带上推荐的负责人
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchItem {
    pub issue: IssueView,
    /// 单人推荐（有 AI 结果时优先用 AI 的）
    pub suggestion: Option<AssignSuggestion>,
    /// 看起来需要多人分工时的方案
    pub multi: Option<MultiPlan>,
    /// 任务没有截止日期时，按优先级给的默认截止日期
    pub default_due: Option<String>,
    /// 描述里分条列出的事项数
    pub item_count: usize,
}

/// 多人分工方案：每人一个子任务
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MultiPlan {
    pub parts: Vec<SplitPart>,
    pub reason: String,
    /// ai | role | history
    pub source: String,
}

/// 分派时可选的成员
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchMember {
    pub id: u64,
    pub name: String,
    pub is_group: bool,
    pub roles: Vec<String>,
    /// 在这个项目里负责的未关闭任务数
    pub load: usize,
    /// 分工方向（设置里标注的，或按历史推断的）
    pub role: String,
    /// 分工是按历史推断的，还没有在设置里确认
    pub role_guessed: bool,
    /// 数字键 1-9
    pub key: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchProject {
    pub id: u64,
    pub name: String,
    pub members: Vec<DispatchMember>,
    pub categories: Vec<NamedId>,
    /// 读取成员或历史失败时的说明
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DispatchQueue {
    pub items: Vec<DispatchItem>,
    pub projects: Vec<DispatchProject>,
    pub me_id: Option<u64>,
    /// 视为“待分派”的状态名
    pub pending_statuses: Vec<String>,
}

/// 派单请求
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DispatchRequest {
    pub issue_id: u64,
    /// single：指派给一个人；split：拆成子任务分别指派；assist：一人负责，其他人协作（加为关注人）
    pub mode: String,
    /// single / assist 的负责人
    pub assignee_id: Option<u64>,
    /// assist 的协作人
    pub collaborator_ids: Vec<u64>,
    /// split 的子任务
    pub parts: Vec<SplitPart>,
    /// split 时前端子任务等后端子任务完成（建立“阻挡”关联）
    pub frontend_waits: bool,
    /// 派单说明，被指派人会收到通知
    pub notes: Option<String>,
    /// 设置截止日期
    pub due_date: Option<String>,
    /// 没有指定截止日期、任务也没有截止日期时，按优先级设默认截止日期
    pub auto_due: bool,
    /// 把我加为关注人；留空按设置
    pub watch: Option<bool>,
    /// 派单后切换到的状态；留空按设置
    pub status_id: Option<u64>,
    /// 同时修改 Redmine 类别
    pub category_id: Option<u64>,
    /// 推荐的负责人，用于统计推荐命中率
    pub suggested_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DispatchOutcome {
    pub issue_id: u64,
    pub ok: bool,
    pub mode: String,
    /// 被指派的人
    pub assigned_to: Vec<String>,
    /// split 时创建的子任务
    pub created: Vec<ChildIssue>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

/// 设置里“团队分工”的一行：成员及按历史推断的分工
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamCandidate {
    pub id: u64,
    pub name: String,
    pub projects: Vec<String>,
    pub load: usize,
    pub guessed_role: String,
    /// 推断依据，例如“近期 12 个任务中 9 个涉及页面”
    pub guess_reason: String,
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
