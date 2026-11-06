import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { Copy, Forward, GitFork, Hand, MessageCircleQuestion, Search, Sparkles, Star, X } from "lucide-react";
import type { AppConfig, DispatchItem, DispatchMember, DispatchOutcome, DispatchProject, DispatchRequest, SplitPart } from "../lib/api";
import { modKey } from "../lib/format";
import { Avatar, Badge, Button, Kbd } from "./ui";
import { SOURCE_LABEL, sourceTone } from "./AssignDialog";

type Mode = "split" | "assist";

const uniq = (ids: (number | null | undefined)[]) => [...new Set(ids.filter((x): x is number => typeof x === "number"))];

/** 推荐的人选：多人方案的各负责人，或单人推荐 */
function recommended(item: DispatchItem): number[] {
  if (item.multi && item.multi.parts.length >= 2) return uniq(item.multi.parts.map((p) => p.assigneeId));
  return item.suggestion?.assigneeId ? [item.suggestion.assigneeId] : [];
}

/** 按推荐生成的派单请求（批量派单用），没有推荐时返回 null */
export function recommendedRequest(item: DispatchItem): DispatchRequest | null {
  const base = { issueId: item.issue.id, autoDue: true, suggestedId: item.suggestion?.assigneeId ?? null };
  if (item.multi && item.multi.parts.length >= 2) return { ...base, mode: "split", parts: item.multi.parts };
  if (item.suggestion?.assigneeId) return { ...base, mode: "single", assigneeId: item.suggestion.assigneeId };
  return null;
}

/** 派单结果的提示内容 */
export function outcomeToast(out: DispatchOutcome): { title: string; body?: string; warn: boolean } {
  const title =
    out.mode === "split"
      ? `#${out.issueId} 已拆成 ${out.created.length} 个子任务`
      : `#${out.issueId} 已派给 ${out.assignedTo[0] ?? "目标成员"}${out.assignedTo.length > 1 ? `，${out.assignedTo.slice(1).join("、")} 协作` : ""}`;
  const lines = [...out.created.map((c) => `#${c.id} ${c.subject} → ${c.assignee || "未指派"}`), ...out.warnings];
  return { title, body: lines.join("\n") || undefined, warn: out.warnings.length > 0 };
}

/** 备注模板：{负责人} {提出人} {截止日期} */
function expand(template: string, who: string, author: string, due: string): string {
  if (!template.trim()) return "";
  const d = due ? `${Number(due.slice(5, 7))} 月 ${Number(due.slice(8, 10))} 日` : "约定时间";
  return template.replaceAll("{负责人}", who).replaceAll("{提出人}", author || "提出人").replaceAll("{截止日期}", d);
}

/**
 * 派单栏：选人（数字键 / 推荐 / 搜索）、多人时选择拆成子任务或一人负责加协作，设置截止日期、关注和备注。
 * `active` 时响应快捷键：数字键直接派给对应成员，⇧+数字增减人选，↵ 派出，A 搜索，Tab 写备注，Q/S/D/E 其他操作。
 */
export function DispatchPanel({
  item,
  project,
  config,
  meId,
  active,
  customParts,
  onClearCustom,
  onSubmit,
  onFollowUp,
  onEditSplit,
  onDuplicate,
  onKeep,
  onAi,
  aiBusy,
}: {
  item: DispatchItem;
  project?: DispatchProject;
  config: AppConfig | null;
  meId?: number | null;
  active: boolean;
  /** 在拆分编辑里改过的子任务方案 */
  customParts?: SplitPart[] | null;
  onClearCustom?: () => void;
  onSubmit: (req: DispatchRequest, title: string) => void;
  onFollowUp?: () => void;
  onEditSplit?: (parts: SplitPart[]) => void;
  onDuplicate?: () => void;
  onKeep?: () => void;
  onAi?: () => void;
  aiBusy?: boolean;
}) {
  const v = item.issue;
  const members = useMemo(() => project?.members ?? [], [project]);
  const [selected, setSelected] = useState<number[]>(() => (customParts?.length ? uniq(customParts.map((p) => p.assigneeId)) : recommended(item)));
  const [mode, setMode] = useState<Mode>("split");
  const [due, setDue] = useState(v.dueDate ?? item.defaultDue ?? "");
  const [dueOn, setDueOn] = useState(!!(v.dueDate || item.defaultDue));
  const [watch, setWatch] = useState(config?.dispatch.watch ?? true);
  const [note, setNote] = useState<string | null>(null);
  const [frontendWaits, setFrontendWaits] = useState(false);
  const [search, setSearch] = useState<string | null>(null);
  const noteRef = useRef<HTMLInputElement>(null);

  // 拆分编辑改完方案后，人选跟着方案走
  useEffect(() => {
    if (customParts?.length) {
      setSelected(uniq(customParts.map((p) => p.assigneeId)));
      setMode("split");
    }
  }, [customParts]);

  const byId = (id: number) => members.find((m) => m.id === id);
  const nameOf = (id: number) => byId(id)?.name ?? (id === meId ? "我" : `成员 #${id}`);
  const keyed = members.filter((m) => m.key != null);
  const multiParts = item.multi?.parts ?? [];
  const suggestedId = item.suggestion?.assigneeId ?? null;
  const recIds = recommended(item);

  // 当前的子任务方案：编辑过的 > 推荐的多人方案（人选一致时）> 按人选生成
  const parts: SplitPart[] = useMemo(() => {
    if (customParts?.length) return customParts;
    return selected.map((id) => {
      const p = multiParts.find((x) => x.assigneeId === id);
      if (p) return p;
      const role = byId(id)?.role ?? "";
      return {
        subject: `${v.subject.slice(0, 50)}（${role || nameOf(id)}）`,
        description: role ? `负责${role}部分，详见原任务 #${v.id}。` : `详见原任务 #${v.id}。`,
        assigneeId: id,
        role,
      };
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [customParts, selected, multiParts, members]);

  const effMode: "single" | Mode = selected.length <= 1 && !customParts?.length ? "single" : mode;
  const roles = new Set(parts.map((p) => p.role).filter(Boolean));
  const canWait = effMode === "split" && roles.has("前端") && roles.has("后端");
  const who = effMode === "split" ? parts.map((p) => (p.assigneeId ? nameOf(p.assigneeId) : "未指派")) : selected.map(nameOf);
  const expanded = expand(config?.dispatch.noteTemplate ?? "", [...new Set(who)].join("、"), v.authorName, dueOn ? due : "");
  const noteValue = note ?? expanded;

  function plan(ids: number[] = selected, m: "single" | Mode = effMode): { req: DispatchRequest; title: string } | null {
    if (ids.length === 0 && !(m === "split" && customParts?.length)) return null;
    const dueDate = dueOn && due && due !== v.dueDate ? due : null;
    const base: DispatchRequest = {
      issueId: v.id,
      mode: "single",
      notes: noteValue.trim() || null,
      dueDate,
      watch,
      suggestedId,
    };
    if (m === "single" || (ids.length === 1 && !(m === "split" && customParts?.length))) {
      return { req: { ...base, mode: "single", assigneeId: ids[0] }, title: `派给 ${nameOf(ids[0])}` };
    }
    if (m === "assist") {
      const [lead, ...rest] = ids;
      return { req: { ...base, mode: "assist", assigneeId: lead, collaboratorIds: rest }, title: `派给 ${nameOf(lead)}，${rest.map(nameOf).join("、")} 协作` };
    }
    return {
      req: { ...base, mode: "split", parts, frontendWaits: canWait && frontendWaits },
      title: `拆成 ${parts.length} 个子任务：${parts.map((p) => `${p.assigneeId ? nameOf(p.assigneeId) : "未指派"}${p.role ? `（${p.role}）` : ""}`).join(" + ")}`,
    };
  }

  const current = plan();

  function submit(ids?: number[], m?: "single" | Mode) {
    const p = plan(ids, m);
    if (p) onSubmit(p.req, p.title);
  }

  function choose(id: number, additive: boolean) {
    onClearCustom?.();
    setSelected((s) => (additive ? (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]) : [id]));
  }

  useEffect(() => {
    if (!active) return;
    const h = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT") return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const digit = /^Digit([1-9])$/.exec(e.code);
      if (digit) {
        const m = keyed.find((x) => x.key === Number(digit[1]));
        if (!m) return;
        e.preventDefault();
        if (e.shiftKey) choose(m.id, true);
        else submit([m.id], "single");
        return;
      }
      if (e.shiftKey) return;
      if (e.key === "Enter") {
        e.preventDefault();
        submit();
      } else if (e.key === "a") {
        e.preventDefault();
        setSearch("");
      } else if (e.key === "Tab") {
        e.preventDefault();
        noteRef.current?.focus();
      } else if (e.key === "q" && onFollowUp) {
        e.preventDefault();
        onFollowUp();
      } else if (e.key === "s" && onEditSplit) {
        e.preventDefault();
        onEditSplit(parts.length >= 2 ? parts : multiParts);
      } else if (e.key === "d" && onDuplicate) {
        e.preventDefault();
        onDuplicate();
      } else if (e.key === "e" && onKeep) {
        e.preventDefault();
        onKeep();
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  });

  const sameAsRec = recIds.length > 0 && recIds.length === selected.length && recIds.every((id) => selected.includes(id)) && !customParts?.length;
  const reason = sameAsRec ? (item.multi && recIds.length >= 2 ? item.multi.reason : item.suggestion?.reason) : null;
  const reasonSource = item.multi && recIds.length >= 2 ? item.multi.source : item.suggestion?.source;

  return (
    <div className="shrink-0 space-y-2.5 border-t border-line bg-panel/90 px-5 py-3 backdrop-blur">
      {/* 推荐与其他操作 */}
      <div className="flex items-center gap-2">
        <div className="flex min-w-0 flex-1 items-center gap-1.5 text-[12.5px]">
          {current ? (
            <>
              {reason && reasonSource && <Badge tone={sourceTone(reasonSource)}>{reasonSource === "ai" ? "AI" : `推荐 · ${SOURCE_LABEL[reasonSource]}`}</Badge>}
              {customParts?.length ? <Badge tone="info">已编辑的方案</Badge> : null}
              <span className="min-w-0 truncate font-medium text-ink" title={reason ? `${current.title}\n${reason}` : current.title}>
                {current.title}
                {reason && <span className="font-normal text-ink-3"> · {reason}</span>}
              </span>
              {!sameAsRec && recIds.length > 0 && (
                <button className="shrink-0 text-accent hover:underline" onClick={() => (onClearCustom?.(), setSelected(recIds))}>
                  恢复推荐（{recIds.map(nameOf).join(" + ")}）
                </button>
              )}
            </>
          ) : (
            <span className="text-ink-3">{project?.error ?? "没有推荐人选，请按数字键或 A 选择成员"}</span>
          )}
        </div>
        {onAi && (
          <Button size="sm" variant="ghost" icon={<Sparkles className="size-3.5 text-accent" />} loading={aiBusy} onClick={onAi} title="让 AI 结合成员分工、正在负责的任务和负载重新推荐">
            AI 推荐
          </Button>
        )}
      </div>

      {/* 成员 */}
      <div className="flex flex-wrap items-center gap-1.5">
        {keyed.map((m) => (
          <MemberChip
            key={m.id}
            m={m}
            selected={selected.includes(m.id)}
            recommended={recIds.includes(m.id)}
            tag={effMode === "assist" && selected.includes(m.id) ? (selected[0] === m.id ? "负责" : "协作") : undefined}
            onClick={(additive) => choose(m.id, additive)}
          />
        ))}
        {selected
          .filter((id) => !keyed.some((m) => m.id === id))
          .map((id) => {
            const m = byId(id);
            return m ? <MemberChip key={id} m={m} selected recommended={recIds.includes(id)} onClick={(additive) => choose(id, additive)} /> : null;
          })}
        <div className="relative">
          <button
            onClick={() => setSearch(search === null ? "" : null)}
            className="flex h-8 items-center gap-1.5 rounded-lg border border-dashed border-line-strong px-2.5 text-[12.5px] text-ink-2 hover:bg-hover"
          >
            <Search className="size-3.5" />
            其他人
            <Kbd>A</Kbd>
          </button>
          {search !== null && (
            <MemberSearch
              members={members}
              meId={meId}
              query={search}
              onQuery={setSearch}
              onPick={(id, additive) => {
                choose(id, additive);
                setSearch(null);
              }}
              onClose={() => setSearch(null)}
            />
          )}
        </div>
      </div>

      {/* 多人 */}
      {effMode !== "single" && (
        <div className="rounded-lg border border-line bg-sunken/50 px-3 py-2">
          <div className="flex flex-wrap items-center gap-3 text-[12.5px]">
            <span className="text-ink-3">方式</span>
            <Radio checked={mode === "split"} onChange={() => setMode("split")} label="拆成子任务，每人一个" />
            <Radio
              checked={mode === "assist"}
              onChange={() => (onClearCustom?.(), setMode("assist"))}
              label={`一人负责（${nameOf(selected[0] ?? 0)}），其他人协作`}
            />
            {canWait && (
              <label className="flex items-center gap-1.5 text-ink-2" title="后端子任务“阻挡”前端子任务，前端能看到自己在等哪个任务">
                <input type="checkbox" checked={frontendWaits} onChange={(e) => setFrontendWaits(e.target.checked)} className="accent-accent" />
                前端等后端完成
              </label>
            )}
          </div>
          {mode === "split" || customParts?.length ? (
            <ol className="mt-1.5 space-y-0.5 text-[12.5px]">
              {parts.map((p, i) => (
                <li key={i} className="flex items-center gap-2">
                  <span className="w-4 text-right text-ink-3 tabular-nums">{i + 1}.</span>
                  <span className="min-w-0 flex-1 truncate text-ink" title={p.description}>
                    {p.subject}
                  </span>
                  <span className="shrink-0 text-ink-2">→ {p.assigneeId ? nameOf(p.assigneeId) : "未指派"}</span>
                </li>
              ))}
              <li className="pl-6 text-[11.5px] text-ink-3">原任务留在你名下跟踪，并写一条拆分说明；按 S 可以修改子任务的标题和内容</li>
            </ol>
          ) : (
            <div className="mt-1 text-[11.5px] text-ink-3">协作人会被加为关注人，备注里写明由谁负责、谁协作</div>
          )}
        </div>
      )}

      {/* 截止、关注、备注 */}
      <div className="flex items-center gap-3 text-[12.5px] text-ink-2">
        <label className="flex shrink-0 items-center gap-1.5" title={v.dueDate ? "任务已有截止日期" : "任务没有截止日期时，按优先级给出默认值（设置 → 分派）"}>
          {!v.dueDate && <input type="checkbox" checked={dueOn} onChange={(e) => setDueOn(e.target.checked)} className="accent-accent" />}
          截止
          <input
            type="date"
            value={due}
            disabled={!dueOn}
            onChange={(e) => (setDue(e.target.value), setDueOn(!!e.target.value))}
            className="h-7 rounded-md border border-line bg-panel px-1.5 text-[12.5px] text-ink disabled:opacity-50"
          />
        </label>
        <label className="flex shrink-0 items-center gap-1.5" title="派出去后进度和退回都能收到通知">
          <input type="checkbox" checked={watch} onChange={(e) => setWatch(e.target.checked)} className="accent-accent" />
          加我为关注人
        </label>
        <div className="relative min-w-0 flex-1">
          <input
            ref={noteRef}
            value={noteValue}
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                e.currentTarget.blur();
                submit();
              } else if (e.key === "Escape") {
                e.stopPropagation();
                e.currentTarget.blur();
              }
            }}
            placeholder="备注（可选），被指派人会收到通知"
            className="h-8 w-full rounded-lg border border-line bg-panel pr-12 pl-2.5 text-[12.5px] text-ink placeholder:text-ink-3 focus:border-accent focus:outline-none"
          />
          <span className="absolute top-1/2 right-2 -translate-y-1/2">
            <Kbd>Tab</Kbd>
          </span>
        </div>
        <Button variant="primary" icon={effMode === "split" ? <GitFork className="size-4" /> : <Forward className="size-4" />} disabled={!current} onClick={() => submit()}>
          {effMode === "split" ? `拆成 ${parts.length} 个子任务` : selected.length ? `派给 ${nameOf(selected[0])}` : "派出"}
          <span className="opacity-70">↵</span>
        </Button>
      </div>

      {/* 其他操作与快捷键提示 */}
      {(onFollowUp || onEditSplit || onDuplicate || onKeep) && (
        <div className="-mb-1 flex items-center gap-1 text-[12px]">
          {onFollowUp && <SmallAction icon={<MessageCircleQuestion className="size-3.5" />} label="追问提出人" k="Q" onClick={onFollowUp} />}
          {onEditSplit && <SmallAction icon={<GitFork className="size-3.5" />} label="拆分编辑" k="S" onClick={() => onEditSplit(parts.length >= 2 ? parts : multiParts)} />}
          {onDuplicate && <SmallAction icon={<Copy className="size-3.5" />} label="标为重复" k="D" onClick={onDuplicate} />}
          {onKeep && <SmallAction icon={<Hand className="size-3.5" />} label="留给自己" k="E" onClick={onKeep} />}
          <span className="ml-auto text-[11.5px] text-ink-3">数字键直接派出 · ⇧+数字多选 · Tab 写备注</span>
        </div>
      )}
    </div>
  );
}

function SmallAction({ icon, label, k, onClick }: { icon: ReactNode; label: string; k: string; onClick: () => void }) {
  return (
    <button onClick={onClick} className="flex h-7 shrink-0 items-center gap-1 rounded-md px-1.5 text-[12px] text-ink-2 hover:bg-hover hover:text-ink">
      {icon}
      {label}
      <Kbd>{k}</Kbd>
    </button>
  );
}

function Radio({ checked, onChange, label }: { checked: boolean; onChange: () => void; label: string }) {
  return (
    <label className="flex items-center gap-1.5 text-ink-2">
      <input type="radio" checked={checked} onChange={onChange} className="accent-accent" />
      {label}
    </label>
  );
}

function MemberChip({
  m,
  selected,
  recommended,
  tag,
  onClick,
}: {
  m: DispatchMember;
  selected: boolean;
  recommended: boolean;
  tag?: string;
  onClick: (additive: boolean) => void;
}) {
  return (
    <button
      onClick={(e) => onClick(e.shiftKey || e.metaKey || e.ctrlKey)}
      title={`${m.name}${m.role ? ` · ${m.role}${m.roleGuessed ? "（按历史推断）" : ""}` : ""} · 当前 ${m.load} 个未关闭${m.key ? `\n按 ${m.key} 直接派出，⇧+${m.key} 加入多选` : ""}`}
      className={clsx(
        "flex h-8 items-center gap-1.5 rounded-lg border pr-2 pl-1.5 text-[12.5px] transition-colors",
        selected ? "border-accent bg-accent-soft text-accent-ink" : "border-line bg-panel text-ink hover:bg-hover",
      )}
    >
      {m.key != null && <Kbd>{m.key}</Kbd>}
      <Avatar name={m.name} size={18} />
      <span className="font-medium">{m.name}</span>
      {m.role && <span className={clsx("text-[11px]", m.roleGuessed ? "text-ink-3" : "text-ink-2")}>{m.role}</span>}
      <span className={clsx("text-[11px] tabular-nums", m.load >= 8 ? "text-warn" : "text-ink-3")}>{m.load}</span>
      {recommended && <Star className="size-3 fill-current text-warn" />}
      {tag && <Badge tone={selected ? "accent" : "muted"}>{tag}</Badge>}
    </button>
  );
}

function MemberSearch({
  members,
  meId,
  query,
  onQuery,
  onPick,
  onClose,
}: {
  members: DispatchMember[];
  meId?: number | null;
  query: string;
  onQuery: (q: string) => void;
  onPick: (id: number, additive: boolean) => void;
  onClose: () => void;
}) {
  const [idx, setIdx] = useState(0);
  const q = query.trim().toLowerCase();
  const list = members.filter((m) => !q || m.name.toLowerCase().includes(q) || m.role.includes(q) || m.roles.some((r) => r.toLowerCase().includes(q)));
  return (
    <div className="absolute bottom-full left-0 z-20 mb-1.5 w-72 overflow-hidden rounded-xl border border-line bg-panel shadow-pop">
      <div className="flex items-center gap-2 border-b border-line px-2.5">
        <Search className="size-3.5 text-ink-3" />
        <input
          autoFocus
          value={query}
          onChange={(e) => (onQuery(e.target.value), setIdx(0))}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") (e.preventDefault(), setIdx((i) => Math.min(list.length - 1, i + 1)));
            else if (e.key === "ArrowUp") (e.preventDefault(), setIdx((i) => Math.max(0, i - 1)));
            else if (e.key === "Enter" && list[idx] && !e.nativeEvent.isComposing) {
              e.preventDefault();
              e.currentTarget.blur();
              onPick(list[idx].id, e.shiftKey);
            } else if (e.key === "Escape") {
              e.stopPropagation();
              onClose();
            }
          }}
          placeholder="搜索姓名或分工"
          className="h-9 flex-1 bg-transparent text-[12.5px] focus:outline-none"
        />
        <button onClick={onClose} className="rounded p-0.5 text-ink-3 hover:bg-hover">
          <X className="size-3.5" />
        </button>
      </div>
      <ul className="max-h-64 overflow-y-auto py-1">
        {list.length === 0 && <li className="px-3 py-2 text-[12.5px] text-ink-3">没有匹配的成员</li>}
        {list.map((m, i) => (
          <li key={m.id}>
            <button
              onMouseEnter={() => setIdx(i)}
              onClick={(e) => onPick(m.id, e.shiftKey)}
              className={clsx("flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12.5px]", i === idx ? "bg-hover" : "")}
            >
              <Avatar name={m.name} size={18} />
              <span className="flex-1 truncate">
                {m.name}
                {m.id === meId && <span className="text-ink-3">（我）</span>}
                {m.isGroup && <span className="text-ink-3">（组）</span>}
              </span>
              {m.role && <span className="text-[11px] text-ink-3">{m.role}</span>}
              <span className="text-[11px] text-ink-3 tabular-nums">{m.load}</span>
            </button>
          </li>
        ))}
      </ul>
      <div className="border-t border-line px-3 py-1.5 text-[11px] text-ink-3">↵ 选择，⇧↵ 或 {modKey}/⇧ 点击加入多选</div>
    </div>
  );
}
