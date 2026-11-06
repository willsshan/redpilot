import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------- 类型（与 crates/redpilot-core/src/models.rs 对应） ----------

export interface NamedId {
  id: number;
  name: string;
  isClosed?: boolean;
}

export interface Analysis {
  category?: string | null;
  categorySource?: "manual" | "ai" | "rule" | null;
  ruleCategory?: string | null;
  module?: string | null;
  urgency?: string | null;
  difficulty?: number | null;
  estHours?: number | null;
  summary?: string | null;
  missingInfo: string[];
  followUp?: string | null;
  suggestion?: string | null;
  aiAnalyzed: boolean;
  aiStale: boolean;
}

export interface IssueView {
  id: number;
  projectId: number;
  projectName: string;
  trackerName: string;
  statusId: number;
  statusName: string;
  priorityId: number;
  priorityName: string;
  authorId?: number | null;
  authorName: string;
  assignedToId?: number | null;
  assignedToName: string;
  subject: string;
  description: string;
  doneRatio: number;
  dueDate?: string | null;
  createdOn: string;
  updatedOn: string;
  inInbox: boolean;
  unread: boolean;
  overdue: boolean;
  /** 上次查看之后的最新动态 */
  changeHint?: string | null;
  /** Redmine 上的类别（区别于本地分类 analysis.category） */
  redmineCategoryId?: number | null;
  redmineCategory?: string | null;
  analysis: Analysis;
}

export interface Journal {
  id: number;
  user?: NamedId | null;
  notes?: string | null;
  createdOn: string;
  privateNotes?: boolean | null;
  details: { property: string; name: string; oldValue?: string | null; newValue?: string | null }[];
}

export interface Attachment {
  id: number;
  filename: string;
  filesize: number;
  contentType?: string | null;
  contentUrl: string;
  author?: NamedId | null;
  createdOn: string;
}

export interface Issue {
  id: number;
  project: NamedId;
  tracker?: NamedId | null;
  status: NamedId;
  priority?: NamedId | null;
  author?: NamedId | null;
  assignedTo?: NamedId | null;
  subject: string;
  description?: string | null;
  dueDate?: string | null;
  doneRatio?: number | null;
  createdOn: string;
  updatedOn: string;
  journals: Journal[];
  attachments: Attachment[];
  relations: { id: number; issueId: number; issueToId: number; relationType: string }[];
  allowedStatuses: NamedId[];
  watchers: NamedId[];
}

export interface SimilarIssue {
  id: number;
  subject: string;
  statusName: string;
  score: number;
  resolutionNotes?: string | null;
}

export interface IssueDetail {
  view: IssueView;
  issue: Issue;
  similar: SimilarIssue[];
  webUrl: string;
  children: ChildIssue[];
}

export interface ChildIssue {
  id: number;
  subject: string;
  tracker: string;
  status: string;
  closed: boolean;
  assignee: string;
  doneRatio: number;
}

export interface IssueFilter {
  includeDone?: boolean;
  category?: string | null;
  projectId?: number | null;
  keyword?: string | null;
  unreadOnly?: boolean;
  limit?: number | null;
}

export interface IssueChange {
  issueId: number;
  subject: string;
  kind: "reply" | "priority" | "due" | "status" | "assigned";
  actor: string;
  byAuthor: boolean;
  summary: string;
  important: boolean;
  createdOn: string;
}

export interface SyncReport {
  total: number;
  newIds: number[];
  updatedIds: number[];
  removed: number;
  finishedAt: string;
  changes: IssueChange[];
  dueSoon: number[];
}

export interface AnalyzeReport {
  analyzed: number[];
  skipped: number[];
  excluded: number[];
}

export interface PlanGroup {
  title: string;
  issueIds: number[];
  reason: string;
  approach: string;
}

export interface Plan {
  id: number;
  createdAt: string;
  issueIds: number[];
  groups: PlanGroup[];
  order: number[];
  duplicates: { issueIds: number[]; reason: string }[];
  dailyPlan: string;
}

export interface ResolveRequest {
  issueId: number;
  notes: string;
  statusId?: number | null;
  assignToAuthor: boolean;
  /** 指派给指定成员，优先于 assignToAuthor */
  assignToId?: number | null;
  doneRatio?: number | null;
  spentHours?: number | null;
  activityId?: number | null;
  privateNotes: boolean;
  /** 暂存附件的本地路径 */
  attachments: string[];
}

export interface Assignee {
  id: number;
  name: string;
  isGroup: boolean;
  roles: string[];
}

export interface ProjectAccess {
  id: number;
  name: string;
  roles: string[];
  isManager: boolean;
}

export interface ProjectIssue {
  id: number;
  projectName: string;
  subject: string;
  description: string;
  tracker: string;
  status: string;
  priorityId: number;
  priority: string;
  authorId?: number | null;
  author: string;
  assigneeId?: number | null;
  assignee: string;
  categoryId?: number | null;
  category?: string | null;
  versionId?: number | null;
  version?: string | null;
  doneRatio: number;
  dueDate?: string | null;
  createdOn: string;
  updatedOn: string;
  overdue: boolean;
  staleDays: number;
}

export interface VersionProgress {
  id: number;
  name: string;
  status: string;
  dueDate?: string | null;
  open: number;
  closed: number;
  overdue: boolean;
}

export interface ProjectOverview {
  projectId: number;
  fetchedAt: string;
  issues: ProjectIssue[];
  truncated: boolean;
  members: Assignee[];
  versions: VersionProgress[];
  weekly: { label: string; created: number; closed: number }[];
}

export interface AssignSuggestion {
  issueId: number;
  assigneeId?: number | null;
  assigneeName: string;
  reason: string;
  source: "ai" | "category" | "history" | "workload" | "role";
}

export interface SplitPart {
  subject: string;
  description: string;
  assigneeId?: number | null;
  /** 为空时沿用原任务的类别 */
  categoryId?: number | null;
  estimatedHours?: number | null;
  assigneeName?: string;
  reason?: string;
  source?: AssignSuggestion["source"] | "";
  /** 分工方向（前端、后端……） */
  role?: string;
}

export interface SplitSuggestion {
  issueId: number;
  subject: string;
  description: string;
  category?: string | null;
  parts: SplitPart[];
  hint?: string | null;
  members: Assignee[];
  categories: NamedId[];
  /** [成员 ID, 当前未关闭任务数] */
  load: [number, number][];
}

export interface SplitOutcome {
  issueId: number;
  created: ChildIssue[];
  failed: string[];
  warnings: string[];
}

export interface StagedFile {
  path: string;
  filename: string;
  size: number;
  contentType?: string | null;
}

export interface ResolveOutcome {
  issueId: number;
  ok: boolean;
  assignedTo?: string | null;
  status?: string | null;
  attached: string[];
  warnings: string[];
  error?: string | null;
}

export interface Stats {
  openTotal: number;
  unread: number;
  overdue: number;
  highPriority: number;
  unanalyzed: number;
  byCategory: [string, number][];
  byProject: [string, number][];
  byPriority: [string, number][];
  resolved7d: number;
  resolved30d: number;
  spentHours7d: number;
  dailyResolved: [string, number][];
  aiTokens30d: number;
  aiPromptTokens30d: number;
  aiCompletionTokens30d: number;
  aiCost30d: number;
  aiCurrency: string;
  lastSyncAt?: string | null;
  dispatchPending: number;
}

export interface ResolutionRecord {
  issueId: number;
  subject: string;
  notes: string;
  statusName: string;
  assignedToName: string;
  spentHours?: number | null;
  createdAt: string;
  via: string;
}

export interface User {
  id: number;
  login: string;
  firstname: string;
  lastname: string;
  mail?: string | null;
}

export type TextFormat = "markdown" | "textile";

export interface AppConfig {
  redmine: { baseUrl: string; acceptInvalidCerts: boolean; proxy: string; textFormat: TextFormat };
  sync: {
    intervalMinutes: number;
    assignedToMe: boolean;
    watched: boolean;
    project: string;
    queryId?: number | null;
    notify: boolean;
    notifyChanges: boolean;
    notifyDue: boolean;
    autoAnalyze: boolean;
    managedUnassigned: boolean;
  };
  workflow: {
    resolvedStatusId?: number | null;
    inProgressStatusId?: number | null;
    feedbackStatusId?: number | null;
    setDoneRatio: boolean;
    defaultActivityId?: number | null;
    noteSections: string[];
  };
  ai: {
    enabled: boolean;
    baseUrl: string;
    model: string;
    planningModel: string;
    jsonMode: boolean;
    temperature: number;
    maskSensitive: boolean;
    excludedProjectIds: number[];
    batchSize: number;
    priceInput: number;
    priceOutput: number;
    currency: string;
    customPrompts: { classify: string; plan: string; note: string; report: string };
  };
  mcp: { enabled: boolean; port: number; token: string; allowWrite: boolean };
  rules: { category: string; keywords: string[] }[];
  dispatch: DispatchConfig;
}

export interface TeamMember {
  id: number;
  name: string;
  role: string;
}

export interface DispatchConfig {
  pendingStatusIds: number[];
  projectIds: number[];
  watch: boolean;
  dueDays: { urgent: number; high: number; normal: number; low: number };
  assignedStatusId?: number | null;
  noteTemplate: string;
  team: TeamMember[];
}

// ---------- 分派台 ----------

export interface MultiPlan {
  parts: SplitPart[];
  reason: string;
  source: "ai" | "role" | "history";
}

export interface DispatchItem {
  issue: IssueView;
  suggestion?: AssignSuggestion | null;
  multi?: MultiPlan | null;
  defaultDue?: string | null;
  itemCount: number;
}

export interface DispatchMember {
  id: number;
  name: string;
  isGroup: boolean;
  roles: string[];
  load: number;
  role: string;
  roleGuessed: boolean;
  key?: number | null;
}

export interface DispatchProject {
  id: number;
  name: string;
  members: DispatchMember[];
  categories: NamedId[];
  error?: string | null;
}

export interface DispatchQueue {
  items: DispatchItem[];
  projects: DispatchProject[];
  meId?: number | null;
  pendingStatuses: string[];
}

export interface DispatchRequest {
  issueId: number;
  mode: "single" | "split" | "assist";
  assigneeId?: number | null;
  collaboratorIds?: number[];
  parts?: SplitPart[];
  frontendWaits?: boolean;
  notes?: string | null;
  dueDate?: string | null;
  autoDue?: boolean;
  watch?: boolean | null;
  statusId?: number | null;
  categoryId?: number | null;
  suggestedId?: number | null;
}

export interface DispatchOutcome {
  issueId: number;
  ok: boolean;
  mode: string;
  assignedTo: string[];
  created: ChildIssue[];
  warnings: string[];
  error?: string | null;
}

export interface TeamCandidate {
  id: number;
  name: string;
  projects: string[];
  load: number;
  guessedRole: string;
  guessReason: string;
}

export interface SecretStatus {
  redmine: boolean;
  ai: boolean;
}

export interface McpStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  url: string;
  token: string;
  allowWrite: boolean;
  error?: string | null;
  exePath: string;
}

export type AppEvent =
  | { type: "syncStarted" }
  | { type: "syncFinished"; report: SyncReport }
  | { type: "syncFailed"; message: string }
  | { type: "analysisStarted"; count: number }
  | { type: "analysisFinished"; report: AnalyzeReport }
  | { type: "analysisFailed"; message: string }
  | { type: "issuesChanged" };

// ---------- 命令 ----------

export const api = {
  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_config", { config }),
  setSecret: (kind: "redmineApiKey" | "aiApiKey", value: string) => invoke<void>("set_secret", { kind, value }),
  secretStatus: () => invoke<SecretStatus>("secret_status"),
  testRedmine: () => invoke<User>("test_redmine"),
  testAi: () => invoke<string>("test_ai"),
  syncNow: () => invoke<SyncReport>("sync_now"),
  listIssues: (filter: IssueFilter) => invoke<IssueView[]>("list_issues", { filter }),
  issueDetail: (id: number, refresh = false) => invoke<IssueDetail>("issue_detail", { id, refresh }),
  markRead: (id: number) => invoke<void>("mark_read", { id }),
  analyze: (ids: number[] | null, force = false) => invoke<AnalyzeReport>("analyze", { ids, force }),
  setClassification: (id: number, category: string | null, module: string | null) =>
    invoke<void>("set_classification", { id, category, module }),
  suggestBatches: (ids: number[] | null) => invoke<Plan>("suggest_batches", { ids }),
  latestPlan: () => invoke<Plan | null>("latest_plan"),
  draftNote: (ids: number[], keyPoints: string) => invoke<string>("draft_note", { ids, keyPoints }),
  resolveIssue: (req: ResolveRequest) => invoke<ResolveOutcome>("resolve_issue", { req }),
  batchResolve: (reqs: ResolveRequest[]) => invoke<ResolveOutcome[]>("batch_resolve", { reqs }),
  requestMoreInfo: (id: number, question: string) => invoke<ResolveOutcome>("request_more_info", { id, question }),
  startProgress: (id: number) => invoke<ResolveOutcome>("start_progress", { id }),
  changeStatus: (id: number, statusId: number, notes: string | null = null) =>
    invoke<ResolveOutcome>("change_status", { id, statusId, notes }),
  stageAttachment: async (file: Blob, filename: string) =>
    invoke<StagedFile>("stage_attachment", new Uint8Array(await file.arrayBuffer()), {
      headers: { "x-filename": encodeURIComponent(filename) },
    }),
  assignees: (projectId: number) => invoke<Assignee[]>("assignees", { projectId }),
  myProjects: () => invoke<ProjectAccess[]>("my_projects"),
  projectOverview: (projectId: number, refresh = false) => invoke<ProjectOverview>("project_overview", { projectId, refresh }),
  suggestAssignments: (projectId: number, issueIds: number[], useAi: boolean) =>
    invoke<AssignSuggestion[]>("suggest_assignments", { projectId, issueIds, useAi }),
  bulkAssign: (items: { issueId: number; assigneeId: number }[], notes: string | null) =>
    invoke<ResolveOutcome[]>("bulk_assign", { items, notes }),
  suggestSplit: (issueId: number, useAi: boolean) => invoke<SplitSuggestion>("suggest_split", { issueId, useAi }),
  splitIssue: (issueId: number, parts: SplitPart[], addNote: boolean) => invoke<SplitOutcome>("split_issue", { issueId, parts, addNote }),
  dispatchQueue: () => invoke<DispatchQueue>("dispatch_queue"),
  dispatchPreview: (issueId: number) => invoke<DispatchQueue>("dispatch_preview", { issueId }),
  dispatchIssue: (req: DispatchRequest) => invoke<DispatchOutcome>("dispatch_issue", { req }),
  dispatchBatch: (reqs: DispatchRequest[]) => invoke<DispatchOutcome[]>("dispatch_batch", { reqs }),
  dispatchKeep: (issueId: number) => invoke<void>("dispatch_keep", { issueId }),
  dispatchUnmark: (issueId: number) => invoke<void>("dispatch_unmark", { issueId }),
  markDuplicate: (issueId: number, originalId: number, notes: string | null, statusId: number | null) =>
    invoke<ResolveOutcome>("mark_duplicate", { issueId, originalId, notes, statusId }),
  aiDispatch: (issueIds: number[]) => invoke<number>("ai_dispatch", { issueIds }),
  dispatchTeam: () => invoke<TeamCandidate[]>("dispatch_team"),
  recentChanges: (limit = 30) => invoke<IssueChange[]>("recent_changes", { limit }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  linkIssues: (id: number, others: number[], relationType = "relates") =>
    invoke<string[]>("link_issues", { id, others, relationType }),
  statuses: () => invoke<NamedId[]>("statuses"),
  activities: () => invoke<NamedId[]>("activities"),
  stats: () => invoke<Stats>("stats"),
  workReport: (days: number) => invoke<string>("work_report", { days }),
  recentResolutions: (days: number) => invoke<ResolutionRecord[]>("recent_resolutions", { days }),
  mcpStatus: () => invoke<McpStatus>("mcp_status"),
  restartMcp: () => invoke<McpStatus>("restart_mcp"),
  regenerateMcpToken: () => invoke<McpStatus>("regenerate_mcp_token"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
};

const blobUrls = new Map<number, Promise<string>>();

/** 附件的本地预览地址：后端带 API Key 下载并缓存，前端转成 Blob URL（同一附件只取一次） */
export function attachmentUrl(id: number, contentType?: string | null): Promise<string> {
  let p = blobUrls.get(id);
  if (!p) {
    p = invoke<ArrayBuffer>("attachment_bytes", { id }).then((buf) => URL.createObjectURL(new Blob([buf], { type: contentType ?? "" })));
    p.catch(() => blobUrls.delete(id));
    blobUrls.set(id, p);
  }
  return p;
}

export function onAppEvent(cb: (e: AppEvent) => void): Promise<UnlistenFn> {
  return listen<AppEvent>("redpilot://event", (e) => cb(e.payload));
}

/** 错误信息形如 "[CODE] message"，拆出错误码 */
export function errorCode(err: unknown): string | null {
  const m = /^\[([A-Z_]+)\]/.exec(String(err));
  return m ? m[1] : null;
}

export function errorText(err: unknown): string {
  return String(err).replace(/^\[[A-Z_]+\]\s*/, "");
}
