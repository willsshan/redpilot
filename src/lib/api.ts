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
  doneRatio?: number | null;
  spentHours?: number | null;
  activityId?: number | null;
  privateNotes: boolean;
  /** 暂存附件的本地路径 */
  attachments: string[];
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
