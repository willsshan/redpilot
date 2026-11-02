import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import clsx from "clsx";
import { Check, CornerUpLeft, Inbox as InboxIcon, Kanban, LayoutList, MessageSquareText, PlugZap, RefreshCw, Search, Sparkles, X } from "lucide-react";
import { api, errorText, type IssueDetail as Detail, type IssueView, type NamedId } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { categoryColor, dueLabel, isMac, priorityTone, relativeTime } from "../lib/format";
import { Avatar, Badge, Button, CategoryBadge, Empty, Kbd } from "../components/ui";
import { IssueDetail } from "../components/IssueDetail";
import { FollowUpDialog, ResolveDialog } from "../components/ResolveDialog";
import { Board, type GroupBy } from "../components/Board";

type Quick = "all" | "unread" | "changed" | "overdue" | "missing";
type View = "list" | "board";

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
  const [groupBy, setGroupByState] = useState<GroupBy>(() => stored("rp.board.group", ["status", "category", "urgency"] as const, "status"));
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

  const visible = useMemo(() => {
    return issues.filter((i) => {
      if (category && (i.analysis.category ?? "未分类") !== category) return false;
      if (quick === "unread" && !i.unread) return false;
      if (quick === "changed" && !i.changeHint) return false;
      if (quick === "overdue" && !i.overdue) return false;
      if (quick === "missing" && i.analysis.missingInfo.length === 0) return false;
      return true;
    });
  }, [issues, category, quick]);

  const categoryCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const i of issues) {
      const c = i.analysis.category ?? "未分类";
      m.set(c, (m.get(c) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  }, [issues]);

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
        <Chip active={category === null} onClick={() => setCategory(null)}>
          全部 <span className="text-ink-3">{issues.length}</span>
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
              issues={visible}
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
          ) : visible.length === 0 ? (
            <Empty
              icon={<InboxIcon className="size-6" />}
              title={issues.length === 0 ? "收件箱已清空" : "没有符合条件的任务"}
              desc={issues.length === 0 ? "当前没有指派给你的未关闭任务，休息一下吧。" : "试试调整筛选条件。"}
            />
          ) : (
            <ul className="divide-y divide-line/70">
              {visible.map((i) => (
                <IssueRow
                  key={i.id}
                  issue={i}
                  active={i.id === selected}
                  checked={checked.has(i.id)}
                  onClick={() => setSelected(i.id)}
                  onCheck={() => toggleCheck(i.id)}
                />
              ))}
            </ul>
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

function IssueRow({
  issue: i,
  active,
  checked,
  onClick,
  onCheck,
}: {
  issue: IssueView;
  active: boolean;
  checked: boolean;
  onClick: () => void;
  onCheck: () => void;
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
          <span className="truncate">· {i.projectName}</span>
          <span className="ml-auto shrink-0">{relativeTime(i.updatedOn)}</span>
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
