import clsx from "clsx";
import { BarChart3, Inbox, RefreshCw, Settings, Sparkles, Plug } from "lucide-react";
import type { ReactNode } from "react";
import { useApp, type Page } from "../lib/store";
import { isMac, relativeTime } from "../lib/format";
import { useEffect, useState } from "react";

export function Sidebar() {
  const { page, setPage, stats, syncing, sync, mcp, analyzing, secrets, config } = useApp();
  const [, tick] = useState(0);
  // 每 30 秒刷新“x 分钟前同步”
  useEffect(() => {
    const t = window.setInterval(() => tick((x) => x + 1), 30000);
    return () => window.clearInterval(t);
  }, []);

  const configured = !!config?.redmine.baseUrl && secrets.redmine;

  return (
    <aside className="flex w-[232px] shrink-0 flex-col border-r border-line bg-sunken">
      {/* macOS 交通灯区域 + 可拖动标题栏 */}
      <div data-tauri-drag-region className={clsx("shrink-0", isMac ? "h-[52px]" : "h-3")} />

      <div className="flex items-center gap-2.5 px-4 pb-4" data-tauri-drag-region>
        <img src="/logo.png" alt="" className="size-8 rounded-[9px] shadow-sm" draggable={false} />
        <div className="leading-tight" data-tauri-drag-region>
          <div className="text-[14.5px] font-semibold tracking-tight text-ink">RedPilot</div>
          <div className="text-[11.5px] text-ink-3">Redmine 工单助手</div>
        </div>
      </div>

      <nav className="flex flex-col gap-0.5 px-2.5">
        <NavItem icon={<Inbox className="size-4" />} label="收件箱" page="inbox" current={page} onClick={setPage} count={stats?.openTotal} dot={!!stats?.unread} />
        <NavItem icon={<Sparkles className="size-4" />} label="AI 规划" page="plan" current={page} onClick={setPage} />
        <NavItem icon={<BarChart3 className="size-4" />} label="统计与报告" page="dashboard" current={page} onClick={setPage} />
        <NavItem icon={<Settings className="size-4" />} label="设置" page="settings" current={page} onClick={setPage} />
      </nav>

      <div className="flex-1" />

      {analyzing > 0 && (
        <div className="mx-3 mb-2 flex items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2 text-[12px] text-ink-2">
          <Sparkles className="size-3.5 animate-pulse text-accent" />
          AI 正在分析 {analyzing} 个任务…
        </div>
      )}

      <div className="space-y-1 border-t border-line px-3 py-3">
        <button
          onClick={() => configured && void sync()}
          disabled={!configured || syncing}
          className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12px] text-ink-2 hover:bg-hover disabled:hover:bg-transparent"
          title="立即同步"
        >
          <RefreshCw className={clsx("size-3.5", syncing && "animate-spin text-accent")} />
          <span className="flex-1 truncate">
            {!configured ? "未连接 Redmine" : syncing ? "正在同步…" : `同步于 ${relativeTime(stats?.lastSyncAt)}`}
          </span>
        </button>
        <button
          onClick={() => setPage("settings")}
          className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12px] text-ink-2 hover:bg-hover"
          title="MCP 服务"
        >
          <Plug className="size-3.5" />
          <span className="flex-1 truncate">MCP {mcp?.running ? `:${mcp.port}` : mcp?.enabled ? "启动失败" : "已关闭"}</span>
          <span className={clsx("size-1.5 rounded-full", mcp?.running ? "bg-ok" : mcp?.enabled ? "bg-accent" : "bg-ink-3")} />
        </button>
      </div>
    </aside>
  );
}

function NavItem({
  icon,
  label,
  page,
  current,
  onClick,
  count,
  dot,
}: {
  icon: ReactNode;
  label: string;
  page: Page;
  current: Page;
  onClick: (p: Page) => void;
  count?: number;
  dot?: boolean;
}) {
  const active = page === current;
  return (
    <button
      onClick={() => onClick(page)}
      className={clsx(
        "flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-[13px] transition-colors",
        active ? "bg-panel font-medium text-ink shadow-[0_1px_2px_rgb(0_0_0/0.06)]" : "text-ink-2 hover:bg-hover hover:text-ink",
      )}
    >
      <span className={clsx(active ? "text-accent" : "text-ink-3")}>{icon}</span>
      <span className="flex-1 text-left">{label}</span>
      {dot && <span className="size-1.5 rounded-full bg-accent" />}
      {count !== undefined && count > 0 && <span className="text-[11.5px] text-ink-3 tabular-nums">{count}</span>}
    </button>
  );
}
