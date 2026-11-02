import { useEffect, useMemo, useState } from "react";
import { ArrowRight, CornerUpLeft, FileText, Sparkles, Wand2 } from "lucide-react";
import { api, errorCode, errorText, type IssueView, type NamedId, type ResolveRequest } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { modKey, noteTemplate } from "../lib/format";
import { clearTimer, msToHours, timerElapsed } from "../lib/timer";
import { Avatar, Button, Field, Input, Modal, Select, Switch, Textarea } from "./ui";

/** 处理完成 → 写处置说明 → 指派回提出人。支持单个或一组合并处理的任务。 */
export function ResolveDialog({
  open,
  onClose,
  issues,
  allowedStatuses,
}: {
  open: boolean;
  onClose: () => void;
  issues: IssueView[];
  /** 单个任务时传入 Redmine 返回的可选状态 */
  allowedStatuses?: NamedId[];
}) {
  const { config } = useApp();
  const toast = useToast();
  const [points, setPoints] = useState("");
  const [notes, setNotes] = useState("");
  const [drafting, setDrafting] = useState(false);
  const [statuses, setStatuses] = useState<NamedId[]>([]);
  const [activities, setActivities] = useState<NamedId[]>([]);
  const [statusId, setStatusId] = useState<number | null>(null);
  const [assignBack, setAssignBack] = useState(true);
  const [doneRatio, setDoneRatio] = useState(true);
  const [hours, setHours] = useState("");
  const [activityId, setActivityId] = useState<number | null>(null);
  const [privateNotes, setPrivateNotes] = useState(false);

  const batch = issues.length > 1;
  const ids = useMemo(() => issues.map((i) => i.id), [issues]);

  useEffect(() => {
    if (!open || !config) return;
    setPoints("");
    setNotes("");
    setStatusId(config.workflow.resolvedStatusId ?? null);
    setAssignBack(true);
    setDoneRatio(config.workflow.setDoneRatio);
    setActivityId(config.workflow.defaultActivityId ?? null);
    setPrivateNotes(false);
    const h = batch ? null : msToHours(timerElapsed(issues[0]?.id ?? 0));
    setHours(h ? String(h) : "");
    api.statuses().then(setStatuses).catch(() => setStatuses([]));
    api.activities().then(setActivities).catch(() => setActivities([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, ids.join(",")]);

  const statusOptions = allowedStatuses && allowedStatuses.length > 0 ? allowedStatuses : statuses;
  const textFormat = config?.redmine.textFormat ?? "markdown";

  async function draft() {
    setDrafting(true);
    try {
      setNotes(await api.draftNote(ids, points));
    } catch (e) {
      toast.show({ tone: "error", title: "生成说明失败", body: errorText(e) });
    } finally {
      setDrafting(false);
    }
  }

  function submit() {
    if (!notes.trim()) {
      toast.show({ tone: "warn", title: "请先填写处置说明" });
      return;
    }
    const h = parseFloat(hours);
    const base = {
      notes,
      statusId,
      assignToAuthor: assignBack,
      doneRatio: doneRatio ? 100 : null,
      spentHours: Number.isFinite(h) && h > 0 ? h : null,
      activityId,
      privateNotes,
    };
    const reqs: ResolveRequest[] = issues.map((i) => ({ ...base, issueId: i.id }));
    onClose();

    const who = batch ? `${issues.length} 个任务` : `#${issues[0].id} 给 ${issues[0].authorName || "提出人"}`;
    toast.withUndo(`正在回派 ${who}`, async () => {
      if (batch) {
        const outs = await api.batchResolve(reqs);
        const ok = outs.filter((o) => o.ok);
        const failed = outs.filter((o) => !o.ok);
        ok.forEach((o) => clearTimer(o.issueId));
        toast.show({
          tone: failed.length ? "warn" : "success",
          title: `已回派 ${ok.length} 个任务${failed.length ? `，${failed.length} 个失败` : ""}`,
          body: failed.map((f) => `#${f.issueId}：${errorText(f.error)}`).join("\n") || undefined,
        });
        return;
      }
      await resolveOne(reqs[0]);
    });
  }

  async function resolveOne(req: ResolveRequest) {
    try {
      const out = await api.resolveIssue(req);
      clearTimer(req.issueId);
      toast.show({
        tone: out.warnings.length ? "warn" : "success",
        title: `#${req.issueId} 已回派${out.assignedTo ? `给 ${out.assignedTo}` : ""}`,
        body: [out.status ? `状态：${out.status}` : "", ...out.warnings].filter(Boolean).join("\n"),
      });
    } catch (e) {
      const code = errorCode(e);
      if (code === "ASSIGNEE_INVALID") {
        toast.show(
          {
            tone: "error",
            title: "无法指派给提出人",
            body: errorText(e),
            action: { label: "仅提交说明", onClick: () => void resolveOne({ ...req, assignToAuthor: false }) },
          },
          12000,
        );
      } else {
        toast.show({ tone: "error", title: `#${req.issueId} 提交失败`, body: errorText(e) });
      }
    }
  }

  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        submit();
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  });

  const authors = [...new Set(issues.map((i) => i.authorName).filter(Boolean))];

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={920}
      title={batch ? `批量处理完成 · ${issues.length} 个任务` : "处理完成 · 回派给提出人"}
      subtitle={batch ? issues.map((i) => `#${i.id}`).join("  ") : `#${issues[0]?.id} ${issues[0]?.subject ?? ""}`}
      footer={
        <>
          <span className="mr-auto text-[12px] text-ink-3">提交后 5 秒内可撤销</span>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" icon={<CornerUpLeft className="size-4" />} onClick={submit}>
            提交并回派 <span className="opacity-70">{modKey}↵</span>
          </Button>
        </>
      }
    >
      <div className="flex min-h-[460px]">
        <div className="flex-1 space-y-4 p-5">
          <Field label="处理要点" hint="写几句关键信息即可，例如：原因是缓存未刷新；已清理缓存并增加失效机制；需要用户重新登录验证">
            <Textarea rows={3} value={points} onChange={(e) => setPoints(e.target.value)} placeholder="简单写下原因和处理方法，AI 会扩写成规范的处置说明…" autoFocus />
          </Field>
          <div className="flex gap-2">
            <Button variant="ai" icon={<Sparkles className="size-3.5" />} loading={drafting} onClick={() => void draft()}>
              AI 生成处置说明
            </Button>
            <Button
              variant="ghost"
              icon={<FileText className="size-3.5" />}
              onClick={() => setNotes(noteTemplate(config?.workflow.noteSections ?? [], textFormat, points))}
            >
              插入模板
            </Button>
          </div>
          <Field
            label={
              <span className="flex items-center justify-between">
                <span>处置说明（提出人可见）</span>
                <span className="font-normal text-ink-3">{textFormat === "markdown" ? "Markdown" : "Textile"} · {notes.length} 字</span>
              </span>
            }
          >
            <Textarea rows={13} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="可直接编辑，也可以先用 AI 生成" />
          </Field>
        </div>

        <div className="w-[280px] shrink-0 space-y-4 border-l border-line bg-sunken/40 p-5">
          <div className="text-[12px] font-semibold tracking-wide text-ink-3">将执行的操作</div>

          <div className="space-y-2 rounded-xl border border-line bg-panel p-3">
            <div className="flex items-center justify-between">
              <span className="text-[13px] font-medium">指派回提出人</span>
              <Switch checked={assignBack} onChange={setAssignBack} />
            </div>
            {assignBack && (
              <div className="flex flex-wrap items-center gap-1.5 text-[12.5px] text-ink-2">
                <span>我</span>
                <ArrowRight className="size-3 text-ink-3" />
                {authors.slice(0, 3).map((a) => (
                  <span key={a} className="inline-flex items-center gap-1">
                    <Avatar name={a} size={18} />
                    {a}
                  </span>
                ))}
                {authors.length > 3 && <span>等 {authors.length} 人</span>}
              </div>
            )}
          </div>

          <Field label="切换状态" hint={!statusId ? "未选择时不修改状态；可在设置中配置默认值" : undefined}>
            <Select value={statusId ?? ""} onChange={(e) => setStatusId(e.target.value ? Number(e.target.value) : null)}>
              <option value="">不修改</option>
              {statusOptions.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </Select>
          </Field>

          <div className="flex items-center justify-between">
            <span className="text-[12.5px] font-medium text-ink-2">完成度设为 100%</span>
            <Switch checked={doneRatio} onChange={setDoneRatio} />
          </div>

          <div className="grid grid-cols-[1fr_1.3fr] gap-2">
            <Field label={batch ? "每个任务工时" : "登记工时"}>
              <Input type="number" min={0} step={0.1} value={hours} onChange={(e) => setHours(e.target.value)} placeholder="小时" />
            </Field>
            <Field label="活动类型">
              <Select value={activityId ?? ""} onChange={(e) => setActivityId(e.target.value ? Number(e.target.value) : null)}>
                <option value="">默认</option>
                {activities.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </Select>
            </Field>
          </div>

          <div className="flex items-center justify-between">
            <span className="text-[12.5px] font-medium text-ink-2">私有备注</span>
            <Switch checked={privateNotes} onChange={setPrivateNotes} />
          </div>

          {!batch && issues[0] && (
            <div className="rounded-lg bg-panel p-3 text-[12px] leading-relaxed text-ink-3">
              <Wand2 className="mb-1 size-3.5 text-accent" />
              计时器记录的处理时长会自动换算为工时，可以手动修改。
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}

/** 信息不足：向提出人追问，状态改为“反馈”并指派给提出人 */
export function FollowUpDialog({ open, onClose, issue }: { open: boolean; onClose: () => void; issue: IssueView | null }) {
  const toast = useToast();
  const [text, setText] = useState("");

  useEffect(() => {
    if (!open || !issue) return;
    const a = issue.analysis;
    if (a.followUp) setText(a.followUp);
    else if (a.missingInfo.length)
      setText(`您好，为尽快处理该问题，麻烦补充以下信息：\n${a.missingInfo.map((m, i) => `${i + 1}. ${m}`).join("\n")}\n\n谢谢！`);
    else setText("您好，为尽快处理该问题，麻烦补充以下信息：\n1. \n\n谢谢！");
  }, [open, issue]);

  if (!issue) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      width={600}
      title={`向 ${issue.authorName || "提出人"} 追问`}
      subtitle={`#${issue.id} ${issue.subject}`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              onClose();
              toast.withUndo(`正在向 ${issue.authorName} 追问`, async () => {
                try {
                  const out = await api.requestMoreInfo(issue.id, text);
                  toast.show({
                    tone: out.warnings.length ? "warn" : "success",
                    title: `已发送追问并指派给 ${out.assignedTo ?? "提出人"}`,
                    body: out.warnings.join("\n") || undefined,
                  });
                } catch (e) {
                  toast.show({ tone: "error", title: "追问失败", body: errorText(e) });
                }
              });
            }}
          >
            发送追问
          </Button>
        </>
      }
    >
      <div className="space-y-3 p-5">
        <Textarea rows={9} value={text} onChange={(e) => setText(e.target.value)} />
        <div className="text-[12px] text-ink-3">发送后会写入 Redmine 备注，状态切换为“反馈”（在设置中配置），并指派给提出人。</div>
      </div>
    </Modal>
  );
}
