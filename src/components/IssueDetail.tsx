import { useState, type ReactNode } from "react";
import clsx from "clsx";
import {
  AlertTriangle,
  BookOpenCheck,
  CornerUpLeft,
  ExternalLink,
  MessageCircleQuestion,
  Paperclip,
  Pause,
  Play,
  PlayCircle,
  RefreshCw,
  Sparkles,
  Timer,
  X,
} from "lucide-react";
import { api, errorText, type IssueDetail as Detail, type Journal } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { categoryColor, dateTime, dueLabel, formatHours, priorityTone, relativeTime, urgencyColor } from "../lib/format";
import { useIssueTimer } from "../lib/timer";
import { Avatar, Badge, Button, IconButton, Input, Kbd, SectionTitle, Select, Spinner } from "./ui";
import { isImage, RichText, Thumbnail } from "./RichText";
import type { Attachment } from "../lib/api";

const FIELD_NAMES: Record<string, string> = {
  status_id: "状态",
  assigned_to_id: "指派",
  priority_id: "优先级",
  done_ratio: "完成度",
  due_date: "截止日期",
  subject: "标题",
  description: "描述",
  tracker_id: "跟踪",
  category_id: "类别",
  fixed_version_id: "目标版本",
  estimated_hours: "预估工时",
  start_date: "开始日期",
};

export function IssueDetail({
  detail,
  loading,
  onRefresh,
  onResolve,
  onFollowUp,
  onClose,
}: {
  detail: Detail | null;
  loading: boolean;
  onRefresh: () => void;
  onResolve: () => void;
  onFollowUp: () => void;
  /** 在看板抽屉中显示时提供关闭按钮 */
  onClose?: () => void;
}) {
  const { config } = useApp();
  const toast = useToast();
  const timer = useIssueTimer(detail?.view.id ?? null);
  const [analyzing, setAnalyzing] = useState(false);
  const [starting, setStarting] = useState(false);

  if (!detail) {
    return <div className="flex h-full items-center justify-center">{loading ? <Spinner /> : null}</div>;
  }
  const v = detail.view;
  const a = v.analysis;
  const is = detail.issue;
  const due = dueLabel(v.dueDate);
  const categories = [...new Set([...(config?.rules.map((r) => r.category) ?? []), "其他"])];

  async function reanalyze() {
    setAnalyzing(true);
    try {
      await api.analyze([v.id], true);
      onRefresh();
    } catch (e) {
      toast.show({ tone: "error", title: "AI 分析失败", body: errorText(e) });
    } finally {
      setAnalyzing(false);
    }
  }

  async function startProgress() {
    setStarting(true);
    try {
      const out = await api.startProgress(v.id);
      if (!timer.running) timer.toggle();
      toast.show({ tone: "success", title: `已开始处理，状态：${out.status ?? ""}` });
      onRefresh();
    } catch (e) {
      toast.show({ tone: "error", title: "切换状态失败", body: errorText(e) });
    } finally {
      setStarting(false);
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[860px] px-8 pt-5 pb-10">
          {/* 头部 */}
          <div className="mb-2 flex items-center gap-2 text-[12.5px] text-ink-3">
            <span>{v.projectName}</span>
            <span>·</span>
            <span>{v.trackerName}</span>
            <span className="font-mono">#{v.id}</span>
            <div className="flex-1" />
            {loading && <Spinner className="size-3.5" />}
            <IconButton title="从 Redmine 刷新" onClick={onRefresh}>
              <RefreshCw className="size-3.5" />
            </IconButton>
            <IconButton title="在浏览器中打开" onClick={() => void api.openUrl(detail.webUrl)}>
              <ExternalLink className="size-3.5" />
            </IconButton>
            {onClose && (
              <IconButton title="关闭 (Esc)" onClick={onClose}>
                <X className="size-4" />
              </IconButton>
            )}
          </div>
          <h1 className="selectable text-[20px] leading-snug font-semibold tracking-tight text-ink">{v.subject}</h1>

          <div className="mt-3 flex flex-wrap items-center gap-2 text-[12.5px] text-ink-2">
            <Badge tone="info">{v.statusName}</Badge>
            <Badge tone={priorityTone(v.priorityName) === "high" ? "accent" : "muted"}>优先级 {v.priorityName}</Badge>
            {due && <Badge tone={due.tone === "danger" ? "accent" : due.tone === "warn" ? "warn" : "muted"}>{due.text}</Badge>}
            <span className="ml-1 inline-flex items-center gap-1.5">
              <Avatar name={v.authorName} size={18} />
              {v.authorName} 提出于 {relativeTime(v.createdOn)}
            </span>
            <span className="text-ink-3">· 更新于 {relativeTime(v.updatedOn)}</span>
          </div>

          {/* AI 分析 */}
          <div className="ai-card mt-5 rounded-xl p-4 shadow-card">
            <div className="mb-3 flex items-center gap-2">
              <Sparkles className="size-4 text-accent" />
              <span className="text-[13px] font-semibold">AI 分析</span>
              {a.aiStale && <Badge tone="warn">任务有更新，建议重新分析</Badge>}
              <div className="flex-1" />
              <Button size="sm" variant="ghost" loading={analyzing} icon={<RefreshCw className="size-3" />} onClick={() => void reanalyze()}>
                {a.aiAnalyzed ? "重新分析" : "分析此任务"}
              </Button>
            </div>

            {a.summary ? (
              <div className="selectable mb-3 text-[14px] leading-relaxed font-medium text-ink">{a.summary}</div>
            ) : (
              !a.aiAnalyzed && <div className="mb-3 text-[12.5px] text-ink-3">尚未进行 AI 分析，当前分类来自关键词规则。</div>
            )}

            <div className="grid grid-cols-5 gap-3">
              <Meta label="分类">
                <Select
                  value={a.category ?? ""}
                  onChange={(e) => void api.setClassification(v.id, e.target.value || null, a.module ?? null).then(onRefresh)}
                  className="h-7 border-transparent bg-sunken px-1.5 text-[12.5px] font-medium"
                  style={{ color: categoryColor(a.category) }}
                >
                  <option value="">未分类</option>
                  {categories.map((c) => (
                    <option key={c} value={c}>
                      {c}
                    </option>
                  ))}
                </Select>
              </Meta>
              <Meta label="模块">
                <Input
                  key={`${v.id}-${a.module}`}
                  defaultValue={a.module ?? ""}
                  placeholder="—"
                  className="h-7 border-transparent bg-sunken px-1.5 text-[12.5px]"
                  onBlur={(e) => {
                    if (e.target.value !== (a.module ?? "")) void api.setClassification(v.id, a.category ?? null, e.target.value || null).then(onRefresh);
                  }}
                />
              </Meta>
              <Meta label="紧急度">
                <span className="text-[13px] font-semibold" style={{ color: urgencyColor(a.urgency) }}>
                  {a.urgency ?? "—"}
                </span>
              </Meta>
              <Meta label="难度">
                <span className="flex gap-0.5 pt-1">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <span key={n} className={clsx("h-1.5 w-3 rounded-full", n <= (a.difficulty ?? 0) ? "bg-accent" : "bg-line")} />
                  ))}
                </span>
              </Meta>
              <Meta label="预估工时">
                <span className="text-[13px] font-semibold tabular-nums">{a.estHours != null ? `${a.estHours} h` : "—"}</span>
              </Meta>
            </div>

            {a.suggestion && (
              <div className="mt-3 rounded-lg bg-sunken/70 p-3">
                <div className="mb-1 text-[11.5px] font-semibold text-ink-3">处理思路</div>
                <div className="selectable text-[13px] leading-relaxed text-ink-2">{a.suggestion}</div>
              </div>
            )}

            {a.missingInfo.length > 0 && (
              <div className="mt-3 flex items-start gap-2.5 rounded-lg bg-warn-soft p-3">
                <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warn" />
                <div className="flex-1">
                  <div className="text-[12.5px] font-semibold text-warn">信息不足，可能需要追问提出人</div>
                  <ul className="mt-1 list-disc pl-4 text-[12.5px] text-ink-2">
                    {a.missingInfo.map((m) => (
                      <li key={m}>{m}</li>
                    ))}
                  </ul>
                </div>
                <Button size="sm" icon={<MessageCircleQuestion className="size-3.5" />} onClick={onFollowUp}>
                  追问
                </Button>
              </div>
            )}
          </div>

          {/* 描述 */}
          <div className="mt-7">
            <SectionTitle>问题描述</SectionTitle>
            {is.description?.trim() ? (
              <RichText text={is.description.trim()} attachments={is.attachments} className="text-[13.5px] leading-[1.75] text-ink" />
            ) : (
              <div className="text-[13.5px] text-ink-3">（无描述）</div>
            )}
          </div>

          {is.attachments.length > 0 && (
            <div className="mt-6">
              <SectionTitle icon={<Paperclip className="size-3.5" />}>附件 {is.attachments.length}</SectionTitle>
              {is.attachments.some(isImage) && (
                <div className="mb-2 flex flex-wrap gap-2">
                  {is.attachments.filter(isImage).map((f) => (
                    <Thumbnail key={f.id} att={f} attachments={is.attachments} />
                  ))}
                </div>
              )}
              <div className="flex flex-wrap gap-2">
                {is.attachments.filter((f) => !isImage(f)).map((f) => (
                  <button
                    key={f.id}
                    onClick={() => void api.openUrl(f.contentUrl)}
                    className="flex items-center gap-2 rounded-lg border border-line bg-panel px-2.5 py-1.5 text-[12.5px] hover:bg-hover"
                  >
                    <Paperclip className="size-3.5 text-ink-3" />
                    <span className="max-w-[220px] truncate">{f.filename}</span>
                    <span className="text-ink-3">{(f.filesize / 1024).toFixed(0)} KB</span>
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* 相似历史 */}
          {detail.similar.length > 0 && (
            <div className="mt-7">
              <SectionTitle icon={<BookOpenCheck className="size-3.5" />}>相似历史任务</SectionTitle>
              <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-panel">
                {detail.similar.map((s) => (
                  <details key={s.id} className="group">
                    <summary className="flex cursor-default list-none items-center gap-3 px-3.5 py-2.5 hover:bg-hover">
                      <span className="font-mono text-[12px] text-ink-3">#{s.id}</span>
                      <span className="min-w-0 flex-1 truncate text-[13px]">{s.subject}</span>
                      {s.resolutionNotes && <Badge tone="ok">有处置记录</Badge>}
                      <span className="text-[11.5px] text-ink-3 tabular-nums">相似度 {Math.round(s.score * 100)}%</span>
                    </summary>
                    <div className="selectable border-t border-line bg-sunken/50 px-4 py-3 text-[12.5px] whitespace-pre-wrap text-ink-2">
                      {s.resolutionNotes || `状态：${s.statusName}（该任务未通过 RedPilot 回派，没有处置记录）`}
                    </div>
                  </details>
                ))}
              </div>
            </div>
          )}

          {/* 沟通记录 */}
          <div className="mt-7">
            <SectionTitle>沟通记录 {is.journals.length > 0 && is.journals.length}</SectionTitle>
            {is.journals.length === 0 ? (
              <div className="text-[12.5px] text-ink-3">暂无</div>
            ) : (
              <ol className="relative space-y-4 border-l border-line pl-5">
                {is.journals.map((j) => (
                  <JournalItem key={j.id} j={j} attachments={is.attachments} />
                ))}
              </ol>
            )}
          </div>
        </div>
      </div>

      {/* 底部操作栏 */}
      <div className="flex shrink-0 items-center gap-2 border-t border-line bg-panel/80 px-5 py-3 backdrop-blur">
        <button
          onClick={timer.toggle}
          className={clsx(
            "flex h-8 items-center gap-2 rounded-lg border px-3 text-[12.5px] font-medium tabular-nums",
            timer.running ? "border-accent/40 bg-accent-soft text-accent-ink" : "border-line text-ink-2 hover:bg-hover",
          )}
          title="处理计时，回派时自动换算工时"
        >
          {timer.running ? <Pause className="size-3.5" /> : timer.elapsedMs > 0 ? <Play className="size-3.5" /> : <Timer className="size-3.5" />}
          {timer.elapsedMs > 0 ? formatHours(timer.elapsedMs) : "计时"}
        </button>
        {config?.workflow.inProgressStatusId && v.statusId !== config.workflow.inProgressStatusId && (
          <Button variant="ghost" icon={<PlayCircle className="size-4" />} loading={starting} onClick={() => void startProgress()}>
            开始处理
          </Button>
        )}
        <div className="flex-1" />
        <Button icon={<MessageCircleQuestion className="size-4" />} onClick={onFollowUp}>
          追问提出人
        </Button>
        <Button variant="primary" size="lg" icon={<CornerUpLeft className="size-4" />} onClick={onResolve}>
          处理完成 · 回派给 {v.authorName || "提出人"}
          <Kbd>R</Kbd>
        </Button>
      </div>
    </div>
  );
}

function Meta({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 text-[11.5px] text-ink-3">{label}</div>
      {children}
    </div>
  );
}

function JournalItem({ j, attachments }: { j: Journal; attachments: Attachment[] }) {
  const name = j.user?.name ?? "未知用户";
  const changes = j.details.filter((d) => d.property === "attr");
  // 这条记录里上传的图片；已在备注中引用的不再重复显示
  const added = j.details
    .filter((d) => d.property === "attachment" && d.newValue)
    .map((d) => attachments.find((a) => String(a.id) === d.name))
    .filter((a): a is Attachment => !!a && isImage(a) && !(j.notes ?? "").includes(a.filename));
  return (
    <li className="relative">
      <span className="absolute top-0.5 -left-[31px] rounded-full ring-4 ring-canvas">
        <Avatar name={name} size={20} />
      </span>
      <div className="flex items-center gap-2 text-[12.5px]">
        <span className="font-medium text-ink">{name}</span>
        <span className="text-ink-3">{dateTime(j.createdOn)}</span>
        {j.privateNotes && <Badge tone="warn">私有</Badge>}
      </div>
      {changes.length > 0 && (
        <div className="mt-0.5 text-[12px] text-ink-3">
          {changes
            .map((d) => {
              const f = FIELD_NAMES[d.name] ?? d.name;
              return d.name === "done_ratio" || d.name === "due_date" ? `${f}：${d.oldValue ?? "—"} → ${d.newValue ?? "—"}` : `更新了${f}`;
            })
            .join("，")}
        </div>
      )}
      {j.notes?.trim() && (
        <RichText
          text={j.notes.trim()}
          attachments={attachments}
          className="mt-1.5 rounded-lg border border-line bg-panel px-3 py-2 text-[13px] leading-relaxed text-ink"
        />
      )}
      {added.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-2">
          {added.map((a) => (
            <Thumbnail key={a.id} att={a} attachments={attachments} />
          ))}
        </div>
      )}
    </li>
  );
}
