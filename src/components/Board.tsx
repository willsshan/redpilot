import { useMemo, useState, type DragEvent } from "react";
import clsx from "clsx";
import { CornerUpLeft, MessageSquareText } from "lucide-react";
import type { IssueView, NamedId } from "../lib/api";
import { categoryColor, dueLabel, priorityTone, urgencyColor } from "../lib/format";
import { Avatar, Badge, CategoryBadge } from "./ui";

export type GroupBy = "status" | "category" | "project" | "urgency";

interface Column {
  key: string;
  title: string;
  color: string;
  items: IssueView[];
  /** 可以把卡片拖进来 */
  droppable: boolean;
}

const DRAG_TYPE = "application/x-redpilot-issue";
const URGENCY = ["高", "中", "低"];

export function Board({
  issues,
  groupBy,
  statuses,
  categories,
  selectedId,
  onOpen,
  onMove,
  onResolve,
}: {
  issues: IssueView[];
  groupBy: GroupBy;
  /** 未关闭的状态，作为状态列 */
  statuses: NamedId[];
  categories: string[];
  selectedId: number | null;
  onOpen: (id: number) => void;
  /** 卡片被拖到另一列：状态分组时 key 是状态 ID，分类分组时是分类名 */
  onMove: (issue: IssueView, key: string) => void;
  onResolve: (issue: IssueView) => void;
}) {
  const columns = useMemo<Column[]>(() => {
    if (groupBy === "status") {
      const cols = statuses.map((s) => ({ key: String(s.id), title: s.name, color: "var(--info)", items: [] as IssueView[], droppable: true }));
      for (const i of issues) {
        let c = cols.find((x) => x.key === String(i.statusId));
        if (!c) {
          c = { key: String(i.statusId), title: i.statusName, color: "var(--info)", items: [], droppable: true };
          cols.push(c);
        }
        c.items.push(i);
      }
      return cols;
    }
    if (groupBy === "category") {
      const names = [...categories];
      if (issues.some((i) => !i.analysis.category)) names.push("未分类");
      const cols = names.map((n) => ({ key: n, title: n, color: categoryColor(n === "未分类" ? null : n), items: [] as IssueView[], droppable: true }));
      for (const i of issues) {
        const k = i.analysis.category ?? "未分类";
        let c = cols.find((x) => x.key === k);
        if (!c) {
          c = { key: k, title: k, color: categoryColor(k), items: [], droppable: true };
          cols.push(c);
        }
        c.items.push(i);
      }
      return cols;
    }
    if (groupBy === "project") {
      const m = new Map<number, Column>();
      for (const i of issues) {
        let c = m.get(i.projectId);
        if (!c) {
          c = { key: String(i.projectId), title: i.projectName, color: "var(--info)", items: [], droppable: false };
          m.set(i.projectId, c);
        }
        c.items.push(i);
      }
      return [...m.values()].sort((a, b) => b.items.length - a.items.length);
    }
    const cols = [...URGENCY, "未评估"].map((u) => ({
      key: u,
      title: u === "未评估" ? u : `紧急度 ${u}`,
      color: u === "未评估" ? "var(--ink-3)" : urgencyColor(u),
      items: [] as IssueView[],
      droppable: false,
    }));
    for (const i of issues) cols[URGENCY.includes(i.analysis.urgency ?? "") ? URGENCY.indexOf(i.analysis.urgency!) : 3].items.push(i);
    return cols;
  }, [issues, groupBy, statuses, categories]);

  const [dragging, setDragging] = useState<IssueView | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const draggable = groupBy === "status" || groupBy === "category";

  function dropOn(key: string, e: DragEvent) {
    e.preventDefault();
    setOverKey(null);
    const id = Number(e.dataTransfer.getData(DRAG_TYPE));
    const issue = issues.find((i) => i.id === id);
    setDragging(null);
    if (!issue) return;
    if (key === "__resolve") return onResolve(issue);
    const current = groupBy === "status" ? String(issue.statusId) : (issue.analysis.category ?? "未分类");
    if (current !== key) onMove(issue, key);
  }

  const dropHandlers = (key: string) => ({
    onDragOver: (e: DragEvent) => {
      if (![...e.dataTransfer.types].includes(DRAG_TYPE)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      if (overKey !== key) setOverKey(key);
    },
    onDragLeave: (e: DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOverKey((k) => (k === key ? null : k));
    },
    onDrop: (e: DragEvent) => dropOn(key, e),
  });

  return (
    <div className="relative h-full">
    <div className="flex h-full gap-3 overflow-x-auto p-4">
      {columns.map((col) => (
        <div
          key={col.key}
          {...(draggable && col.droppable ? dropHandlers(col.key) : {})}
          className={clsx(
            "flex max-w-[360px] min-w-[240px] flex-1 flex-col rounded-xl border bg-sunken/60 transition-colors",
            overKey === col.key ? "border-accent bg-accent-soft/40" : "border-line",
          )}
        >
          <div className="flex shrink-0 items-center gap-2 px-3 pt-3 pb-2">
            <span className="size-2 rounded-full" style={{ backgroundColor: col.color }} />
            <span className="text-[13px] font-semibold text-ink">{col.title}</span>
            <span className="text-[12px] text-ink-3 tabular-nums">{col.items.length}</span>
          </div>
          <div className="min-h-0 flex-1 space-y-2 overflow-y-auto px-2 pb-2">
            {col.items.map((i) => (
              <Card
                key={i.id}
                issue={i}
                groupBy={groupBy}
                active={i.id === selectedId}
                dragging={dragging?.id === i.id}
                draggable={draggable}
                onClick={() => onOpen(i.id)}
                onDragStart={(e) => {
                  e.dataTransfer.setData(DRAG_TYPE, String(i.id));
                  e.dataTransfer.effectAllowed = "move";
                  setDragging(i);
                }}
                onDragEnd={() => {
                  setDragging(null);
                  setOverKey(null);
                }}
              />
            ))}
            {col.items.length === 0 && (
              <div className="rounded-lg border border-dashed border-line py-6 text-center text-[12px] text-ink-3">{draggable ? "拖动任务到这里" : "暂无任务"}</div>
            )}
          </div>
        </div>
      ))}

    </div>

      {/* 拖动时出现的回派区 */}
      {dragging && (
        <div
          {...dropHandlers("__resolve")}
          className={clsx(
            "animate-in absolute bottom-5 left-1/2 flex h-16 w-[440px] -translate-x-1/2 items-center justify-center gap-3 rounded-2xl border-2 border-dashed shadow-pop backdrop-blur transition-colors",
            overKey === "__resolve" ? "border-accent bg-accent-soft text-accent-ink" : "border-accent/50 bg-panel/90 text-ink-2",
          )}
        >
          <CornerUpLeft className="size-5 text-accent" />
          <div>
            <div className="text-[13px] font-semibold">处理完成 · 回派给 {dragging.authorName || "提出人"}</div>
            <div className="text-[11.5px] text-ink-3">松开后填写处置说明</div>
          </div>
        </div>
      )}
    </div>
  );
}

function Card({
  issue: i,
  groupBy,
  active,
  dragging,
  draggable,
  onClick,
  onDragStart,
  onDragEnd,
}: {
  issue: IssueView;
  groupBy: GroupBy;
  active: boolean;
  dragging: boolean;
  draggable: boolean;
  onClick: () => void;
  onDragStart: (e: DragEvent) => void;
  onDragEnd: () => void;
}) {
  const a = i.analysis;
  const due = dueLabel(i.dueDate);
  const high = priorityTone(i.priorityName) === "high";
  return (
    <div
      draggable={draggable}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onClick}
      className={clsx(
        "relative cursor-default rounded-lg border bg-panel p-3 shadow-[0_1px_2px_rgb(0_0_0/0.04)] transition-[border-color,opacity,box-shadow] hover:shadow-card",
        active ? "border-accent/60 ring-2 ring-accent/15" : "border-line",
        dragging && "opacity-40",
      )}
    >
      {high && <span className="absolute top-3 bottom-3 left-0 w-[3px] rounded-r bg-accent" />}
      <div className="flex items-center gap-1.5 text-[11.5px] text-ink-3">
        {i.unread && <span className="size-1.5 rounded-full bg-accent" />}
        <span className="font-mono">#{i.id}</span>
        {groupBy !== "project" && <span className="truncate">· {i.projectName}</span>}
      </div>
      <div className={clsx("mt-1 line-clamp-2 text-[13px] leading-snug text-ink", i.unread ? "font-semibold" : "font-medium")}>{i.subject}</div>
      {i.changeHint && (
        <div className="mt-1.5 flex items-start gap-1 text-[12px] text-accent-ink">
          <MessageSquareText className="mt-0.5 size-3 shrink-0" />
          <span className="line-clamp-2">{i.changeHint}</span>
        </div>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {groupBy !== "category" && <CategoryBadge category={a.category} source={a.categorySource} />}
        {groupBy === "category" && a.module && <Badge>{a.module}</Badge>}
        {groupBy !== "status" && <Badge tone="info">{i.statusName}</Badge>}
        {a.missingInfo.length > 0 && <Badge tone="warn">缺信息</Badge>}
        {due && due.tone !== "muted" && <Badge tone={due.tone === "danger" ? "accent" : "warn"}>{due.text}</Badge>}
        {a.estHours != null && <span className="text-[11.5px] text-ink-3 tabular-nums">{a.estHours}h</span>}
        <span className="ml-auto">
          <Avatar name={i.authorName} size={18} />
        </span>
      </div>
    </div>
  );
}
