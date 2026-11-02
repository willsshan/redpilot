import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { Check, ChevronDown, CornerUpLeft, Inbox as InboxIcon, Kanban, LayoutList, MessageSquareText, PlugZap, RefreshCw, Search, Sparkles, X } from "lucide-react";
import { api, errorText, type IssueDetail as Detail, type IssueView, type NamedId } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { categoryColor, dateTime, dueLabel, isMac, priorityTone, relativeTime } from "../lib/format";
import { Avatar, Badge, Button, CategoryBadge, Empty, Kbd } from "../components/ui";
import { IssueDetail } from "../components/IssueDetail";
import { FollowUpDialog, ResolveDialog } from "../components/ResolveDialog";
import { Board, type GroupBy } from "../components/Board";

type Quick = "all" | "unread" | "changed" | "overdue" | "missing";
type View = "list" | "board";
type SortBy = "created" | "updated" | "priority" | "due";
type ListGroup = "month" | "project" | "none";

const SORTERS: Record<SortBy, (a: IssueView, b: IssueView) => number> = {
  created: (a, b) => b.createdOn.localeCompare(a.createdOn) || b.id - a.id,
  updated: (a, b) => b.updatedOn.localeCompare(a.updatedOn) || b.id - a.id,
  priority: (a, b) => b.priorityId - a.priorityId || b.createdOn.localeCompare(a.createdOn),
  // 有截止日期的按日期从近到远，没有的排在最后
  due: (a, b) => (a.dueDate ?? "9999").localeCompare(b.dueDate ?? "9999") || b.createdOn.localeCompare(a.createdOn),
};

interface IssueGroup {
  key: string;
  title: string;
  items: IssueView[];
}

function monthKey(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "未知";
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}

/** 列表分组：按月份（按更新排序时用更新月份，否则用提出月份，新的在前）或按项目（任务多的在前） */
function groupIssues(sorted: IssueView[], by: ListGroup, sortBy: SortBy): IssueGroup[] {
  if (by === "none") return [{ key: "all", title: "", items: sorted }];
  const m = new Map<string, IssueGroup>();
  const now = monthKey(new Date().toISOString());
  for (const i of sorted) {
    let key: string;
    let title: string;
    if (by === "month") {
      key = monthKey(sortBy === "updated" ? i.updatedOn : i.createdOn);
      const [y, mo] = key.split("-");
      title = key === now ? `本月 · ${Number(mo)} 月` : mo ? `${y} 年 ${Number(mo)} 月` : key;
    } else {
      key = String(i.projectId);
      title = i.projectName || "未知项目";
    }
    let g = m.get(key);
    if (!g) {
      g = { key, title, items: [] };
      m.set(key, g);
    }
    g.items.push(i);
  }
  const groups = [...m.values()];
  if (by === "month") groups.sort((a, b) => b.key.localeCompare(a.key));
  else groups.sort((a, b) => b.items.length - a.items.length || a.title.localeCompare(b.title, "zh-CN"));
  return groups;
}

function stored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // 无法写入时只影响记住视图
  }
}

export function InboxPage() {
  const { rev, bump, config, secrets, setPage, syncing, sync, analyzing, focusIssue, setFocusIssue, intent, clearIntent } = useApp();
  const toast = useToast();
  const [issues, setIssues] = useState<IssueView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [keyword, setKeyword] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [quick, setQuick] = useState<Quick>("all");
  const [selected, setSelected] = useState<number | null>(null);
  const [checked, setChecked] = useState<Set<number>>(new Set());
  const [detail, setDetail] = useState<Detail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [resolveFor, setResolveFor] = useState<IssueView[] | null>(null);
  const [followUp, setFollowUp] = useState(false);
  const [analyzingAll, setAnalyzingAll] = useState(false);
  const [view, setViewState] = useState<View>(() => stored("rp.inbox.view", ["list", "board"] as const, "list"));
  const [groupBy, setGroupByState] = useState<GroupBy>(() => stored("rp.board.group", ["status", "category", "project", "urgency"] as const, "status"));
  const [sortBy, setSortByState] = useState<SortBy>(() => stored("rp.list.sort", ["created", "updated", "priority", "due"] as const, "created"));
  const [listGroup, setListGroupState] = useState<ListGroup>(() => stored("rp.list.group", ["month", "project", "none"] as const, "month"));
  const [project, setProject] = useState<number | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [drawer, setDrawer] = useState(false);
  const [statuses, setStatuses] = useState<NamedId[]>([]);
  const searchRef = useRef<HTMLInputElement>(null);

  const setView = useCallback((v: View) => {
    setViewState(v);
    remember("rp.inbox.view", v);
    setDrawer(false);
  }, []);
  const setGroupBy = (g: GroupBy) => {
    setGroupByState(g);
    remember("rp.board.group", g);
  };
  const setSortBy = (v: SortBy) => {
    setSortByState(v);
    remember("rp.list.sort", v);
  };
  const setListGroup = (v: ListGroup) => {
    setListGroupState(v);
    setCollapsed(new Set());
    remember("rp.list.group", v);
  };

  useEffect(() => {
    if (intent?.type === "inboxView") {
      setView(intent.view);
      clearIntent();
    }
  }, [intent, clearIntent, setView]);

  const configured = !!config?.redmine.baseUrl && secrets.redmine;

  const load = useCallback(async () => {
    try {
      setIssues(await api.listIssues({ keyword: keyword || null }));
      setLoadError(null);
    } catch (e) {
      setLoadError(errorText(e));
    } finally {
      setLoaded(true);
    }
  }, [keyword]);

  useEffect(() => {
    void load();
  }, [load, rev]);

  useEffect(() => {
    if (view === "board" && configured) api.statuses().then((xs) => setStatuses(xs.filter((x) => !x.isClosed))).catch(() => {});
  }, [view, configured]);

  // 从规划页或命令面板跳转过来时聚焦指定任务
  useEffect(() => {
    if (focusIssue !== null) {
      setSelected(focusIssue);
      setFocusIssue(null);
      if (view === "board") setDrawer(true);
    }
  }, [focusIssue, setFocusIssue, view]);

  const projects = useMemo(() => {
    const m = new Map<number, { id: number; name: string; count: number }>();
    for (const i of issues) {
      const p = m.get(i.projectId) ?? { id: i.projectId, name: i.projectName, count: 0 };
      p.count += 1;
      m.set(i.projectId, p);
    }
    return [...m.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name, "zh-CN"));
  }, [issues]);

  // 选中的项目同步后不存在了，就回到全部项目
  useEffect(() => {
    if (project !== null && loaded && !projects.some((p) => p.id === project)) setProject(null);
  }, [projects, project, loaded]);

  const inProject = useMemo(() => (project === null ? issues : issues.filter((i) => i.projectId === project)), [issues, project]);

  const sorted = useMemo(() => {
    const filtered = inProject.filter((i) => {
      if (category && (i.analysis.category ?? "未分类") !== category) return false;
      if (quick === "unread" && !i.unread) return false;
      if (quick === "changed" && !i.changeHint) return false;
      if (quick === "overdue" && !i.overdue) return false;
      if (quick === "missing" && i.analysis.missingInfo.length === 0) return false;
      return true;
    });
    return filtered.sort(SORTERS[sortBy]);
  }, [inProject, category, quick, sortBy]);

  const groups = useMemo(() => groupIssues(sorted, listGroup, sortBy), [sorted, listGroup, sortBy]);
  // 列表中实际显示的顺序（折叠的分组不参与 J/K 切换）
  const visible = useMemo(() => groups.flatMap((g) => (collapsed.has(g.key) ? [] : g.items)), [groups, collapsed]);

  const categoryCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const i of inProject) {
      const c = i.analysis.category ?? "未分类";
      m.set(c, (m.get(c) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [inProject]);

  // 列表视图默认选中第一条
  useEffect(() => {
    if (visible.length === 0 || view === "board") return;
    if (selected === null || !issues.some((i) => i.id === selected)) setSelected(visible[0].id);
  }, [visible, issues, selected, view]);

  const loadDetail = useCallback(
    async (id: number, refresh = false) => {
      setDetailLoading(true);
      try {
        const d = await api.issueDetail(id, refresh);
        setDetail(d);
        if (d.view.unread) {
          await api.markRead(id);
          setIssues((xs) => xs.map((x) => (x.id === id ? { ...x, unread: false, changeHint: null } : x)));
        }
      } catch (e) {
        toast.show({ tone: "error", title: `加载 #${id} 失败`, body: errorText(e) });
      } finally {
        setDetailLoading(false);
      }
    },
    [toast],
  );

  useEffect(() => {
    if (selected !== null) void loadDetail(selected);
    else setDetail(null);
  }, [selected, loadDetail]);

  // 分析完成等事件后刷新当前详情中的 AI 结果
  useEffect(() => {
    if (selected === null) return;
    const v = issues.find((i) => i.id === selected);
    if (v && detail && detail.view.id === v.id) setDetail({ ...detail, view: { ...v, unread: false, changeHint: null } });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issues]);

  const current = issues.find((i) => i.id === selected) ?? null;

  // 键盘：J/K 上下，R 回派，/ 搜索，Esc 清除多选
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || resolveFor || followUp) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (view === "board") {
        if (e.key === "Escape" && drawer) setDrawer(false);
        else if (e.key === "r" && drawer && current) {
          e.preventDefault();
          setResolveFor([current]);
        } else if (e.key === "/") {
          e.preventDefault();
          searchRef.current?.focus();
        }
        return;
      }
      const idx = visible.findIndex((i) => i.id === selected);
      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        const n = visible[Math.min(visible.length - 1, idx + 1)];
        if (n) setSelected(n.id);
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = visible[Math.max(0, idx - 1)];
        if (n) setSelected(n.id);
      } else if (e.key === "r" && current) {
        e.preventDefault();
        setResolveFor(checked.size > 1 ? issues.filter((i) => checked.has(i.id)) : [current]);
      } else if (e.key === "/") {
        e.preventDefault();
        searchRef.current?.focus();
      } else if (e.key === "Escape") {
        setChecked(new Set());
      } else if (e.key === "x" && current) {
        toggleCheck(current.id);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  });

  function toggleCheck(id: number) {
    setChecked((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  }

  async function analyzeAll() {
    setAnalyzingAll(true);
    try {
      const r = await api.analyze(checked.size > 0 ? [...checked] : null, false);
      toast.show({
        tone: "success",
        title: r.analyzed.length ? `已分析 ${r.analyzed.length} 个任务` : "所有任务都已是最新分析结果",
        body: r.excluded.length ? `${r.excluded.length} 个任务所属项目已设置为不发送给 AI` : undefined,
      });
    } catch (e) {
      toast.show({ tone: "error", title: "AI 分析失败", body: errorText(e) });
    } finally {
      setAnalyzingAll(false);
    }
  }

  // 看板拖动：状态分组改 Redmine 状态，分类分组改本地分类
  async function moveCard(issue: IssueView, key: string) {
    if (groupBy === "category") {
      const cat = key === "未分类" ? null : key;
      setIssues((xs) => xs.map((x) => (x.id === issue.id ? { ...x, analysis: { ...x.analysis, category: cat, categorySource: "manual" } } : x)));
      try {
        await api.setClassification(issue.id, cat, issue.analysis.module ?? null);
      } catch (e) {
        toast.show({ tone: "error", title: "修改分类失败", body: errorText(e) });
        bump();
      }
      return;
    }
    const statusId = Number(key);
    const name = statuses.find((s) => s.id === statusId)?.name ?? "";
    setIssues((xs) => xs.map((x) => (x.id === issue.id ? { ...x, statusId, statusName: name } : x)));
    try {
      const out = await api.changeStatus(issue.id, statusId);
      toast.show({ tone: out.warnings.length ? "warn" : "success", title: `#${issue.id} 已改为“${out.status ?? name}”`, body: out.warnings.join("\n") || undefined });
    } catch (e) {
      setIssues((xs) => xs.map((x) => (x.id === issue.id ? { ...x, statusId: issue.statusId, statusName: issue.statusName } : x)));
      toast.show({ tone: "error", title: `#${issue.id} 无法改为“${name}”`, body: errorText(e) });
    }
  }

  const categories = [...new Set([...(config?.rules.map((r) => r.category) ?? []), "其他"])];

  if (!configured) {
    return (
      <Empty
        icon={<PlugZap className="size-6" />}
        title="连接你的 Redmine"
        desc="填写 Redmine 地址和 API Key 后，RedPilot 会自动拉取指派给你的任务，并用 AI 做分类和合并规划。"
        action={
          <Button variant="primary" onClick={() => setPage("settings")}>
            前往设置
          </Button>
        }
      />
    );
  }

  const checkedIssues = issues.filter((i) => checked.has(i.id));

  return (
    <div className="flex h-full min-w-0 flex-col">
      {/* 顶栏 */}
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center gap-3 border-b border-line px-5", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">
          收件箱 <span className="ml-1 font-normal text-ink-3 tabular-nums">{issues.length}</span>
        </h2>
        <div className="relative ml-2 w-[300px]">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-ink-3" />
          <input
            ref={searchRef}
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && (e.currentTarget.blur(), setKeyword(""))}
            placeholder="搜索标题、描述、提出人或 #任务号"
            className="h-8 w-full rounded-lg border border-line bg-sunken pr-8 pl-8 text-[13px] placeholder:text-ink-3 focus:border-accent focus:bg-panel focus:outline-none"
          />
          <span className="absolute top-1/2 right-2 -translate-y-1/2">
            <Kbd>/</Kbd>
          </span>
        </div>
        <div className="flex-1" data-tauri-drag-region />
        {view === "board" && (
          <select
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value as GroupBy)}
            className="h-8 rounded-lg border border-line bg-panel px-2 text-[12.5px] text-ink-2 focus:border-accent focus:outline-none"
            title="看板分组"
          >
            <option value="status">按状态分组</option>
            <option value="category">按分类分组</option>
            <option value="project">按项目分组</option>
            <option value="urgency">按紧急度分组</option>
          </select>
        )}
        <div className="flex h-8 items-center rounded-lg border border-line bg-sunken p-0.5">
          {(
            [
              ["list", LayoutList, "列表"],
              ["board", Kanban, "看板"],
            ] as const
          ).map(([v, Icon, label]) => (
            <button
              key={v}
              onClick={() => setView(v)}
              title={`${label}视图`}
              className={clsx(
                "flex h-full items-center gap-1 rounded-md px-2 text-[12.5px]",
                view === v ? "bg-panel font-medium text-ink shadow-sm" : "text-ink-3 hover:text-ink",
              )}
            >
              <Icon className="size-3.5" />
              {label}
            </button>
          ))}
        </div>
        <Button variant="ai" icon={<Sparkles className="size-3.5" />} loading={analyzingAll || analyzing > 0} onClick={() => void analyzeAll()}>
          {checked.size > 0 ? `AI 分析所选 ${checked.size}` : "AI 分析"}
        </Button>
        <Button icon={<RefreshCw className={clsx("size-3.5", syncing && "animate-spin")} />} disabled={syncing} onClick={() => void sync()}>
          同步
        </Button>
      </header>

      {/* 筛选 */}
      <div className="flex shrink-0 items-center gap-1.5 overflow-x-auto border-b border-line px-5 py-2">
        <label
          className={clsx(
            "relative inline-flex h-7 shrink-0 items-center rounded-full border pr-6 pl-2.5 text-[12.5px]",
            project !== null ? "border-accent/50 bg-accent-soft text-accent-ink" : "border-line bg-panel text-ink-2 hover:bg-hover",
          )}
          title="按项目筛选"
        >
          <select
            value={project ?? ""}
            onChange={(e) => setProject(e.target.value ? Number(e.target.value) : null)}
            className="absolute inset-0 cursor-default opacity-0"
          >
            <option value="">全部项目（{issues.length}）</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}（{p.count}）
              </option>
            ))}
          </select>
          <span className="max-w-[160px] truncate">{project !== null ? (projects.find((p) => p.id === project)?.name ?? "项目") : "全部项目"}</span>
          <ChevronDown className="pointer-events-none absolute right-2 size-3" />
        </label>
        <div className="mx-1 h-4 w-px shrink-0 bg-line" />
        <Chip active={category === null} onClick={() => setCategory(null)}>
          全部 <span className="text-ink-3">{inProject.length}</span>
        </Chip>
        {categoryCounts.map(([c, n]) => (
          <Chip key={c} active={category === c} onClick={() => setCategory(category === c ? null : c)}>
            <span className="size-1.5 rounded-full" style={{ backgroundColor: categoryColor(c === "未分类" ? null : c) }} />
            {c} <span className="text-ink-3">{n}</span>
          </Chip>
        ))}
        <div className="mx-1 h-4 w-px shrink-0 bg-line" />
        {(
          [
            ["unread", "未读"],
            ["changed", "有新动态"],
            ["overdue", "已逾期"],
            ["missing", "信息不足"],
          ] as const
        ).map(([q, label]) => (
          <Chip key={q} active={quick === q} onClick={() => setQuick(quick === q ? "all" : q)}>
            {label}
          </Chip>
        ))}
      </div>

      {view === "board" ? (
        <div className="relative min-h-0 flex-1">
          {loadError ? (
            <Empty icon={<X className="size-6" />} title="无法读取本地任务列表" desc={loadError} action={<Button onClick={() => void load()}>重试</Button>} />
          ) : loaded && issues.length === 0 ? (
            <Empty icon={<InboxIcon className="size-6" />} title="收件箱已清空" desc="当前没有指派给你的未关闭任务，休息一下吧。" />
          ) : (
            <Board
              issues={sorted}
              groupBy={groupBy}
              statuses={statuses}
              categories={categories}
              selectedId={drawer ? selected : null}
              onOpen={(id) => {
                setSelected(id);
                setDrawer(true);
              }}
              onMove={(i, k) => void moveCard(i, k)}
              onResolve={(i) => setResolveFor([i])}
            />
          )}
          {drawer && current && (
            <>
              <div className="absolute inset-0 bg-black/15" onClick={() => setDrawer(false)} />
              <div className="animate-slide-in absolute inset-y-0 right-0 w-[min(860px,78%)] border-l border-line bg-canvas shadow-pop">
                <IssueDetail
                  detail={detail && detail.view.id === current.id ? detail : null}
                  loading={detailLoading}
                  onRefresh={() => void loadDetail(current.id, true)}
                  onResolve={() => setResolveFor([current])}
                  onFollowUp={() => setFollowUp(true)}
                  onClose={() => setDrawer(false)}
                />
              </div>
            </>
          )}
        </div>
      ) : (
      <div className="flex min-h-0 flex-1">
        {/* 列表 */}
        <div className="relative w-[400px] shrink-0 overflow-y-auto border-r border-line">
          {!loaded ? (
            <div className="space-y-3 p-4">
              {[0, 1, 2, 3, 4].map((i) => (
                <div key={i} className="skeleton h-[86px]" />
              ))}
            </div>
          ) : loadError ? (
            <Empty
              icon={<X className="size-6" />}
              title="无法读取本地任务列表"
              desc={loadError}
              action={<Button onClick={() => void load()}>重试</Button>}
            />
          ) : (
            <>
              <div className="sticky top-0 z-[2] flex items-center gap-1 border-b border-line bg-canvas/95 px-3 py-1.5 text-[12px] text-ink-3 backdrop-blur">
                <span className="mr-auto pl-1 tabular-nums">{sorted.length} 个任务</span>
                <MiniSelect
                  value={listGroup}
                  onChange={(v) => setListGroup(v as ListGroup)}
                  options={[
                    ["month", "按月份分组"],
                    ["project", "按项目分组"],
                    ["none", "不分组"],
                  ]}
                />
                <MiniSelect
                  value={sortBy}
                  onChange={(v) => setSortBy(v as SortBy)}
                  options={[
                    ["created", "提出时间 · 新→旧"],
                    ["updated", "更新时间 · 新→旧"],
                    ["priority", "优先级 · 高→低"],
                    ["due", "截止日期 · 近→远"],
                  ]}
                />
              </div>
              {sorted.length === 0 ? (
                <Empty
                  icon={<InboxIcon className="size-6" />}
                  title={issues.length === 0 ? "收件箱已清空" : "没有符合条件的任务"}
                  desc={issues.length === 0 ? "当前没有指派给你的未关闭任务，休息一下吧。" : "试试调整筛选条件。"}
                />
              ) : (
                groups.map((g) => (
                  <section key={g.key}>
                    {g.title && (
                      <button
                        onClick={() =>
                          setCollapsed((s) => {
                            const n = new Set(s);
                            if (n.has(g.key)) n.delete(g.key);
                            else n.add(g.key);
                            return n;
                          })
                        }
                        className="sticky top-[33px] z-[1] flex w-full items-center gap-1.5 border-b border-line/70 bg-sunken/95 px-4 py-1.5 text-left text-[12px] font-semibold text-ink-2 backdrop-blur hover:text-ink"
                      >
                        <ChevronDown className={clsx("size-3.5 text-ink-3 transition-transform", collapsed.has(g.key) && "-rotate-90")} />
                        <span className="truncate">{g.title}</span>
                        <span className="font-normal text-ink-3 tabular-nums">{g.items.length}</span>
                      </button>
                    )}
                    {!collapsed.has(g.key) && (
                      <ul className="divide-y divide-line/70">
                        {g.items.map((i) => (
                          <IssueRow
                            key={i.id}
                            issue={i}
                            active={i.id === selected}
                            checked={checked.has(i.id)}
                            onClick={() => setSelected(i.id)}
                            onCheck={() => toggleCheck(i.id)}
                            showProject={listGroup !== "project"}
                            showUpdated={sortBy === "updated"}
                          />
                        ))}
                      </ul>
                    )}
                  </section>
                ))
              )}
            </>
          )}

          {checked.size > 0 && (
            <div className="sticky bottom-3 mx-3 flex items-center gap-2 rounded-xl border border-line bg-panel p-2 pl-3 shadow-pop">
              <span className="text-[12.5px] font-medium">已选 {checked.size} 个</span>
              <button className="rounded p-0.5 text-ink-3 hover:bg-hover" title="清除选择 (Esc)" onClick={() => setChecked(new Set())}>
                <X className="size-3.5" />
              </button>
              <div className="flex-1" />
              <Button size="sm" variant="primary" icon={<CornerUpLeft className="size-3.5" />} onClick={() => setResolveFor(checkedIssues)}>
                批量回派
              </Button>
            </div>
          )}
        </div>

        {/* 详情 */}
        <div className="min-w-0 flex-1 bg-canvas">
          {current ? (
            <IssueDetail
              detail={detail && detail.view.id === current.id ? detail : null}
              loading={detailLoading}
              onRefresh={() => void loadDetail(current.id, true)}
              onResolve={() => setResolveFor([current])}
              onFollowUp={() => setFollowUp(true)}
            />
          ) : (
            loaded && <Empty icon={<Check className="size-6" />} title="选择一个任务查看详情" desc={<>用 <Kbd>J</Kbd> <Kbd>K</Kbd> 上下切换，<Kbd>X</Kbd> 多选，<Kbd>R</Kbd> 回派</>} />
          )}
        </div>
      </div>
      )}

      <ResolveDialog
        open={!!resolveFor}
        onClose={() => setResolveFor(null)}
        issues={resolveFor ?? []}
        allowedStatuses={resolveFor?.length === 1 && detail?.view.id === resolveFor[0].id ? detail.issue.allowedStatuses : undefined}
      />
      <FollowUpDialog open={followUp} onClose={() => setFollowUp(false)} issue={current} />
    </div>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors",
        active ? "border-ink/80 bg-ink text-canvas [&_.text-ink-3]:text-canvas/60" : "border-line bg-panel text-ink-2 hover:bg-hover",
      )}
    >
      {children}
    </button>
  );
}

function MiniSelect({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: [string, string][] }) {
  return (
    <label className="relative inline-flex h-6 items-center rounded-md pr-5 pl-1.5 text-ink-2 hover:bg-hover">
      <select value={value} onChange={(e) => onChange(e.target.value)} className="absolute inset-0 cursor-default opacity-0">
        {options.map(([v, label]) => (
          <option key={v} value={v}>
            {label}
          </option>
        ))}
      </select>
      {options.find(([v]) => v === value)?.[1]}
      <ChevronDown className="pointer-events-none absolute right-1 size-3 text-ink-3" />
    </label>
  );
}

function IssueRow({
  issue: i,
  active,
  checked,
  onClick,
  onCheck,
  showProject = true,
  showUpdated = false,
}: {
  issue: IssueView;
  active: boolean;
  checked: boolean;
  onClick: () => void;
  onCheck: () => void;
  showProject?: boolean;
  /** 按更新时间排序时显示更新时间，否则显示提出时间 */
  showUpdated?: boolean;
}) {
  const a = i.analysis;
  const due = dueLabel(i.dueDate);
  const high = priorityTone(i.priorityName) === "high";
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: "nearest" });
  }, [active]);

  return (
    <li
      ref={ref}
      onClick={onClick}
      className={clsx("group relative flex gap-2.5 px-4 py-3 transition-colors", active ? "bg-accent-soft/60" : "hover:bg-hover/60")}
    >
      {high && <span className="absolute top-3 bottom-3 left-0 w-[3px] rounded-r bg-accent" />}
      <button
        onClick={(e) => {
          e.stopPropagation();
          onCheck();
        }}
        className={clsx(
          "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded border transition-opacity",
          checked ? "border-accent bg-accent text-white" : "border-line-strong bg-panel opacity-0 group-hover:opacity-100",
        )}
        title="多选 (X)"
      >
        {checked && <Check className="size-3" strokeWidth={3} />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[11.5px] text-ink-3">
          {i.unread && <span className="size-1.5 rounded-full bg-accent" />}
          <span className="font-mono">#{i.id}</span>
          {showProject && <span className="truncate">· {i.projectName}</span>}
          <span className="ml-auto shrink-0" title={`提出于 ${dateTime(i.createdOn)}，更新于 ${dateTime(i.updatedOn)}`}>
            {showUpdated ? `${relativeTime(i.updatedOn)}更新` : `${relativeTime(i.createdOn)}提出`}
          </span>
        </div>
        <div className={clsx("mt-0.5 line-clamp-2 text-[13.5px] leading-snug text-ink", i.unread ? "font-semibold" : "font-medium")}>{i.subject}</div>
        {i.changeHint ? (
          <div className="mt-0.5 flex items-center gap-1 truncate text-[12.5px] text-accent-ink">
            <MessageSquareText className="size-3 shrink-0" />
            <span className="truncate">{i.changeHint}</span>
          </div>
        ) : (
          a.summary && <div className="mt-0.5 truncate text-[12.5px] text-ink-3">{a.summary}</div>
        )}
        <div className="mt-1.5 flex items-center gap-1.5">
          <CategoryBadge category={a.category} source={a.categorySource} />
          {a.module && <Badge>{a.module}</Badge>}
          {a.urgency === "高" && <Badge tone="accent">紧急</Badge>}
          {a.missingInfo.length > 0 && <Badge tone="warn">缺信息</Badge>}
          {due && due.tone !== "muted" && <Badge tone={due.tone === "danger" ? "accent" : "warn"}>{due.text}</Badge>}
          <span className="ml-auto">
            <Avatar name={i.authorName} size={18} />
          </span>
        </div>
      </div>
    </li>
  );
}
