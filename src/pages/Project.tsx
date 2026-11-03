import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import clsx from "clsx";
import { AlertTriangle, ChevronDown, FolderKanban, GitFork, Hourglass, Inbox, Milestone, RefreshCw, Search, TrendingUp, UserX, Users, X } from "lucide-react";
import { api, errorText, type IssueDetail as Detail, type ProjectAccess, type ProjectIssue, type ProjectOverview } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { dueLabel, isMac, priorityTone, relativeTime } from "../lib/format";
import { Avatar, Badge, Button, Card, Empty, SectionTitle, Spinner } from "../components/ui";
import { IssueDetail } from "../components/IssueDetail";
import { FollowUpDialog, ResolveDialog } from "../components/ResolveDialog";
import { AssignDialog } from "../components/AssignDialog";
import { SplitDialog } from "../components/SplitDialog";

type Quick = "unassigned" | "overdue" | "stale" | "high" | null;
type SortBy = "updated" | "created" | "due" | "priority";

const STALE_DAYS = 14;
const PAGE = 200;
/** 未指派在负责人筛选中的取值 */
const UNASSIGNED = -1;

function storedProject(): number | null {
  try {
    const v = Number(localStorage.getItem("rp.project.id"));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

export function ProjectPage() {
  const { config, secrets, setPage } = useApp();
  const toast = useToast();
  const [projects, setProjects] = useState<ProjectAccess[] | null>(null);
  const [projectId, setProjectId] = useState<number | null>(storedProject);
  const [ov, setOv] = useState<ProjectOverview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [keyword, setKeyword] = useState("");
  const [assignee, setAssignee] = useState<number | null>(null);
  const [status, setStatus] = useState("");
  const [version, setVersion] = useState<number | null>(null);
  const [quick, setQuick] = useState<Quick>(null);
  const [sortBy, setSortBy] = useState<SortBy>("updated");
  const [shown, setShown] = useState(PAGE);
  const [checked, setChecked] = useState<Set<number>>(new Set());
  const [assignOpen, setAssignOpen] = useState(false);

  const [openId, setOpenId] = useState<number | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [resolveOpen, setResolveOpen] = useState(false);
  const [followUp, setFollowUp] = useState(false);
  const [splitFor, setSplitFor] = useState<number | null>(null);

  const configured = !!config?.redmine.baseUrl && secrets.redmine;

  useEffect(() => {
    if (!configured) return;
    api
      .myProjects()
      .then((ps) => {
        setProjects(ps);
        setProjectId((cur) => (cur && ps.some((p) => p.id === cur) ? cur : (ps[0]?.id ?? null)));
      })
      .catch((e) => {
        setProjects([]);
        setError(errorText(e));
      });
  }, [configured]);

  const load = useCallback(
    async (refresh = false) => {
      if (projectId === null) return;
      setLoading(true);
      setError(null);
      try {
        setOv(await api.projectOverview(projectId, refresh));
      } catch (e) {
        setError(errorText(e));
      } finally {
        setLoading(false);
      }
    },
    [projectId],
  );

  useEffect(() => {
    setOv(null);
    setChecked(new Set());
    setAssignee(null);
    setStatus("");
    setVersion(null);
    setQuick(null);
    setOpenId(null);
    if (projectId !== null) {
      try {
        localStorage.setItem("rp.project.id", String(projectId));
      } catch {
        // 只影响记住上次的项目
      }
    }
    void load();
  }, [projectId, load]);

  const issues = useMemo(() => ov?.issues ?? [], [ov]);

  const load_ = useMemo(() => {
    const m = new Map<number, number>();
    for (const i of issues) if (i.assigneeId != null) m.set(i.assigneeId, (m.get(i.assigneeId) ?? 0) + 1);
    return m;
  }, [issues]);

  const workload = useMemo(() => {
    const m = new Map<number, { id: number; name: string; open: number; overdue: number; stale: number }>();
    for (const i of issues) {
      const id = i.assigneeId ?? UNASSIGNED;
      const w = m.get(id) ?? { id, name: i.assignee || "未指派", open: 0, overdue: 0, stale: 0 };
      w.open += 1;
      w.overdue += Number(i.overdue);
      w.stale += Number(i.staleDays >= STALE_DAYS);
      m.set(id, w);
    }
    return [...m.values()].sort((a, b) => (a.id === UNASSIGNED ? -1 : b.id === UNASSIGNED ? 1 : b.open - a.open));
  }, [issues]);

  const statuses = useMemo(() => [...new Set(issues.map((i) => i.status))], [issues]);

  const filtered = useMemo(() => {
    const k = keyword.trim().toLowerCase();
    const list = issues.filter((i) => {
      if (assignee !== null && (i.assigneeId ?? UNASSIGNED) !== assignee) return false;
      if (status && i.status !== status) return false;
      if (version !== null && (i.versionId ?? -1) !== version) return false;
      if (quick === "unassigned" && i.assigneeId != null) return false;
      if (quick === "overdue" && !i.overdue) return false;
      if (quick === "stale" && i.staleDays < STALE_DAYS) return false;
      if (quick === "high" && priorityTone(i.priority) !== "high") return false;
      if (k && !(`#${i.id} ${i.subject} ${i.assignee} ${i.author} ${i.category ?? ""}`.toLowerCase().includes(k))) return false;
      return true;
    });
    const by: Record<SortBy, (a: ProjectIssue, b: ProjectIssue) => number> = {
      updated: (a, b) => b.updatedOn.localeCompare(a.updatedOn),
      created: (a, b) => b.createdOn.localeCompare(a.createdOn) || b.id - a.id,
      due: (a, b) => (a.dueDate ?? "9999").localeCompare(b.dueDate ?? "9999"),
      priority: (a, b) => b.priorityId - a.priorityId || b.updatedOn.localeCompare(a.updatedOn),
    };
    return list.sort(by[sortBy]);
  }, [issues, keyword, assignee, status, version, quick, sortBy]);

  useEffect(() => setShown(PAGE), [filtered]);

  // 打开任务详情（抽屉）
  useEffect(() => {
    if (openId === null) {
      setDetail(null);
      return;
    }
    let alive = true;
    setDetailLoading(true);
    api
      .issueDetail(openId, true)
      .then((d) => alive && setDetail(d))
      .catch((e) => alive && toast.show({ tone: "error", title: `加载 #${openId} 失败`, body: errorText(e) }))
      .finally(() => alive && setDetailLoading(false));
    return () => {
      alive = false;
    };
  }, [openId, toast]);

  function refreshDetail() {
    const id = openId;
    if (id === null) return;
    setOpenId(null);
    window.setTimeout(() => setOpenId(id), 0);
  }

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape" && openId !== null && !resolveOpen && !followUp && !assignOpen && splitFor === null) setOpenId(null);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [openId, resolveOpen, followUp, assignOpen, splitFor]);

  if (!configured) {
    return (
      <Empty
        icon={<FolderKanban className="size-6" />}
        title="连接 Redmine 后查看项目总览"
        desc="项目管理员可以在这里把控整个项目的进度，并批量转派任务。"
        action={
          <Button variant="primary" onClick={() => setPage("settings")}>
            前往设置
          </Button>
        }
      />
    );
  }

  const current = projects?.find((p) => p.id === projectId);
  const weekly = ov?.weekly ?? [];
  const thisWeek = weekly[weekly.length - 1];
  const maxWeek = Math.max(1, ...weekly.flatMap((w) => [w.created, w.closed]));
  const maxLoad = Math.max(1, ...workload.map((w) => w.open));
  const selectedIssues = issues.filter((i) => checked.has(i.id));
  const allChecked = filtered.length > 0 && filtered.every((i) => checked.has(i.id));
  const count = (q: Exclude<Quick, null>) =>
    issues.filter((i) =>
      q === "unassigned" ? i.assigneeId == null : q === "overdue" ? i.overdue : q === "stale" ? i.staleDays >= STALE_DAYS : priorityTone(i.priority) === "high",
    ).length;

  return (
    <div className="relative flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center gap-3 border-b border-line px-6", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">
          项目总览
        </h2>
        {projects && projects.length > 0 && (
          <label className="relative inline-flex h-8 items-center gap-1.5 rounded-lg border border-line bg-panel pr-7 pl-2.5 text-[13px] font-medium hover:bg-hover">
            <select
              value={projectId ?? ""}
              onChange={(e) => setProjectId(Number(e.target.value))}
              className="absolute inset-0 cursor-default opacity-0"
            >
              {projects.some((p) => p.isManager) && (
                <optgroup label="我管理的项目">
                  {projects
                    .filter((p) => p.isManager)
                    .map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                </optgroup>
              )}
              {projects.some((p) => !p.isManager) && (
                <optgroup label="我参与的项目">
                  {projects
                    .filter((p) => !p.isManager)
                    .map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                </optgroup>
              )}
            </select>
            <span className="max-w-[240px] truncate">{current?.name ?? "选择项目"}</span>
            {current?.isManager && <Badge tone="accent">管理员</Badge>}
            <ChevronDown className="pointer-events-none absolute right-2 size-3.5 text-ink-3" />
          </label>
        )}
        {current && !current.isManager && <span className="text-[12px] text-ink-3">你在该项目的角色：{current.roles.join("、") || "成员"}</span>}
        <div className="flex-1" data-tauri-drag-region />
        {ov && <span className="text-[12px] text-ink-3">数据更新于 {relativeTime(ov.fetchedAt)}</span>}
        <Button icon={<RefreshCw className={clsx("size-3.5", loading && "animate-spin")} />} disabled={loading || projectId === null} onClick={() => void load(true)}>
          刷新
        </Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {projects && projects.length === 0 ? (
          <Empty icon={<FolderKanban className="size-6" />} title="没有可查看的项目" desc={error ?? "你还没有加入任何 Redmine 项目。"} />
        ) : error && !ov ? (
          <Empty icon={<X className="size-6" />} title="无法读取项目数据" desc={error} action={<Button onClick={() => void load(true)}>重试</Button>} />
        ) : !ov ? (
          <div className="flex h-full items-center justify-center gap-2 text-[13px] text-ink-3">
            <Spinner /> 正在读取项目的全部任务…
          </div>
        ) : (
          <div className="mx-auto max-w-[1280px] space-y-5 p-6 pb-24">
            {/* 关键指标 */}
            <div className="grid grid-cols-6 gap-3">
              <Kpi icon={<Inbox className="size-4" />} label="未关闭" value={issues.length} active={quick === null && assignee === null} onClick={() => (setQuick(null), setAssignee(null))} />
              <Kpi icon={<UserX className="size-4" />} label="未指派" value={count("unassigned")} tone="warn" active={quick === "unassigned"} onClick={() => setQuick(quick === "unassigned" ? null : "unassigned")} />
              <Kpi icon={<AlertTriangle className="size-4" />} label="已逾期" value={count("overdue")} tone="accent" active={quick === "overdue"} onClick={() => setQuick(quick === "overdue" ? null : "overdue")} />
              <Kpi icon={<Hourglass className="size-4" />} label={`${STALE_DAYS} 天未更新`} value={count("stale")} tone="info" active={quick === "stale"} onClick={() => setQuick(quick === "stale" ? null : "stale")} />
              <Kpi icon={<TrendingUp className="size-4" />} label="本周新增" value={thisWeek?.created ?? 0} />
              <Kpi icon={<TrendingUp className="size-4 rotate-180" />} label="本周关闭" value={thisWeek?.closed ?? 0} tone="ok" />
            </div>
            {ov.truncated && <div className="rounded-lg bg-warn-soft px-3 py-2 text-[12.5px] text-warn">未关闭任务超过 2000 个，只统计了最近更新的 2000 个。</div>}

            <div className="grid grid-cols-3 gap-4">
              {/* 版本进度 */}
              <Card className="p-4">
                <SectionTitle icon={<Milestone className="size-3.5" />}>版本进度</SectionTitle>
                {ov.versions.length === 0 ? (
                  <div className="py-8 text-center text-[12.5px] text-ink-3">项目没有进行中的版本</div>
                ) : (
                  <div className="space-y-3">
                    {ov.versions.map((v) => {
                      const total = v.open + v.closed;
                      const pct = total ? Math.round((v.closed / total) * 100) : 0;
                      const due = dueLabel(v.dueDate);
                      return (
                        <button key={v.id} className="block w-full text-left" onClick={() => setVersion(version === v.id ? null : v.id)}>
                          <div className="flex items-center gap-2 text-[12.5px]">
                            <span className={clsx("min-w-0 flex-1 truncate font-medium", version === v.id ? "text-accent" : "text-ink")}>{v.name}</span>
                            {v.dueDate && <span className={clsx("shrink-0 text-[11.5px]", v.overdue ? "text-accent" : "text-ink-3")}>{v.overdue ? due?.text : v.dueDate}</span>}
                            <span className="shrink-0 text-[11.5px] text-ink-3 tabular-nums">{pct}%</span>
                          </div>
                          <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-sunken">
                            <div className={clsx("h-full rounded-full", v.overdue ? "bg-accent" : "bg-ok")} style={{ width: `${pct}%` }} />
                          </div>
                          <div className="mt-0.5 text-[11px] text-ink-3">
                            已关闭 {v.closed} · 未关闭 {v.open}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                )}
              </Card>

              {/* 成员负载 */}
              <Card className="p-4">
                <SectionTitle icon={<Users className="size-3.5" />} right={assignee !== null && <button className="text-[11.5px] text-accent" onClick={() => setAssignee(null)}>清除筛选</button>}>
                  成员负载
                </SectionTitle>
                <div className="max-h-[236px] space-y-1 overflow-y-auto pr-1">
                  {workload.map((w) => (
                    <button
                      key={w.id}
                      onClick={() => setAssignee(assignee === w.id ? null : w.id)}
                      className={clsx("flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left text-[12.5px]", assignee === w.id ? "bg-accent-soft/60" : "hover:bg-hover")}
                    >
                      {w.id === UNASSIGNED ? <UserX className="size-[18px] shrink-0 text-warn" /> : <Avatar name={w.name} size={18} />}
                      <span className={clsx("w-16 shrink-0 truncate", w.id === UNASSIGNED && "text-warn")}>{w.name}</span>
                      <div className="flex h-2 flex-1 overflow-hidden rounded-full bg-sunken">
                        <div className="h-full bg-accent" style={{ width: `${(w.overdue / maxLoad) * 100}%` }} />
                        <div className="h-full bg-info/70" style={{ width: `${((w.open - w.overdue) / maxLoad) * 100}%` }} />
                      </div>
                      <span className="w-14 shrink-0 text-right text-ink-3 tabular-nums">
                        {w.open}
                        {w.overdue > 0 && <span className="text-accent"> /{w.overdue}</span>}
                      </span>
                    </button>
                  ))}
                </div>
                <div className="mt-2 flex gap-3 text-[11px] text-ink-3">
                  <span className="flex items-center gap-1">
                    <span className="size-2 rounded-full bg-info/70" />
                    未关闭
                  </span>
                  <span className="flex items-center gap-1">
                    <span className="size-2 rounded-full bg-accent" />
                    其中逾期
                  </span>
                </div>
              </Card>

              {/* 近 8 周趋势 */}
              <Card className="p-4">
                <SectionTitle icon={<TrendingUp className="size-3.5" />}>近 8 周新增与关闭</SectionTitle>
                <div className="flex h-[200px] items-end gap-2 pt-2">
                  {weekly.map((w) => (
                    <div key={w.label} className="group flex flex-1 flex-col items-center gap-1" title={`${w.label} 这周：新增 ${w.created}，关闭 ${w.closed}`}>
                      <div className="flex h-[160px] w-full items-end justify-center gap-0.5">
                        <div className="w-1/2 rounded-t bg-info/70" style={{ height: `${Math.max(2, (w.created / maxWeek) * 150)}px` }} />
                        <div className="w-1/2 rounded-t bg-ok/80" style={{ height: `${Math.max(2, (w.closed / maxWeek) * 150)}px` }} />
                      </div>
                      <span className="text-[9.5px] text-ink-3">{w.label}</span>
                    </div>
                  ))}
                </div>
                <div className="mt-1 flex gap-3 text-[11px] text-ink-3">
                  <span className="flex items-center gap-1">
                    <span className="size-2 rounded-full bg-info/70" />
                    新增
                  </span>
                  <span className="flex items-center gap-1">
                    <span className="size-2 rounded-full bg-ok/80" />
                    关闭
                  </span>
                </div>
              </Card>
            </div>

            {/* 任务表 */}
            <Card className="overflow-hidden">
              <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5">
                <div className="relative w-[240px]">
                  <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-ink-3" />
                  <input
                    value={keyword}
                    onChange={(e) => setKeyword(e.target.value)}
                    placeholder="搜索标题、负责人、#任务号"
                    className="h-8 w-full rounded-lg border border-line bg-sunken pr-2 pl-8 text-[13px] placeholder:text-ink-3 focus:border-accent focus:bg-panel focus:outline-none"
                  />
                </div>
                <FilterSelect value={assignee === null ? "" : String(assignee)} onChange={(v) => setAssignee(v === "" ? null : Number(v))} placeholder="全部负责人">
                  <option value={UNASSIGNED}>未指派</option>
                  {workload
                    .filter((w) => w.id !== UNASSIGNED)
                    .map((w) => (
                      <option key={w.id} value={w.id}>
                        {w.name}（{w.open}）
                      </option>
                    ))}
                </FilterSelect>
                <FilterSelect value={status} onChange={setStatus} placeholder="全部状态">
                  {statuses.map((s) => (
                    <option key={s} value={s}>
                      {s}
                    </option>
                  ))}
                </FilterSelect>
                {ov.versions.length > 0 && (
                  <FilterSelect value={version === null ? "" : String(version)} onChange={(v) => setVersion(v === "" ? null : Number(v))} placeholder="全部版本">
                    <option value={-1}>未设置版本</option>
                    {ov.versions.map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.name}
                      </option>
                    ))}
                  </FilterSelect>
                )}
                <button
                  onClick={() => setQuick(quick === "high" ? null : "high")}
                  className={clsx("h-8 rounded-lg border px-2.5 text-[12.5px]", quick === "high" ? "border-accent/50 bg-accent-soft text-accent-ink" : "border-line text-ink-2 hover:bg-hover")}
                >
                  高优先级 {count("high")}
                </button>
                <div className="flex-1" />
                <span className="text-[12px] text-ink-3 tabular-nums">{filtered.length} 个任务</span>
                <FilterSelect value={sortBy} onChange={(v) => setSortBy(v as SortBy)}>
                  <option value="updated">最近更新</option>
                  <option value="created">最新提出</option>
                  <option value="due">截止日期</option>
                  <option value="priority">优先级</option>
                </FilterSelect>
              </div>

              <table className="w-full text-[12.5px]">
                <thead className="bg-sunken/60 text-left text-[11.5px] text-ink-3">
                  <tr>
                    <th className="w-10 py-2 pl-4">
                      <Check
                        checked={allChecked}
                        partial={!allChecked && filtered.some((i) => checked.has(i.id))}
                        onClick={() =>
                          setChecked((s) => {
                            const n = new Set(s);
                            if (allChecked) filtered.forEach((i) => n.delete(i.id));
                            else filtered.forEach((i) => n.add(i.id));
                            return n;
                          })
                        }
                        title="全选当前筛选结果"
                      />
                    </th>
                    <th className="py-2 font-medium">任务</th>
                    <th className="w-24 py-2 font-medium">状态</th>
                    <th className="w-20 py-2 font-medium">优先级</th>
                    <th className="w-28 py-2 font-medium">负责人</th>
                    <th className="w-28 py-2 font-medium">版本</th>
                    <th className="w-24 py-2 font-medium">截止</th>
                    <th className="w-24 py-2 pr-4 font-medium">更新</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {filtered.slice(0, shown).map((i) => {
                    const due = dueLabel(i.dueDate);
                    const on = checked.has(i.id);
                    return (
                      <tr key={i.id} onClick={() => setOpenId(i.id)} className={clsx("cursor-default", on ? "bg-accent-soft/40" : openId === i.id ? "bg-hover" : "hover:bg-hover/60")}>
                        <td className="py-2 pl-4" onClick={(e) => e.stopPropagation()}>
                          <Check
                            checked={on}
                            onClick={() =>
                              setChecked((s) => {
                                const n = new Set(s);
                                if (n.has(i.id)) n.delete(i.id);
                                else n.add(i.id);
                                return n;
                              })
                            }
                          />
                        </td>
                        <td className="py-2 pr-3">
                          <div className="flex items-center gap-1.5 text-[11.5px] text-ink-3">
                            <span className="font-mono">#{i.id}</span>
                            <span>· {i.tracker}</span>
                            {i.category && <span>· {i.category}</span>}
                            {i.projectName && current && i.projectName !== current.name && <span>· {i.projectName}</span>}
                          </div>
                          <div className="line-clamp-1 text-[13px] text-ink">{i.subject}</div>
                        </td>
                        <td className="py-2 text-ink-2">{i.status}</td>
                        <td className={clsx("py-2", priorityTone(i.priority) === "high" ? "font-medium text-accent-ink" : "text-ink-2")}>{i.priority}</td>
                        <td className="py-2">
                          {i.assignee ? (
                            <span className="inline-flex max-w-[104px] items-center gap-1.5">
                              <Avatar name={i.assignee} size={18} />
                              <span className="truncate">{i.assignee}</span>
                            </span>
                          ) : (
                            <span className="text-warn">未指派</span>
                          )}
                        </td>
                        <td className="max-w-[112px] truncate py-2 text-ink-3">{i.version ?? "—"}</td>
                        <td className={clsx("py-2", due?.tone === "danger" ? "text-accent" : due?.tone === "warn" ? "text-warn" : "text-ink-3")}>
                          {i.dueDate ? (due && due.tone !== "muted" ? due.text : i.dueDate.slice(5)) : "—"}
                        </td>
                        <td className={clsx("py-2 pr-4", i.staleDays >= STALE_DAYS ? "text-info" : "text-ink-3")}>{relativeTime(i.updatedOn)}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {filtered.length === 0 && <div className="py-12 text-center text-[13px] text-ink-3">没有符合条件的任务</div>}
              {filtered.length > shown && (
                <div className="border-t border-line py-2 text-center">
                  <Button size="sm" variant="ghost" onClick={() => setShown((n) => n + PAGE)}>
                    显示更多（还有 {filtered.length - shown} 个）
                  </Button>
                </div>
              )}
            </Card>
          </div>
        )}
      </div>

      {/* 多选操作栏 */}
      {checked.size > 0 && (
        <div className="animate-in absolute bottom-5 left-1/2 z-20 flex -translate-x-1/2 items-center gap-3 rounded-2xl border border-line bg-panel py-2 pr-2 pl-4 shadow-pop">
          <span className="text-[13px] font-medium">已选 {checked.size} 个任务</span>
          <button className="rounded p-0.5 text-ink-3 hover:bg-hover" title="清除选择" onClick={() => setChecked(new Set())}>
            <X className="size-3.5" />
          </button>
          {checked.size === 1 && (
            <Button icon={<GitFork className="size-4" />} onClick={() => setSplitFor([...checked][0])} title="把这个问题的不同部分拆成子任务，分给不同的人">
              拆分…
            </Button>
          )}
          <Button variant="primary" icon={<Users className="size-4" />} onClick={() => setAssignOpen(true)}>
            转派给对应负责人…
          </Button>
        </div>
      )}

      {/* 任务详情抽屉 */}
      {openId !== null && (
        <>
          <div className="absolute inset-0 z-30 bg-black/15" onClick={() => setOpenId(null)} />
          <div className="animate-slide-in absolute inset-y-0 right-0 z-30 w-[min(860px,78%)] border-l border-line bg-canvas shadow-pop">
            <IssueDetail
              detail={detail && detail.view.id === openId ? detail : null}
              loading={detailLoading}
              hideAi
              onRefresh={refreshDetail}
              onResolve={() => setResolveOpen(true)}
              onFollowUp={() => setFollowUp(true)}
              onSplit={() => setSplitFor(openId)}
              onClose={() => setOpenId(null)}
            />
          </div>
        </>
      )}

      {ov && projectId !== null && (
        <AssignDialog
          open={assignOpen}
          onClose={() => setAssignOpen(false)}
          projectId={projectId}
          issues={selectedIssues}
          members={ov.members}
          load={load_}
          onDone={() => {
            setChecked(new Set());
            void load(true);
          }}
        />
      )}
      <ResolveDialog
        open={resolveOpen}
        onClose={() => setResolveOpen(false)}
        issues={detail ? [detail.view] : []}
        allowedStatuses={detail?.issue.allowedStatuses}
      />
      <FollowUpDialog open={followUp} onClose={() => setFollowUp(false)} issue={detail?.view ?? null} />
      <SplitDialog
        issueId={splitFor}
        onClose={() => setSplitFor(null)}
        onDone={() => {
          setChecked(new Set());
          void load(true);
          refreshDetail();
        }}
      />
    </div>
  );
}

function Kpi({
  icon,
  label,
  value,
  tone,
  active,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  value: number;
  tone?: "accent" | "warn" | "ok" | "info";
  active?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={!onClick}
      className={clsx(
        "rounded-xl border bg-panel p-4 text-left shadow-card transition-colors disabled:cursor-default",
        active ? "border-accent/60 ring-2 ring-accent/10" : "border-line enabled:hover:border-line-strong",
      )}
    >
      <div
        className={clsx(
          "mb-3 flex size-8 items-center justify-center rounded-lg",
          tone === "accent" && value > 0 && "bg-accent-soft text-accent-ink",
          tone === "warn" && value > 0 && "bg-warn-soft text-warn",
          tone === "ok" && "bg-ok-soft text-ok",
          tone === "info" && value > 0 && "bg-info-soft text-info",
          (!tone || (tone !== "ok" && value === 0)) && "bg-sunken text-ink-3",
        )}
      >
        {icon}
      </div>
      <div className="text-[22px] leading-none font-semibold tracking-tight tabular-nums">{value}</div>
      <div className="mt-1.5 text-[12px] text-ink-3">{label}</div>
    </button>
  );
}

function FilterSelect({ value, onChange, placeholder, children }: { value: string; onChange: (v: string) => void; placeholder?: string; children: ReactNode }) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className={clsx(
        "h-8 max-w-[160px] rounded-lg border bg-panel px-2 text-[12.5px] focus:border-accent focus:outline-none",
        value !== "" && placeholder ? "border-accent/50 text-accent-ink" : "border-line text-ink-2",
      )}
    >
      {placeholder && <option value="">{placeholder}</option>}
      {children}
    </select>
  );
}

function Check({ checked, partial, onClick, title }: { checked: boolean; partial?: boolean; onClick: () => void; title?: string }) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={clsx(
        "flex size-4 items-center justify-center rounded border",
        checked || partial ? "border-accent bg-accent text-white" : "border-line-strong bg-panel hover:border-accent",
      )}
    >
      {checked ? (
        <svg viewBox="0 0 12 12" className="size-3" fill="none" stroke="currentColor" strokeWidth={2.2}>
          <path d="M2.5 6.2 5 8.5 9.5 3.5" />
        </svg>
      ) : partial ? (
        <span className="h-0.5 w-2 rounded bg-white" />
      ) : null}
    </button>
  );
}
