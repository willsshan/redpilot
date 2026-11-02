import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import clsx from "clsx";
import {
  BarChart3,
  Bot,
  CalendarDays,
  ExternalLink,
  FileText,
  Inbox,
  Kanban,
  LayoutList,
  Plug,
  RefreshCw,
  Search,
  Server,
  Settings,
  Sparkles,
  Tags,
  Workflow,
} from "lucide-react";
import { api, errorText, type IssueView } from "../lib/api";
import { useApp, type Intent, type Page } from "../lib/store";
import { useToast } from "../lib/toast";
import { categoryColor, modKey } from "../lib/format";
import { Kbd } from "./ui";

interface Item {
  id: string;
  group: "任务" | "命令" | "页面";
  title: string;
  subtitle?: string;
  keywords?: string;
  icon: ReactNode;
  run: () => void;
}

/** 子串匹配优先，其次按顺序出现的字符（容忍中间有间隔） */
function score(text: string, query: string): number {
  const t = text.toLowerCase();
  const q = query.toLowerCase().trim();
  if (!q) return 1;
  const idx = t.indexOf(q);
  if (idx >= 0) return 1000 - idx;
  let from = 0;
  let gaps = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const f = t.indexOf(ch, from);
    if (f < 0) return 0;
    gaps += f - from;
    from = f + 1;
  }
  return Math.max(1, 200 - gaps);
}

export function CommandPalette() {
  const { setPage, dispatch, sync, config, setFocusIssue } = useApp();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [issues, setIssues] = useState<IssueView[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    const openEvt = () => setOpen(true);
    window.addEventListener("keydown", h);
    window.addEventListener("redpilot:palette", openEvt);
    return () => {
      window.removeEventListener("keydown", h);
      window.removeEventListener("redpilot:palette", openEvt);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    api.listIssues({}).then(setIssues).catch(() => setIssues([]));
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }, [open]);

  const close = () => setOpen(false);
  const go = (p: Page, intent?: Intent) => {
    dispatch(p, intent);
    close();
  };

  const commands: Item[] = useMemo(() => {
    const section = (id: string, title: string, icon: ReactNode, keywords: string): Item => ({
      id: `settings-${id}`,
      group: "页面",
      title: `设置 · ${title}`,
      keywords: `settings ${keywords}`,
      icon,
      run: () => go("settings", { type: "settingsSection", id }),
    });
    const list: Item[] = [
      { id: "sync", group: "命令", title: "立即同步", subtitle: "从 Redmine 拉取最新任务", keywords: "sync refresh tongbu", icon: <RefreshCw className="size-4" />, run: () => (void sync(), close()) },
      {
        id: "analyze",
        group: "命令",
        title: "AI 分析全部任务",
        subtitle: "分类、摘要、缺失信息、处理思路",
        keywords: "ai analyze fenxi",
        icon: <Sparkles className="size-4" />,
        run: () => {
          close();
          api
            .analyze(null, false)
            .then((r) => toast.show({ tone: "success", title: r.analyzed.length ? `已分析 ${r.analyzed.length} 个任务` : "所有任务都已是最新分析结果" }))
            .catch((e) => toast.show({ tone: "error", title: "AI 分析失败", body: errorText(e) }));
        },
      },
      { id: "plan", group: "命令", title: "生成合并规划", subtitle: "找出可以一起做的任务", keywords: "plan batch guihua hebing", icon: <Sparkles className="size-4" />, run: () => go("plan", { type: "generatePlan" }) },
      { id: "daily", group: "命令", title: "生成日报", keywords: "report daily ribao", icon: <FileText className="size-4" />, run: () => go("dashboard", { type: "report", days: 1 }) },
      { id: "weekly", group: "命令", title: "生成周报", keywords: "report weekly zhoubao", icon: <CalendarDays className="size-4" />, run: () => go("dashboard", { type: "report", days: 7 }) },
      { id: "board", group: "命令", title: "切换到看板视图", keywords: "board kanban", icon: <Kanban className="size-4" />, run: () => go("inbox", { type: "inboxView", view: "board" }) },
      { id: "list", group: "命令", title: "切换到列表视图", keywords: "list liebiao", icon: <LayoutList className="size-4" />, run: () => go("inbox", { type: "inboxView", view: "list" }) },
      { id: "inbox", group: "页面", title: "收件箱", keywords: "inbox shoujianxiang", icon: <Inbox className="size-4" />, run: () => go("inbox") },
      { id: "plan-page", group: "页面", title: "AI 规划", keywords: "plan guihua", icon: <Sparkles className="size-4" />, run: () => go("plan") },
      { id: "dashboard", group: "页面", title: "统计与报告", keywords: "dashboard stats tongji", icon: <BarChart3 className="size-4" />, run: () => go("dashboard") },
      { id: "settings", group: "页面", title: "设置", keywords: "settings shezhi", icon: <Settings className="size-4" />, run: () => go("settings") },
      section("redmine", "Redmine 连接", <Server className="size-4" />, "redmine api key"),
      section("sync", "同步与提醒", <RefreshCw className="size-4" />, "sync notify tongbu tixing"),
      section("workflow", "工作流", <Workflow className="size-4" />, "workflow status zhuangtai"),
      section("ai", "AI 模型", <Bot className="size-4" />, "ai deepseek model prompt"),
      section("mcp", "MCP 服务", <Plug className="size-4" />, "mcp token"),
      section("rules", "分类规则", <Tags className="size-4" />, "rules category fenlei"),
    ];
    if (config?.redmine.baseUrl) {
      list.push({
        id: "open-redmine",
        group: "命令",
        title: "在浏览器中打开 Redmine",
        subtitle: config.redmine.baseUrl,
        keywords: "open browser",
        icon: <ExternalLink className="size-4" />,
        run: () => (void api.openUrl(config.redmine.baseUrl), close()),
      });
    }
    return list;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config?.redmine.baseUrl, sync, toast, dispatch]);

  const results = useMemo(() => {
    const q = query.trim();
    const idQuery = /^#?\d+$/.test(q) ? q.replace("#", "") : null;
    const issueItems: { item: Item; s: number }[] = issues
      .map((i) => {
        const s = idQuery
          ? String(i.id).startsWith(idQuery)
            ? 2000 - String(i.id).length
            : 0
          : q
            ? Math.max(
                score(i.subject, q) * 1.2,
                score(`${i.authorName} ${i.projectName} ${i.analysis.category ?? ""} ${i.analysis.module ?? ""}`, q),
                score(i.analysis.summary ?? "", q) * 0.8,
              )
            : 0;
        return {
          s,
          item: {
            id: `issue-${i.id}`,
            group: "任务" as const,
            title: i.subject,
            subtitle: `#${i.id} · ${i.authorName} · ${i.statusName}${i.changeHint ? ` · ${i.changeHint}` : ""}`,
            icon: <span className="size-2 rounded-full" style={{ backgroundColor: categoryColor(i.analysis.category) }} />,
            run: () => {
              setFocusIssue(i.id);
              setPage("inbox");
              close();
            },
          },
        };
      })
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 8);
    const cmdItems = commands
      .map((c) => ({ item: c, s: q ? Math.max(score(c.title, q) * 1.5, score(`${c.subtitle ?? ""} ${c.keywords ?? ""}`, q)) : 1 }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s);
    // 有关键词时任务排在前面；没有关键词时只显示命令和页面
    return [...issueItems, ...cmdItems].map((x) => x.item);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, issues, commands]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (!open) return null;

  function onKey(e: ReactKeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(results.length - 1, a + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      results[active]?.run();
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  let lastGroup = "";
  return (
    <div className="fixed inset-0 z-[90] flex items-start justify-center bg-black/25 px-6 pt-[14vh] backdrop-blur-[2px]" onMouseDown={close}>
      <div className="animate-in w-full max-w-[620px] overflow-hidden rounded-2xl border border-line bg-panel shadow-pop" onMouseDown={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2.5 border-b border-line px-4">
          <Search className="size-4 shrink-0 text-ink-3" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="搜索任务（标题、提出人、#任务号），或输入命令…"
            className="h-12 flex-1 bg-transparent text-[14px] text-ink placeholder:text-ink-3 focus:outline-none"
          />
          <Kbd>Esc</Kbd>
        </div>
        <div ref={listRef} className="max-h-[52vh] overflow-y-auto p-1.5">
          {results.length === 0 && <div className="px-3 py-8 text-center text-[13px] text-ink-3">没有匹配的任务或命令</div>}
          {results.map((r, idx) => {
            const header = r.group !== lastGroup ? r.group : null;
            lastGroup = r.group;
            return (
              <div key={r.id}>
                {header && <div className="px-2.5 pt-2 pb-1 text-[11.5px] font-semibold text-ink-3">{header}</div>}
                <button
                  data-index={idx}
                  onMouseMove={() => setActive(idx)}
                  onClick={r.run}
                  className={clsx("flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left", idx === active ? "bg-accent-soft/70" : "hover:bg-hover")}
                >
                  <span className={clsx("flex size-6 shrink-0 items-center justify-center", idx === active ? "text-accent" : "text-ink-3")}>{r.icon}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13.5px] text-ink">{r.title}</span>
                    {r.subtitle && <span className="block truncate text-[12px] text-ink-3">{r.subtitle}</span>}
                  </span>
                  {idx === active && <Kbd>↵</Kbd>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="flex items-center gap-3 border-t border-line bg-sunken/50 px-4 py-2 text-[11.5px] text-ink-3">
          <span className="flex items-center gap-1">
            <Kbd>↑</Kbd>
            <Kbd>↓</Kbd> 选择
          </span>
          <span className="flex items-center gap-1">
            <Kbd>↵</Kbd> 执行
          </span>
          <span className="ml-auto flex items-center gap-1">
            <Kbd>{modKey}</Kbd>
            <Kbd>K</Kbd> 随时打开
          </span>
        </div>
      </div>
    </div>
  );
}
