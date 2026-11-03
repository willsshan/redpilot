import { useEffect, useMemo, useState } from "react";
import { GitFork, ListOrdered, Plus, Sparkles, Trash2 } from "lucide-react";
import { api, errorText, type SplitPart, type SplitSuggestion } from "../lib/api";
import { useToast } from "../lib/toast";
import { modKey } from "../lib/format";
import { Badge, Button, Input, Modal, Select, Spinner, Switch, Textarea } from "./ui";
import { MemberOptions, SOURCE_LABEL, sourceTone } from "./AssignDialog";

type Row = SplitPart & { key: number; suggestedId?: number | null };

let seq = 1;
const blank = (): Row => ({ key: seq++, subject: "", description: "" });
const toRows = (parts: SplitPart[]): Row[] => parts.map((p) => ({ ...p, key: seq++, suggestedId: p.assigneeId ?? null }));

/** 拆分任务：同一个问题的不同部分（前端、后端、测试……）建成子任务，分别指派 */
export function SplitDialog({ issueId, onClose, onDone }: { issueId: number | null; onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const [ctx, setCtx] = useState<SplitSuggestion | null>(null);
  const [rows, setRows] = useState<Row[]>([]);
  const [hint, setHint] = useState<string | null>(null);
  const [busy, setBusy] = useState<"local" | "ai" | null>(null);
  const [addNote, setAddNote] = useState(true);
  const open = issueId !== null;

  async function suggest(useAi: boolean, initial = false) {
    if (issueId === null) return;
    setBusy(useAi ? "ai" : "local");
    try {
      const s = await api.suggestSplit(issueId, useAi);
      setCtx(s);
      setHint(s.hint ?? null);
      if (s.parts.length > 0) setRows(toRows(s.parts));
      else if (initial) setRows([blank(), blank()]);
    } catch (e) {
      toast.show({ tone: "error", title: useAi ? "AI 拆分失败" : "读取拆分建议失败", body: errorText(e) });
      if (initial) setRows([blank(), blank()]);
    } finally {
      setBusy(null);
    }
  }

  useEffect(() => {
    if (!open) return;
    setCtx(null);
    setRows([]);
    setHint(null);
    setAddNote(true);
    void suggest(false, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issueId]);

  const load = useMemo(() => new Map(ctx?.load ?? []), [ctx]);
  const members = ctx?.members ?? [];
  const valid = rows.filter((r) => r.subject.trim());
  const people = [...new Set(valid.map((r) => members.find((m) => m.id === r.assigneeId)?.name ?? "未指派"))];

  function update(key: number, patch: Partial<Row>) {
    setRows((rs) => rs.map((r) => (r.key === key ? { ...r, ...patch } : r)));
  }

  function submit() {
    if (issueId === null || valid.length === 0) return;
    const parts: SplitPart[] = valid.map((r) => ({
      subject: r.subject.trim(),
      description: r.description.trim(),
      assigneeId: r.assigneeId ?? null,
      categoryId: r.categoryId ?? null,
      estimatedHours: r.estimatedHours ?? null,
    }));
    const id = issueId;
    onClose();
    toast.withUndo(`正在把 #${id} 拆分为 ${parts.length} 个子任务`, async () => {
      try {
        const out = await api.splitIssue(id, parts, addNote);
        const lines = [
          ...out.created.map((c) => `#${c.id} ${c.subject} → ${c.assignee || "未指派"}`),
          ...out.failed.map((f) => `失败：${f}`),
          ...out.warnings,
        ];
        toast.show({
          tone: out.failed.length || out.warnings.length ? "warn" : "success",
          title: `已创建 ${out.created.length} 个子任务${out.failed.length ? `，${out.failed.length} 个失败` : ""}`,
          body: lines.join("\n"),
        });
      } catch (e) {
        toast.show({ tone: "error", title: "拆分失败", body: errorText(e) });
      }
      onDone();
    });
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

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={1120}
      title={ctx ? `拆分任务 #${ctx.issueId}` : "拆分任务"}
      subtitle="把问题的不同部分建成子任务，分别指派给不同的人；子任务沿用原任务的项目、跟踪、优先级、版本和截止日期"
      footer={
        <>
          <label className="mr-auto flex items-center gap-2 text-[12.5px] text-ink-2">
            <Switch checked={addNote} onChange={setAddNote} />
            在原任务添加拆分说明（列出子任务和负责人）
          </label>
          <span className="text-[12px] text-ink-3">
            {valid.length > 0 ? `分给 ${people.slice(0, 3).join("、")}${people.length > 3 ? ` 等 ${people.length} 人` : ""} · ` : ""}提交后 5 秒内可撤销
          </span>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" icon={<GitFork className="size-4" />} disabled={valid.length === 0 || busy !== null} onClick={submit}>
            创建 {valid.length} 个子任务 <span className="opacity-70">{modKey}↵</span>
          </Button>
        </>
      }
    >
      <div className="flex min-h-0 flex-1">
        {/* 原问题 */}
        <aside className="hidden w-[340px] shrink-0 flex-col border-r border-line bg-sunken/40 md:flex">
          <div className="border-b border-line px-4 py-3">
            <div className="text-[11.5px] font-medium text-ink-3">原问题{ctx?.category ? ` · ${ctx.category}` : ""}</div>
            <div className="mt-0.5 text-[13.5px] leading-snug font-semibold text-ink">{ctx?.subject ?? "…"}</div>
          </div>
          <div className="selectable max-h-[58vh] min-h-0 flex-1 overflow-y-auto px-4 py-3 text-[12.5px] leading-relaxed whitespace-pre-wrap text-ink-2">
            {ctx ? ctx.description.trim() || "（无描述）" : <Spinner />}
          </div>
        </aside>

        {/* 子任务 */}
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3">
            <Button variant="ai" icon={<Sparkles className="size-3.5" />} loading={busy === "ai"} disabled={busy === "local"} onClick={() => void suggest(true)}>
              AI 拆分
            </Button>
            <Button icon={<ListOrdered className="size-3.5" />} loading={busy === "local"} disabled={busy === "ai"} onClick={() => void suggest(false)}>
              按条目拆分
            </Button>
            <div className="flex-1" />
            <Button variant="ghost" icon={<Plus className="size-3.5" />} onClick={() => setRows((rs) => [...rs, blank()])}>
              添加子任务
            </Button>
          </div>

          <div className="max-h-[58vh] min-h-[280px] space-y-3 overflow-y-auto px-5 py-4">
            {hint && <div className="rounded-lg bg-info-soft px-3 py-2 text-[12.5px] text-info">{hint}</div>}
            {rows.length === 0 && busy && (
              <div className="flex h-40 items-center justify-center gap-2 text-[12.5px] text-ink-3">
                <Spinner /> 正在读取项目成员和历史分工…
              </div>
            )}
            {rows.map((r, idx) => (
              <div key={r.key} className="rounded-xl border border-line bg-panel p-3 shadow-card">
                <div className="flex items-center gap-2">
                  <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-accent-soft text-[12px] font-semibold text-accent-ink">{idx + 1}</span>
                  <Input
                    value={r.subject}
                    onChange={(e) => update(r.key, { subject: e.target.value })}
                    placeholder={`子任务标题，例如：${ctx?.subject ? `${ctx.subject.slice(0, 16)}：` : ""}前端调整页面`}
                    className="h-8 flex-1 font-medium"
                  />
                  <button
                    className="rounded-md p-1.5 text-ink-3 hover:bg-hover hover:text-accent-ink"
                    title="删除这个子任务"
                    onClick={() => setRows((rs) => rs.filter((x) => x.key !== r.key))}
                  >
                    <Trash2 className="size-4" />
                  </button>
                </div>
                <div className="mt-2 pl-8">
                  <Textarea
                    rows={3}
                    value={r.description}
                    onChange={(e) => update(r.key, { description: e.target.value })}
                    placeholder="这部分具体要做什么（被指派人只看到这里的内容和原任务链接）"
                    className="text-[12.5px]"
                  />
                </div>
                <div className="mt-2 flex flex-wrap items-center gap-2 pl-8 text-[12px] text-ink-3">
                  <span>负责人</span>
                  <div className="w-44">
                    <Select
                      value={r.assigneeId ?? ""}
                      onChange={(e) => update(r.key, { assigneeId: e.target.value ? Number(e.target.value) : null })}
                      className="h-7 text-[12.5px]"
                    >
                      <option value="">未指派</option>
                      <MemberOptions members={members} load={load} />
                    </Select>
                  </div>
                  {(ctx?.categories.length ?? 0) > 0 && (
                    <>
                      <span className="ml-2">类别</span>
                      <div className="w-40">
                        <Select
                          value={r.categoryId ?? ""}
                          onChange={(e) => update(r.key, { categoryId: e.target.value ? Number(e.target.value) : null })}
                          className="h-7 text-[12.5px]"
                        >
                          <option value="">沿用原任务{ctx?.category ? `（${ctx.category}）` : ""}</option>
                          {ctx?.categories.map((c) => (
                            <option key={c.id} value={c.id}>
                              {c.name}
                            </option>
                          ))}
                        </Select>
                      </div>
                    </>
                  )}
                  <span className="ml-2">预估</span>
                  <div className="w-16">
                    <Input
                      type="number"
                      min={0}
                      step={0.5}
                      value={r.estimatedHours ?? ""}
                      onChange={(e) => update(r.key, { estimatedHours: e.target.value ? Number(e.target.value) : null })}
                      placeholder="—"
                      className="h-7 text-[12.5px]"
                    />
                  </div>
                  <span>h</span>
                  {r.reason && r.source && r.assigneeId === r.suggestedId && (
                    <span className="flex min-w-0 basis-full items-center gap-1">
                      <Badge tone={sourceTone(r.source)}>{SOURCE_LABEL[r.source]}</Badge>
                      <span className="truncate" title={r.reason}>
                        {r.reason}
                      </span>
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </Modal>
  );
}
