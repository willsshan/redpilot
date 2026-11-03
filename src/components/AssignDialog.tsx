import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { ArrowRight, History, Sparkles, Users } from "lucide-react";
import { api, errorText, type Assignee, type AssignSuggestion, type ProjectIssue } from "../lib/api";
import { useToast } from "../lib/toast";
import { modKey } from "../lib/format";
import { Badge, Button, Modal, Select, Textarea } from "./ui";

export const SOURCE_LABEL: Record<AssignSuggestion["source"], string> = {
  ai: "AI",
  category: "类别负责人",
  history: "相似任务",
  workload: "负载最低",
};

export const sourceTone = (s: AssignSuggestion["source"]): "accent" | "ok" | "info" => (s === "ai" ? "accent" : s === "category" ? "ok" : "info");

/** 成员下拉选项：用户（带当前未关闭数）和组 */
export function MemberOptions({ members, load }: { members: Assignee[]; load: Map<number, number> }) {
  const users = members.filter((m) => !m.isGroup);
  const groups = members.filter((m) => m.isGroup);
  return (
    <>
      {users.length > 0 && (
        <optgroup label="成员（当前未关闭数）">
          {users.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}（{load.get(m.id) ?? 0}）
            </option>
          ))}
        </optgroup>
      )}
      {groups.length > 0 && (
        <optgroup label="组">
          {groups.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </optgroup>
      )}
    </>
  );
}

/** 批量转派：每个任务可以指派给不同的人，支持“全部指派给某人”和按历史 / AI 推荐 */
export function AssignDialog({
  open,
  onClose,
  projectId,
  issues,
  members,
  load,
  onDone,
}: {
  open: boolean;
  onClose: () => void;
  projectId: number;
  issues: ProjectIssue[];
  members: Assignee[];
  /** 成员 ID → 当前负责的未关闭任务数 */
  load: Map<number, number>;
  onDone: () => void;
}) {
  const toast = useToast();
  const [targets, setTargets] = useState<Map<number, number | null>>(new Map());
  const [reasons, setReasons] = useState<Map<number, AssignSuggestion>>(new Map());
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState<"history" | "ai" | null>(null);
  const [all, setAll] = useState("");

  const ids = useMemo(() => issues.map((i) => i.id), [issues]);

  async function suggest(useAi: boolean) {
    setBusy(useAi ? "ai" : "history");
    try {
      const list = await api.suggestAssignments(projectId, ids, useAi);
      setReasons(new Map(list.map((s) => [s.issueId, s])));
      setTargets(new Map(list.map((s) => [s.issueId, s.assigneeId ?? null])));
      setAll("");
    } catch (e) {
      toast.show({ tone: "error", title: useAi ? "AI 推荐失败" : "推荐失败", body: errorText(e) });
    } finally {
      setBusy(null);
    }
  }

  useEffect(() => {
    if (!open) return;
    setTargets(new Map(issues.map((i) => [i.id, null])));
    setReasons(new Map());
    setNotes("");
    setAll("");
    // 打开时先用本地算法给出建议，不消耗 token
    void suggest(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, ids.join(",")]);

  const nameOf = (id: number | null | undefined) => members.find((m) => m.id === id)?.name ?? (id ? `成员 #${id}` : "");
  const changes = issues.filter((i) => {
    const t = targets.get(i.id);
    return t != null && t !== i.assigneeId;
  });

  function setAllTo(v: string) {
    setAll(v);
    if (!v) return;
    const id = Number(v);
    setTargets(new Map(issues.map((i) => [i.id, id])));
    setReasons(new Map());
  }

  function submit() {
    if (changes.length === 0) {
      toast.show({ tone: "warn", title: "没有需要转派的任务", body: "请为任务选择新的负责人" });
      return;
    }
    const items = changes.map((i) => ({ issueId: i.id, assigneeId: targets.get(i.id)! }));
    const people = [...new Set(items.map((x) => nameOf(x.assigneeId)))];
    onClose();
    toast.withUndo(`正在转派 ${items.length} 个任务给 ${people.slice(0, 3).join("、")}${people.length > 3 ? ` 等 ${people.length} 人` : ""}`, async () => {
      const outs = await api.bulkAssign(items, notes.trim() || null);
      const ok = outs.filter((o) => o.ok);
      const failed = outs.filter((o) => !o.ok);
      toast.show({
        tone: failed.length ? "warn" : "success",
        title: `已转派 ${ok.length} 个任务${failed.length ? `，${failed.length} 个失败` : ""}`,
        body: failed.map((f) => `#${f.issueId}：${errorText(f.error)}`).join("\n") || undefined,
      });
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

  const memberOptions = <MemberOptions members={members} load={load} />;

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={1000}
      title={`转派 ${issues.length} 个任务`}
      subtitle="每个任务可以指派给不同的人；已按类别默认负责人、相似任务和负载给出建议，可逐个调整"
      footer={
        <>
          <span className="mr-auto text-[12px] text-ink-3">
            将转派 {changes.length} 个{issues.length - changes.length > 0 ? `，${issues.length - changes.length} 个不变` : ""} · 提交后 5 秒内可撤销
          </span>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" icon={<Users className="size-4" />} disabled={changes.length === 0} onClick={submit}>
            转派 {changes.length} 个任务 <span className="opacity-70">{modKey}↵</span>
          </Button>
        </>
      }
    >
      <div className="space-y-4 p-5">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[12.5px] font-medium text-ink-2">全部指派给</span>
          <div className="w-52">
            <Select value={all} onChange={(e) => setAllTo(e.target.value)}>
              <option value="">选择成员…</option>
              {memberOptions}
            </Select>
          </div>
          <span className="mx-1 text-[12px] text-ink-3">或</span>
          <Button icon={<History className="size-3.5" />} loading={busy === "history"} onClick={() => void suggest(false)}>
            按历史推荐
          </Button>
          <Button variant="ai" icon={<Sparkles className="size-3.5" />} loading={busy === "ai"} onClick={() => void suggest(true)}>
            AI 推荐
          </Button>
        </div>

        <div className="max-h-[46vh] overflow-y-auto rounded-xl border border-line">
          <table className="w-full text-[12.5px]">
            <thead className="sticky top-0 z-[1] bg-sunken text-left text-[11.5px] text-ink-3">
              <tr>
                <th className="px-3 py-2 font-medium">任务</th>
                <th className="w-28 px-2 py-2 font-medium">当前负责人</th>
                <th className="w-6" />
                <th className="w-48 px-2 py-2 font-medium">转派给</th>
                <th className="w-[260px] px-3 py-2 font-medium">推荐理由</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {issues.map((i) => {
                const t = targets.get(i.id) ?? null;
                const r = reasons.get(i.id);
                const changed = t != null && t !== i.assigneeId;
                return (
                  <tr key={i.id} className={clsx(changed && "bg-accent-soft/30")}>
                    <td className="px-3 py-2">
                      <div className="flex items-center gap-1.5 text-[11.5px] text-ink-3">
                        <span className="font-mono">#{i.id}</span>
                        {i.category && <span>· {i.category}</span>}
                        {i.overdue && <Badge tone="accent">逾期</Badge>}
                      </div>
                      <div className="line-clamp-1 text-ink" title={i.subject}>
                        {i.subject}
                      </div>
                    </td>
                    <td className="px-2 py-2 text-ink-2">{i.assignee || <span className="text-warn">未指派</span>}</td>
                    <td>
                      <ArrowRight className={clsx("size-3.5", changed ? "text-accent" : "text-line-strong")} />
                    </td>
                    <td className="px-2 py-1.5">
                      <Select
                        value={t ?? ""}
                        onChange={(e) => {
                          const v = e.target.value ? Number(e.target.value) : null;
                          setTargets((m) => new Map(m).set(i.id, v));
                          setAll("");
                        }}
                        className="h-7 text-[12.5px]"
                      >
                        <option value="">不转派</option>
                        {memberOptions}
                      </Select>
                    </td>
                    <td className="px-3 py-2 text-[12px] text-ink-3">
                      {r && t === r.assigneeId ? (
                        <span className="line-clamp-2">
                          <Badge tone={sourceTone(r.source)} className="mr-1">
                            {SOURCE_LABEL[r.source]}
                          </Badge>
                          {r.reason}
                        </span>
                      ) : t != null && t === i.assigneeId ? (
                        "已是当前负责人"
                      ) : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>

        <div>
          <div className="mb-1 text-[12.5px] font-medium text-ink-2">转派说明（可选）</div>
          <Textarea
            rows={2}
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            placeholder="例如：请本周五前处理完；有问题找我。填写后会作为备注写入每个任务，被指派人会收到 Redmine 通知"
          />
        </div>
      </div>
    </Modal>
  );
}
