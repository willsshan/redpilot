import { useEffect, useState } from "react";
import clsx from "clsx";
import { AlertTriangle, CheckCircle2, Clock, Copy, FileText, Flame, Inbox, Mail, Sparkles } from "lucide-react";
import { api, errorText, type ResolutionRecord } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { categoryColor, dateTime, isMac } from "../lib/format";
import { Badge, Button, Card, SectionTitle } from "../components/ui";
import { Markdown } from "../components/Markdown";
import type { ReactNode } from "react";

export function DashboardPage() {
  const { stats, rev } = useApp();
  const toast = useToast();
  const [report, setReport] = useState("");
  const [reportDays, setReportDays] = useState<number | null>(null);
  const [records, setRecords] = useState<ResolutionRecord[]>([]);

  useEffect(() => {
    api.recentResolutions(30).then(setRecords).catch(() => {});
  }, [rev]);

  async function gen(days: number) {
    setReportDays(days);
    try {
      setReport(await api.workReport(days));
    } catch (e) {
      toast.show({ tone: "error", title: "生成报告失败", body: errorText(e) });
    } finally {
      setReportDays(null);
    }
  }

  const s = stats;
  const maxDaily = Math.max(1, ...(s?.dailyResolved.map((d) => d[1]) ?? [1]));

  return (
    <div className="flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center border-b border-line px-6", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">统计与报告</h2>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[1200px] space-y-6 p-6">
          <div className="grid grid-cols-6 gap-3">
            <Stat icon={<Inbox className="size-4" />} label="待办" value={s?.openTotal ?? 0} />
            <Stat icon={<Mail className="size-4" />} label="未读" value={s?.unread ?? 0} tone={s?.unread ? "info" : undefined} />
            <Stat icon={<AlertTriangle className="size-4" />} label="已逾期" value={s?.overdue ?? 0} tone={s?.overdue ? "accent" : undefined} />
            <Stat icon={<Flame className="size-4" />} label="高优先级" value={s?.highPriority ?? 0} tone={s?.highPriority ? "warn" : undefined} />
            <Stat icon={<CheckCircle2 className="size-4" />} label="近 7 天回派" value={s?.resolved7d ?? 0} tone="ok" />
            <Stat icon={<Clock className="size-4" />} label="近 7 天工时" value={`${(s?.spentHours7d ?? 0).toFixed(1)}h`} />
          </div>

          <div className="grid grid-cols-3 gap-4">
            <Card className="p-4">
              <SectionTitle>待办分类</SectionTitle>
              <Bars data={s?.byCategory ?? []} color={(k) => categoryColor(k === "未分类" ? null : k)} />
            </Card>
            <Card className="p-4">
              <SectionTitle>按项目</SectionTitle>
              <Bars data={(s?.byProject ?? []).slice(0, 8)} color={() => "var(--info)"} />
            </Card>
            <Card className="p-4">
              <SectionTitle right={<span className="text-[11.5px] text-ink-3">30 天 AI 用量 {((s?.aiTokens30d ?? 0) / 1000).toFixed(1)}k tokens</span>}>近 14 天回派</SectionTitle>
              <div className="flex h-[150px] items-end gap-1.5 pt-2">
                {(s?.dailyResolved ?? []).map(([d, n]) => (
                  <div key={d} className="group flex flex-1 flex-col items-center gap-1">
                    <span className="text-[10.5px] text-ink-3 opacity-0 group-hover:opacity-100">{n}</span>
                    <div className="w-full rounded-t bg-accent/80 transition-all group-hover:bg-accent" style={{ height: `${Math.max(3, (n / maxDaily) * 110)}px`, opacity: n ? 1 : 0.25 }} />
                    <span className="text-[9.5px] text-ink-3">{d.slice(3)}</span>
                  </div>
                ))}
              </div>
            </Card>
          </div>

          <div className="grid grid-cols-[1fr_400px] gap-4">
            <Card className="flex min-h-[320px] flex-col">
              <div className="flex items-center gap-2 border-b border-line px-4 py-3">
                <Sparkles className="size-4 text-accent" />
                <span className="text-[13px] font-semibold">AI 工作报告</span>
                <div className="flex-1" />
                <Button size="sm" loading={reportDays === 1} onClick={() => void gen(1)}>
                  日报
                </Button>
                <Button size="sm" variant="ai" loading={reportDays === 7} onClick={() => void gen(7)}>
                  周报
                </Button>
                {report && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<Copy className="size-3.5" />}
                    onClick={() => void navigator.clipboard.writeText(report).then(() => toast.show({ tone: "success", title: "已复制到剪贴板" }))}
                  >
                    复制
                  </Button>
                )}
              </div>
              <div className="flex-1 p-4 text-[13px]">
                {report ? (
                  <Markdown text={report} />
                ) : (
                  <div className="flex h-full flex-col items-center justify-center text-center text-[12.5px] text-ink-3">
                    <FileText className="mb-2 size-6" />
                    根据你通过 RedPilot 回派的处置说明，一键生成日报 / 周报
                  </div>
                )}
              </div>
            </Card>

            <Card className="flex flex-col">
              <div className="border-b border-line px-4 py-3 text-[13px] font-semibold">最近回派 · 30 天 {records.length}</div>
              <ul className="max-h-[420px] flex-1 divide-y divide-line overflow-y-auto">
                {records.length === 0 && <li className="p-4 text-[12.5px] text-ink-3">暂无记录</li>}
                {records.map((r, i) => (
                  <li key={i} className="px-4 py-2.5">
                    <div className="flex items-center gap-2 text-[11.5px] text-ink-3">
                      <span className="font-mono">#{r.issueId}</span>
                      <span>{dateTime(r.createdAt)}</span>
                      {r.via === "mcp" && <Badge tone="info">MCP</Badge>}
                      {r.spentHours ? <span className="ml-auto">{r.spentHours}h</span> : null}
                    </div>
                    <div className="truncate text-[13px]">{r.subject}</div>
                    <div className="truncate text-[12px] text-ink-3">→ {r.assignedToName || "—"} · {r.statusName}</div>
                  </li>
                ))}
              </ul>
            </Card>
          </div>
        </div>
      </div>
    </div>
  );
}

function Stat({ icon, label, value, tone }: { icon: ReactNode; label: string; value: number | string; tone?: "accent" | "warn" | "ok" | "info" }) {
  return (
    <Card className="p-4">
      <div
        className={clsx(
          "mb-3 flex size-8 items-center justify-center rounded-lg",
          tone === "accent" && "bg-accent-soft text-accent-ink",
          tone === "warn" && "bg-warn-soft text-warn",
          tone === "ok" && "bg-ok-soft text-ok",
          tone === "info" && "bg-info-soft text-info",
          !tone && "bg-sunken text-ink-3",
        )}
      >
        {icon}
      </div>
      <div className="text-[22px] leading-none font-semibold tracking-tight tabular-nums">{value}</div>
      <div className="mt-1.5 text-[12px] text-ink-3">{label}</div>
    </Card>
  );
}

function Bars({ data, color }: { data: [string, number][]; color: (k: string) => string }) {
  const max = Math.max(1, ...data.map((d) => d[1]));
  if (data.length === 0) return <div className="py-10 text-center text-[12.5px] text-ink-3">暂无数据</div>;
  return (
    <div className="space-y-2">
      {data.map(([k, n]) => (
        <div key={k} className="flex items-center gap-2 text-[12.5px]">
          <span className="w-20 shrink-0 truncate text-ink-2" title={k}>
            {k}
          </span>
          <div className="h-2 flex-1 overflow-hidden rounded-full bg-sunken">
            <div className="h-full rounded-full" style={{ width: `${(n / max) * 100}%`, backgroundColor: color(k) }} />
          </div>
          <span className="w-6 text-right text-ink-3 tabular-nums">{n}</span>
        </div>
      ))}
    </div>
  );
}
