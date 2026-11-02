import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { CalendarCheck, Copy, CornerUpLeft, Layers, Link2, ListOrdered, Sparkles } from "lucide-react";
import { api, errorText, type IssueView, type Plan } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { isMac, relativeTime } from "../lib/format";
import { Badge, Button, Card, CategoryBadge, Empty, SectionTitle } from "../components/ui";
import { Markdown } from "../components/Markdown";
import { ResolveDialog } from "../components/ResolveDialog";

export function PlanPage() {
  const { rev, setPage, setFocusIssue, secrets, intent, clearIntent } = useApp();
  const toast = useToast();
  const [plan, setPlan] = useState<Plan | null>(null);
  const [issues, setIssues] = useState<IssueView[]>([]);
  const [busy, setBusy] = useState(false);
  const [resolveFor, setResolveFor] = useState<IssueView[] | null>(null);

  useEffect(() => {
    api.latestPlan().then(setPlan).catch(() => {});
  }, []);
  useEffect(() => {
    api.listIssues({ includeDone: true }).then(setIssues).catch(() => {});
  }, [rev]);

  useEffect(() => {
    if (intent?.type === "generatePlan") {
      clearIntent();
      void generate();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intent]);

  const byId = useMemo(() => new Map(issues.map((i) => [i.id, i])), [issues]);
  const openCount = issues.filter((i) => i.inInbox).length;

  async function generate() {
    setBusy(true);
    try {
      setPlan(await api.suggestBatches(null));
    } catch (e) {
      toast.show({ tone: "error", title: "生成规划失败", body: errorText(e) });
    } finally {
      setBusy(false);
    }
  }

  async function link(ids: number[], relation: string) {
    const [first, ...rest] = ids;
    try {
      const errs = await api.linkIssues(first, rest, relation);
      toast.show(errs.length ? { tone: "warn", title: "部分关联失败", body: errs.join("\n") } : { tone: "success", title: `已在 Redmine 上关联 ${ids.length} 个任务` });
    } catch (e) {
      toast.show({ tone: "error", title: "关联失败", body: errorText(e) });
    }
  }

  function open(id: number) {
    setFocusIssue(id);
    setPage("inbox");
  }

  return (
    <div className="flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center gap-3 border-b border-line px-6", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">AI 规划</h2>
        {plan && <span className="text-[12.5px] text-ink-3">生成于 {relativeTime(plan.createdAt)} · 基于 {plan.issueIds.length} 个任务</span>}
        <div className="flex-1" data-tauri-drag-region />
        <Button variant="ai" icon={<Sparkles className="size-3.5" />} loading={busy} disabled={!secrets.ai && !secrets.redmine} onClick={() => void generate()}>
          {plan ? "重新规划" : "生成合并规划"}
        </Button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!plan ? (
          <Empty
            icon={<Layers className="size-6" />}
            title="让 AI 帮你安排今天的工作"
            desc={`AI 会分析收件箱里的 ${openCount} 个任务，找出可以一起处理的任务组和疑似重复的问题，并给出处理顺序和今日计划。`}
            action={
              <Button variant="ai" size="lg" icon={<Sparkles className="size-4" />} loading={busy} onClick={() => void generate()}>
                生成合并规划
              </Button>
            }
          />
        ) : (
          <div className="mx-auto grid max-w-[1200px] grid-cols-[1fr_360px] gap-6 p-6">
            <div className="min-w-0 space-y-6">
              <section>
                <SectionTitle icon={<Layers className="size-3.5" />}>可合并处理 · {plan.groups.length} 组</SectionTitle>
                {plan.groups.length === 0 && <Card className="p-5 text-[13px] text-ink-3">没有发现明显可以合并处理的任务。</Card>}
                <div className="space-y-3">
                  {plan.groups.map((g, gi) => {
                    const members = g.issueIds.map((id) => byId.get(id)).filter((x): x is IssueView => !!x);
                    const open_ = members.filter((m) => m.inInbox);
                    return (
                      <Card key={gi} className="overflow-hidden">
                        <div className="flex items-start gap-3 px-4 pt-4">
                          <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-accent-soft text-[13px] font-semibold text-accent-ink">{gi + 1}</div>
                          <div className="min-w-0 flex-1">
                            <div className="text-[14.5px] font-semibold">{g.title}</div>
                            <div className="selectable mt-1 text-[13px] leading-relaxed text-ink-2">{g.reason}</div>
                          </div>
                        </div>
                        {g.approach && (
                          <div className="selectable mx-4 mt-3 rounded-lg bg-sunken p-3 text-[12.5px] leading-relaxed text-ink-2">
                            <span className="font-semibold text-ink">处理思路：</span>
                            {g.approach}
                          </div>
                        )}
                        <ul className="mt-3 divide-y divide-line border-t border-line">
                          {members.map((m) => (
                            <li key={m.id} onClick={() => open(m.id)} className="flex items-center gap-3 px-4 py-2 hover:bg-hover">
                              <span className="font-mono text-[12px] text-ink-3">#{m.id}</span>
                              <span className={clsx("min-w-0 flex-1 truncate text-[13px]", !m.inInbox && "text-ink-3 line-through")}>{m.subject}</span>
                              <CategoryBadge category={m.analysis.category} />
                              <span className="w-16 truncate text-right text-[12px] text-ink-3">{m.authorName}</span>
                            </li>
                          ))}
                        </ul>
                        <div className="flex items-center gap-2 border-t border-line bg-sunken/40 px-4 py-2.5">
                          <Button size="sm" variant="ghost" icon={<Link2 className="size-3.5" />} onClick={() => void link(g.issueIds, "relates")}>
                            在 Redmine 建立关联
                          </Button>
                          <div className="flex-1" />
                          <Button size="sm" variant="primary" disabled={open_.length === 0} icon={<CornerUpLeft className="size-3.5" />} onClick={() => setResolveFor(open_)}>
                            一起处理完成 · 批量回派 {open_.length}
                          </Button>
                        </div>
                      </Card>
                    );
                  })}
                </div>
              </section>

              {plan.duplicates.length > 0 && (
                <section>
                  <SectionTitle icon={<Copy className="size-3.5" />}>疑似重复 · {plan.duplicates.length} 组</SectionTitle>
                  <div className="space-y-2">
                    {plan.duplicates.map((d, di) => (
                      <Card key={di} className="flex items-center gap-3 p-3.5">
                        <div className="flex shrink-0 gap-1">
                          {d.issueIds.map((id) => (
                            <button key={id} onClick={() => open(id)}>
                              <Badge tone="warn">#{id}</Badge>
                            </button>
                          ))}
                        </div>
                        <div className="selectable min-w-0 flex-1 text-[12.5px] text-ink-2">{d.reason}</div>
                        <Button size="sm" variant="ghost" onClick={() => void link(d.issueIds, "duplicates")}>
                          标记重复
                        </Button>
                      </Card>
                    ))}
                  </div>
                </section>
              )}
            </div>

            <div className="space-y-6">
              {plan.dailyPlan && (
                <section>
                  <SectionTitle icon={<CalendarCheck className="size-3.5" />}>今日计划</SectionTitle>
                  <Card className="ai-card p-4 text-[13px]">
                    <Markdown text={plan.dailyPlan} />
                  </Card>
                </section>
              )}
              <section>
                <SectionTitle icon={<ListOrdered className="size-3.5" />}>建议处理顺序</SectionTitle>
                <Card className="overflow-hidden">
                  <ol>
                    {plan.order.map((id, idx) => {
                      const m = byId.get(id);
                      if (!m) return null;
                      return (
                        <li key={id} onClick={() => open(id)} className="flex items-center gap-2.5 border-b border-line px-3.5 py-2 last:border-b-0 hover:bg-hover">
                          <span className="w-5 text-right text-[12px] text-ink-3 tabular-nums">{idx + 1}</span>
                          <span className={clsx("min-w-0 flex-1 truncate text-[12.5px]", !m.inInbox && "text-ink-3 line-through")}>{m.subject}</span>
                          {m.analysis.urgency === "高" && <span className="size-1.5 shrink-0 rounded-full bg-accent" />}
                        </li>
                      );
                    })}
                  </ol>
                </Card>
              </section>
            </div>
          </div>
        )}
      </div>

      <ResolveDialog open={!!resolveFor} onClose={() => setResolveFor(null)} issues={resolveFor ?? []} />
    </div>
  );
}
