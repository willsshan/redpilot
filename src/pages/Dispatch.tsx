import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import clsx from "clsx";
import { Check, ChevronRight, Copy, Forward, GitFork, PlugZap, RefreshCw, Settings2, Sparkles, X } from "lucide-react";
import {
  api,
  errorText,
  type DispatchItem,
  type DispatchQueue,
  type DispatchRequest,
  type IssueDetail as Detail,
  type NamedId,
  type SimilarIssue,
  type SplitPart,
} from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { dateTime, dueLabel, isMac, priorityTone, relativeTime } from "../lib/format";
import { Avatar, Badge, Button, Empty, Input, Kbd, Modal, Select, Textarea } from "../components/ui";
import { IssueDetail } from "../components/IssueDetail";
import { FollowUpDialog } from "../components/ResolveDialog";
import { SplitDialog } from "../components/SplitDialog";
import { DispatchPanel, outcomeToast, recommendedRequest } from "../components/DispatchPanel";

const PROJECT_KEY = "rp.dispatch.project";

/** 记住的项目筛选（null 表示全部项目） */
function storedProject(): number | null {
  try {
    const v = Number(localStorage.getItem(PROJECT_KEY));
    return Number.isInteger(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

interface ProjectGroup {
  id: number | null;
  /** 按项目分组时的项目名，不分组时为空 */
  title: string | null;
  items: DispatchItem[];
}

/** 分派台：开发经理把收到的问题快速派给团队成员 */
export function DispatchPage() {
  const { rev, config, secrets, setPage, dispatch: go, syncing, sync } = useApp();
  const toast = useToast();
  const [queue, setQueue] = useState<DispatchQueue | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  // 已提交（或在撤销倒计时中）的任务先从列表里拿掉
  const [hidden, setHidden] = useState<Set<number>>(new Set());
  const [selected, setSelected] = useState<number | null>(null);
  const [checked, setChecked] = useState<Set<number>>(new Set());
  const [detail, setDetail] = useState<Detail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [followUp, setFollowUp] = useState(false);
  const [splitFor, setSplitFor] = useState<{ id: number; parts: SplitPart[] } | null>(null);
  const [custom, setCustom] = useState<{ id: number; parts: SplitPart[] } | null>(null);
  const [dupFor, setDupFor] = useState<number | null>(null);
  const [aiBusy, setAiBusy] = useState<"all" | number | null>(null);
  const [project, setProjectState] = useState<number | null>(storedProject);
  const loadSeq = useRef(0);

  const setProject = useCallback((id: number | null) => {
    setProjectState(id);
    setChecked(new Set());
    try {
      if (id === null) localStorage.removeItem(PROJECT_KEY);
      else localStorage.setItem(PROJECT_KEY, String(id));
    } catch {
      // 无法写入时只影响记住筛选
    }
  }, []);

  const configured = !!config?.redmine.baseUrl && secrets.redmine;

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    try {
      const q = await api.dispatchQueue();
      if (seq !== loadSeq.current) return;
      setQueue(q);
      // 已经离开分派台的任务不用再隐藏（之后再回到分派台时能正常显示）
      setHidden((h) => new Set([...h].filter((id) => q.items.some((i) => i.issue.id === id))));
      setLoadError(null);
    } catch (e) {
      if (seq === loadSeq.current) setLoadError(errorText(e));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (configured) void load();
  }, [load, rev, configured]);

  const all = useMemo(() => (queue?.items ?? []).filter((i) => !hidden.has(i.issue.id)), [queue, hidden]);
  // 各项目的待分派数，多的在前
  const projectCounts = useMemo(() => {
    const m = new Map<number, { id: number; name: string; count: number }>();
    for (const i of all) {
      const p = m.get(i.issue.projectId) ?? { id: i.issue.projectId, name: i.issue.projectName, count: 0 };
      p.count += 1;
      m.set(p.id, p);
    }
    return [...m.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name, "zh-CN"));
  }, [all]);
  // 选了项目只看这个项目；全部项目时按项目分组（只有一个项目时不分组）
  const groups: ProjectGroup[] = useMemo(() => {
    if (project !== null) return [{ id: project, title: null, items: all.filter((i) => i.issue.projectId === project) }];
    if (projectCounts.length <= 1) return [{ id: null, title: null, items: all }];
    return projectCounts.map((p) => ({ id: p.id, title: p.name, items: all.filter((i) => i.issue.projectId === p.id) }));
  }, [all, project, projectCounts]);
  // 列表中实际显示的顺序，切换、批量派单和 AI 推荐都只针对这些任务
  const items = useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const projectName = project === null ? null : (queue?.projects.find((p) => p.id === project)?.name ?? projectCounts.find((p) => p.id === project)?.name ?? `项目 #${project}`);
  const projectOf = (pid: number) => queue?.projects.find((p) => p.id === pid);
  const current = items.find((i) => i.issue.id === selected) ?? null;

  useEffect(() => {
    if (items.length === 0) return;
    if (selected === null || !items.some((i) => i.issue.id === selected)) setSelected(items[0].issue.id);
  }, [items, selected]);

  const loadDetail = useCallback(
    async (id: number, refresh = false) => {
      setDetailLoading(true);
      try {
        const d = await api.issueDetail(id, refresh);
        setDetail(d);
        if (d.view.unread) await api.markRead(id);
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

  /** 从列表里拿掉这些任务，选中下一个 */
  function hide(ids: number[]) {
    const idx = items.findIndex((i) => i.issue.id === selected);
    const rest = items.filter((i) => !ids.includes(i.issue.id));
    if (selected !== null && ids.includes(selected)) setSelected(rest[Math.min(idx, rest.length - 1)]?.issue.id ?? null);
    setHidden((h) => new Set([...h, ...ids]));
    setChecked((c) => new Set([...c].filter((x) => !ids.includes(x))));
  }
  const unhide = (ids: number[]) => setHidden((h) => new Set([...h].filter((x) => !ids.includes(x))));

  function submitOne(req: DispatchRequest, title: string) {
    const id = req.issueId;
    hide([id]);
    setCustom(null);
    toast.withUndo(
      `#${id} ${title}`,
      async () => {
        try {
          const t = outcomeToast(await api.dispatchIssue(req));
          toast.show({ tone: t.warn ? "warn" : "success", title: t.title, body: t.body });
        } catch (e) {
          unhide([id]);
          toast.show({ tone: "error", title: `#${id} 派单失败`, body: errorText(e) });
        }
      },
      5000,
      () => unhide([id]),
    );
  }

  /** 批量：按推荐派出，或全部派给同一个人 */
  function submitMany(targets: DispatchItem[], to?: { id: number; name: string }) {
    const reqs = targets
      .map((i) => (to ? ({ issueId: i.issue.id, mode: "single", assigneeId: to.id, autoDue: true, suggestedId: i.suggestion?.assigneeId ?? null } as DispatchRequest) : recommendedRequest(i)))
      .filter((r): r is DispatchRequest => !!r);
    const skipped = targets.length - reqs.length;
    if (reqs.length === 0) {
      toast.show({ tone: "warn", title: "没有可以按推荐派出的任务", body: "这些任务没有推荐人选，请逐个选择成员" });
      return;
    }
    const ids = reqs.map((r) => r.issueId);
    hide(ids);
    toast.withUndo(
      to ? `正在把 ${reqs.length} 个任务派给 ${to.name}` : `正在按推荐派出 ${reqs.length} 个任务${skipped ? `（${skipped} 个没有推荐，已跳过）` : ""}`,
      async () => {
        const outs = await api.dispatchBatch(reqs);
        const failed = outs.filter((o) => !o.ok);
        unhide(failed.map((f) => f.issueId));
        toast.show({
          tone: failed.length || outs.some((o) => o.warnings.length) ? "warn" : "success",
          title: `已派出 ${outs.length - failed.length} 个任务${failed.length ? `，${failed.length} 个失败` : ""}`,
          body:
            [
              ...failed.map((f) => `#${f.issueId}：${errorText(f.error)}`),
              ...outs.filter((o) => o.ok).flatMap((o) => o.warnings.map((w) => `#${o.issueId}：${w}`)),
            ].join("\n") || undefined,
        });
      },
      5000,
      () => unhide(ids),
    );
  }

  async function keep(id: number) {
    hide([id]);
    try {
      await api.dispatchKeep(id);
      toast.show({
        tone: "success",
        title: `#${id} 留给自己处理`,
        body: "已移出分派台，仍在收件箱里",
        action: {
          label: "撤销",
          onClick: () => {
            unhide([id]);
            void api.dispatchUnmark(id);
          },
        },
      });
    } catch (e) {
      unhide([id]);
      toast.show({ tone: "error", title: "操作失败", body: errorText(e) });
    }
  }

  async function aiSuggest(ids: number[], scope: "all" | number) {
    setAiBusy(scope);
    try {
      const n = await api.aiDispatch(ids);
      await load();
      toast.show({ tone: "success", title: n ? `AI 已给出 ${n} 个任务的派单建议` : "AI 没有给出新的建议" });
    } catch (e) {
      toast.show({ tone: "error", title: "AI 推荐失败", body: errorText(e) });
    } finally {
      setAiBusy(null);
    }
  }

  const modal = followUp || splitFor !== null || dupFor !== null;
  const checkedItems = items.filter((i) => checked.has(i.issue.id));
  const sameProject = checkedItems.length > 0 && checkedItems.every((i) => i.issue.projectId === checkedItems[0].issue.projectId);

  // 列表快捷键：J/K 切换，X 多选，⇧↵ 按推荐派出，Esc 清除多选；有多选时数字键把所选任务都派给该成员
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || modal) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const idx = items.findIndex((i) => i.issue.id === selected);
      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        const n = items[Math.min(items.length - 1, idx + 1)];
        if (n) setSelected(n.issue.id);
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = items[Math.max(0, idx - 1)];
        if (n) setSelected(n.issue.id);
      } else if (e.key === "x" && current) {
        setChecked((s) => {
          const n = new Set(s);
          if (n.has(current.issue.id)) n.delete(current.issue.id);
          else n.add(current.issue.id);
          return n;
        });
      } else if ((e.key === "[" || e.key === "]") && projectCounts.length > 0) {
        // 在“全部项目”和各项目之间切换
        e.preventDefault();
        const order: (number | null)[] = [null, ...projectCounts.map((p) => p.id)];
        const at = Math.max(0, order.indexOf(project));
        setProject(order[(at + (e.key === "]" ? 1 : -1) + order.length) % order.length]);
      } else if (e.key === "Escape") {
        setChecked(new Set());
      } else if (e.key === "Enter" && e.shiftKey) {
        e.preventDefault();
        submitMany(checkedItems.length ? checkedItems : items);
      } else if (checked.size > 0 && /^Digit[1-9]$/.test(e.code) && !e.shiftKey) {
        e.preventDefault();
        if (!sameProject) {
          toast.show({ tone: "warn", title: "所选任务属于不同项目", body: "数字键对应的成员按项目区分，请按项目分别选择，或用 ⇧↵ 按推荐派出" });
          return;
        }
        const m = projectOf(checkedItems[0].issue.projectId)?.members.find((x) => x.key === Number(e.code.slice(5)));
        if (m) submitMany(checkedItems, { id: m.id, name: m.name });
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  });

  if (!configured) {
    return (
      <Empty
        icon={<PlugZap className="size-6" />}
        title="连接你的 Redmine"
        desc="连接后，指派给你的新问题和你管理的项目里未指派的问题会出现在这里，可以一键派给团队成员。"
        action={
          <Button variant="primary" onClick={() => setPage("settings")}>
            前往设置
          </Button>
        }
      />
    );
  }

  const pendingText = queue?.pendingStatuses.length ? queue.pendingStatuses.join("、") : "新建";
  const scopeText = `指派给你且状态为「${pendingText}」的任务${config?.sync.managedUnassigned ? "，以及你管理的项目里未指派的任务" : ""}`;

  return (
    <div className="flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center gap-3 border-b border-line px-5", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">
          分派台 <span className="ml-1 font-normal text-ink-3 tabular-nums">{queue ? all.length : ""}</span>
        </h2>
        <span data-tauri-drag-region className="truncate text-[12px] text-ink-3" title={scopeText}>
          {scopeText}
        </span>
        <div className="flex-1" data-tauri-drag-region />
        <Button variant="ghost" icon={<Settings2 className="size-3.5" />} onClick={() => go("settings", { type: "settingsSection", id: "dispatch" })}>
          分派设置
        </Button>
        <Button
          variant="ai"
          icon={<Sparkles className="size-3.5" />}
          loading={aiBusy === "all"}
          disabled={items.length === 0}
          onClick={() => void aiSuggest((checkedItems.length ? checkedItems : items).slice(0, 30).map((i) => i.issue.id), "all")}
        >
          {checkedItems.length ? `AI 推荐所选 ${checkedItems.length}` : "AI 推荐"}
        </Button>
        <Button icon={<RefreshCw className={clsx("size-3.5", (syncing || loading) && "animate-spin")} />} disabled={syncing} onClick={() => void sync()}>
          同步
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        {/* 队列 */}
        <div className="relative w-[390px] shrink-0 overflow-y-auto border-r border-line">
          {!queue && !loadError ? (
            <div className="space-y-3 p-4">
              <div className="text-[12px] text-ink-3">正在读取成员、历史分工和负载…</div>
              {[0, 1, 2, 3].map((i) => (
                <div key={i} className="skeleton h-[92px]" />
              ))}
            </div>
          ) : loadError && !queue ? (
            <Empty icon={<X className="size-6" />} title="无法读取分派台" desc={loadError} action={<Button onClick={() => void load()}>重试</Button>} />
          ) : all.length === 0 ? (
            <Empty
              icon={<Check className="size-6" />}
              title="没有待分派的任务"
              desc={`${scopeText}会出现在这里。`}
              action={<Button onClick={() => go("settings", { type: "settingsSection", id: "dispatch" })}>调整分派范围</Button>}
            />
          ) : (
            <>
              <div className="sticky top-0 z-[3] flex h-11 items-center gap-2 border-b border-line bg-canvas/95 px-3 text-[12px] whitespace-nowrap text-ink-3 backdrop-blur">
                {projectCounts.length > 1 || project !== null ? (
                  <div className="min-w-0 flex-1">
                    <Select
                      value={project ?? ""}
                      onChange={(e) => {
                        setProject(e.target.value ? Number(e.target.value) : null);
                        // 选完交还焦点，J/K 和数字键可以直接用
                        e.currentTarget.blur();
                      }}
                      className="h-7 max-w-full truncate text-[12.5px]"
                      title={projectCounts.length > 1 ? "按 [ 和 ] 也可以切换项目" : undefined}
                    >
                      <option value="">全部项目（{all.length}）</option>
                      {projectCounts.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name}（{p.count}）
                        </option>
                      ))}
                      {project !== null && !projectCounts.some((p) => p.id === project) && <option value={project}>{projectName}（0）</option>}
                    </Select>
                  </div>
                ) : (
                  <span className="min-w-0 flex-1 truncate pl-1 tabular-nums">{items.length} 个待分派</span>
                )}
                <span className="shrink-0" title="J / K 上下切换">
                  <Kbd>X</Kbd> 多选 · <Kbd>⇧↵</Kbd> 全部派出
                </span>
              </div>
              {items.length === 0 ? (
                <Empty
                  icon={<Check className="size-6" />}
                  title={`「${projectName}」已全部分派完`}
                  desc={`其他项目还有 ${all.length} 个待分派`}
                  action={<Button onClick={() => setProject(null)}>查看全部项目</Button>}
                />
              ) : (
                groups.map((g) => (
                  <section key={g.id ?? "all"}>
                    {g.title && (
                      <button
                        onClick={() => g.id !== null && setProject(g.id)}
                        title="只看这个项目"
                        className="group/h sticky top-11 z-[2] flex w-full items-center gap-1.5 border-b border-line/70 bg-sunken/95 px-4 py-1.5 text-left text-[12px] font-semibold text-ink-2 backdrop-blur hover:text-ink"
                      >
                        <span className="truncate">{g.title}</span>
                        <span className="font-normal text-ink-3 tabular-nums">{g.items.length}</span>
                        <span className="ml-auto flex items-center gap-0.5 font-normal text-ink-3 opacity-0 group-hover/h:opacity-100">
                          只看这个项目 <ChevronRight className="size-3" />
                        </span>
                      </button>
                    )}
                    <ul className="divide-y divide-line/70">
                      {g.items.map((i) => (
                        <QueueRow
                          key={i.issue.id}
                          item={i}
                          active={i.issue.id === selected}
                          checked={checked.has(i.issue.id)}
                          onClick={() => setSelected(i.issue.id)}
                          onCheck={() =>
                            setChecked((s) => {
                              const n = new Set(s);
                              if (n.has(i.issue.id)) n.delete(i.issue.id);
                              else n.add(i.issue.id);
                              return n;
                            })
                          }
                        />
                      ))}
                    </ul>
                  </section>
                ))
              )}
            </>
          )}

          {checkedItems.length > 0 && (
            <div className="sticky bottom-3 mx-3 flex items-center gap-2 rounded-xl border border-line bg-panel p-2 pl-3 shadow-pop">
              <span className="text-[12.5px] font-medium whitespace-nowrap">已选 {checkedItems.length} 个</span>
              <button className="rounded p-0.5 text-ink-3 hover:bg-hover" title="清除选择 (Esc)" onClick={() => setChecked(new Set())}>
                <X className="size-3.5" />
              </button>
              <div className="flex-1" />
              {sameProject && (
                <div className="w-28">
                  <Select
                    value=""
                    onChange={(e) => {
                      const m = projectOf(checkedItems[0].issue.projectId)?.members.find((x) => x.id === Number(e.target.value));
                      if (m) submitMany(checkedItems, { id: m.id, name: m.name });
                    }}
                    className="h-7 text-[12.5px]"
                    title="全部派给同一个人（也可以按数字键）"
                  >
                    <option value="">全部派给…</option>
                    {projectOf(checkedItems[0].issue.projectId)
                      ?.members.filter((m) => !m.isGroup)
                      .map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.key ? `${m.key} · ` : ""}
                          {m.name}（{m.load}）
                        </option>
                      ))}
                  </Select>
                </div>
              )}
              <Button size="sm" variant="primary" icon={<Forward className="size-3.5" />} onClick={() => submitMany(checkedItems)}>
                按推荐派出 <span className="opacity-70">⇧↵</span>
              </Button>
            </div>
          )}
        </div>

        {/* 详情 + 派单栏 */}
        <div className="min-w-0 flex-1 bg-canvas">
          {current ? (
            <IssueDetail
              detail={detail && detail.view.id === current.issue.id ? detail : null}
              loading={detailLoading}
              onRefresh={() => void loadDetail(current.issue.id, true)}
              onResolve={() => {}}
              onFollowUp={() => setFollowUp(true)}
              footer={
                <DispatchPanel
                  key={current.issue.id}
                  item={current}
                  project={projectOf(current.issue.projectId)}
                  config={config}
                  meId={queue?.meId}
                  active={!modal && checked.size === 0}
                  customParts={custom?.id === current.issue.id ? custom.parts : null}
                  onClearCustom={() => setCustom(null)}
                  onSubmit={submitOne}
                  onFollowUp={() => setFollowUp(true)}
                  onEditSplit={(parts) => setSplitFor({ id: current.issue.id, parts })}
                  onDuplicate={() => setDupFor(current.issue.id)}
                  onKeep={() => void keep(current.issue.id)}
                  onAi={() => void aiSuggest([current.issue.id], current.issue.id)}
                  aiBusy={aiBusy === current.issue.id}
                />
              }
            />
          ) : (
            queue && all.length > 0 && <Empty icon={<Forward className="size-6" />} title="选择一个任务开始分派" desc="数字键直接派给对应成员，⇧+数字可以同时选前端和后端" />
          )}
        </div>
      </div>

      <FollowUpDialog open={followUp} onClose={() => setFollowUp(false)} issue={current?.issue ?? null} />
      <SplitDialog
        issueId={splitFor?.id ?? null}
        initialParts={splitFor?.parts}
        onClose={() => setSplitFor(null)}
        onDone={() => {}}
        onSubmitParts={(parts) => splitFor && setCustom({ id: splitFor.id, parts })}
      />
      <DuplicateDialog
        issueId={dupFor}
        similar={detail && detail.view.id === dupFor ? detail.similar : []}
        statuses={detail && detail.view.id === dupFor ? detail.issue.allowedStatuses : []}
        onClose={() => setDupFor(null)}
        onSubmit={(original, notes, statusId) => {
          const id = dupFor!;
          setDupFor(null);
          hide([id]);
          toast.withUndo(
            `#${id} 标为与 #${original} 重复`,
            async () => {
              try {
                await api.markDuplicate(id, original, notes, statusId);
                toast.show({ tone: "success", title: `#${id} 已标为与 #${original} 重复`, body: `#${original} 关闭时 Redmine 会一并关闭 #${id}` });
              } catch (e) {
                unhide([id]);
                toast.show({ tone: "error", title: "标记重复失败", body: errorText(e) });
              }
            },
            5000,
            () => unhide([id]),
          );
        }}
      />
    </div>
  );
}

function QueueRow({ item, active, checked, onClick, onCheck }: { item: DispatchItem; active: boolean; checked: boolean; onClick: () => void; onCheck: () => void }) {
  const i = item.issue;
  const a = i.analysis;
  const due = dueLabel(i.dueDate);
  const high = priorityTone(i.priorityName) === "high";
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: "nearest" });
  }, [active]);
  const multi = item.multi && item.multi.parts.length >= 2 ? item.multi : null;

  return (
    <li ref={ref} onClick={onClick} className={clsx("group relative flex gap-2.5 px-4 py-3 transition-colors", active ? "bg-accent-soft/60" : "hover:bg-hover/60")}>
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
          {!i.assignedToId && <span className="shrink-0 text-warn">· 未指派</span>}
          <span className="ml-auto shrink-0" title={`提出于 ${dateTime(i.createdOn)}`}>
            {relativeTime(i.createdOn)}
          </span>
        </div>
        <div className={clsx("mt-0.5 line-clamp-2 text-[13.5px] leading-snug text-ink", i.unread ? "font-semibold" : "font-medium")}>{i.subject}</div>
        <div className="mt-1 flex items-center gap-1.5 text-[12px]">
          {multi ? (
            <>
              <GitFork className="size-3 shrink-0 text-accent" />
              <span className="truncate text-ink-2" title={multi.reason}>
                {multi.parts.map((p) => `${p.assigneeName}${p.role ? `（${p.role}）` : ""}`).join(" + ")}
              </span>
            </>
          ) : item.suggestion ? (
            <>
              <Forward className="size-3 shrink-0 text-ink-3" />
              <span className="truncate text-ink-2" title={item.suggestion.reason}>
                {item.suggestion.assigneeName}
                <span className="text-ink-3"> · {item.suggestion.reason}</span>
              </span>
            </>
          ) : (
            <span className="text-ink-3">没有推荐人选</span>
          )}
        </div>
        <div className="mt-1.5 flex items-center gap-1.5">
          {a.missingInfo.length > 0 && <Badge tone="warn">缺信息，可先追问</Badge>}
          {item.itemCount >= 2 && !multi && <Badge tone="info">{item.itemCount} 个条目</Badge>}
          {a.urgency === "高" && <Badge tone="accent">紧急</Badge>}
          {due && due.tone !== "muted" && <Badge tone={due.tone === "danger" ? "accent" : "warn"}>{due.text}</Badge>}
          <span className="ml-auto flex items-center gap-1 text-[11.5px] text-ink-3">
            <Avatar name={i.authorName} size={16} />
            {i.authorName}
          </span>
        </div>
      </div>
    </li>
  );
}

/** 标为重复：从相似任务中选，或输入任务号 */
function DuplicateDialog({
  issueId,
  similar,
  statuses,
  onClose,
  onSubmit,
}: {
  issueId: number | null;
  similar: SimilarIssue[];
  statuses: NamedId[];
  onClose: () => void;
  onSubmit: (original: number, notes: string | null, statusId: number | null) => void;
}) {
  const [target, setTarget] = useState("");
  const [notes, setNotes] = useState("");
  const [statusId, setStatusId] = useState("");
  const open = issueId !== null;

  useEffect(() => {
    if (!open) return;
    setTarget(similar[0] ? String(similar[0].id) : "");
    setNotes("");
    setStatusId("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issueId]);

  const original = Number(target.replace(/^#/, ""));
  const valid = Number.isInteger(original) && original > 0 && original !== issueId;
  const submit = () => valid && onSubmit(original, notes.trim() || null, statusId ? Number(statusId) : null);

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={620}
      title={`#${issueId} 标为重复`}
      subtitle="建立“重复”关联：原任务关闭时，Redmine 会一并关闭这个任务"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            取消
          </Button>
          <Button variant="primary" icon={<Copy className="size-4" />} disabled={!valid} onClick={submit}>
            标为与 #{valid ? original : "…"} 重复
          </Button>
        </>
      }
    >
      <div className="space-y-4 p-5">
        {similar.length > 0 && (
          <div>
            <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">相似的任务</div>
            <div className="divide-y divide-line overflow-hidden rounded-xl border border-line">
              {similar.map((s) => (
                <label key={s.id} className="flex cursor-default items-center gap-3 px-3 py-2 hover:bg-hover">
                  <input type="radio" checked={target === String(s.id)} onChange={() => setTarget(String(s.id))} className="accent-accent" />
                  <span className="font-mono text-[12px] text-ink-3">#{s.id}</span>
                  <span className="min-w-0 flex-1 truncate text-[13px]">{s.subject}</span>
                  <span className="text-[11.5px] text-ink-3">{s.statusName}</span>
                  <span className="text-[11.5px] text-ink-3 tabular-nums">{Math.round(s.score * 100)}%</span>
                </label>
              ))}
            </div>
          </div>
        )}
        <div className="grid grid-cols-2 gap-4">
          <label className="block">
            <div className="mb-1 text-[12.5px] font-medium text-ink-2">与哪个任务重复</div>
            <Input value={target} onChange={(e) => setTarget(e.target.value)} placeholder="任务号，例如 1201" />
          </label>
          <label className="block">
            <div className="mb-1 text-[12.5px] font-medium text-ink-2">同时改为状态（可选）</div>
            <Select value={statusId} onChange={(e) => setStatusId(e.target.value)}>
              <option value="">不修改</option>
              {statuses.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </Select>
          </label>
        </div>
        <label className="block">
          <div className="mb-1 text-[12.5px] font-medium text-ink-2">说明（可选）</div>
          <Textarea rows={2} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder={`默认：与 #${valid ? original : "…"} 重复，请在 #${valid ? original : "…"} 中跟进。`} />
        </label>
      </div>
    </Modal>
  );
}
