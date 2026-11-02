import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api, onAppEvent, type AppConfig, type McpStatus, type SecretStatus, type Stats } from "./api";
import { useToast } from "./toast";

export type Page = "inbox" | "plan" | "dashboard" | "settings";

interface AppState {
  page: Page;
  setPage: (p: Page) => void;
  config: AppConfig | null;
  reloadConfig: () => Promise<void>;
  secrets: SecretStatus;
  reloadSecrets: () => Promise<void>;
  stats: Stats | null;
  mcp: McpStatus | null;
  reloadMcp: () => Promise<void>;
  syncing: boolean;
  analyzing: number;
  /** 任务数据有变化时递增，页面据此刷新 */
  rev: number;
  bump: () => void;
  sync: () => Promise<void>;
  /** 收件箱要聚焦的任务（从规划页跳转） */
  focusIssue: number | null;
  setFocusIssue: (id: number | null) => void;
}

const Ctx = createContext<AppState | null>(null);

export function useApp(): AppState {
  const c = useContext(Ctx);
  if (!c) throw new Error("AppProvider missing");
  return c;
}

export function AppProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  // 支持 ?page=settings 直接打开指定页面
  const [page, setPage] = useState<Page>(() => {
    const p = new URLSearchParams(window.location.search).get("page");
    return p === "plan" || p === "dashboard" || p === "settings" ? p : "inbox";
  });
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [secrets, setSecrets] = useState<SecretStatus>({ redmine: false, ai: false });
  const [stats, setStats] = useState<Stats | null>(null);
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [analyzing, setAnalyzing] = useState(0);
  const [rev, setRev] = useState(0);
  const [focusIssue, setFocusIssue] = useState<number | null>(null);

  const bump = useCallback(() => setRev((r) => r + 1), []);
  const reloadConfig = useCallback(async () => setConfig(await api.getConfig()), []);
  const reloadSecrets = useCallback(async () => setSecrets(await api.secretStatus()), []);
  const reloadMcp = useCallback(async () => setMcp(await api.mcpStatus()), []);

  const sync = useCallback(async () => {
    try {
      await api.syncNow();
    } catch {
      // 失败信息通过 syncFailed 事件提示
    }
  }, []);

  useEffect(() => {
    void reloadConfig();
    void reloadSecrets();
    void reloadMcp();
    // MCP 服务在后台启动，稍后再刷新一次状态
    const t = window.setTimeout(() => void reloadMcp(), 1500);
    return () => window.clearTimeout(t);
  }, [reloadConfig, reloadSecrets, reloadMcp]);

  useEffect(() => {
    api.stats().then(setStats).catch(() => {});
  }, [rev]);

  useEffect(() => {
    const un = onAppEvent((e) => {
      switch (e.type) {
        case "syncStarted":
          setSyncing(true);
          break;
        case "syncFinished": {
          setSyncing(false);
          bump();
          const n = e.report.newIds.length;
          if (n > 0) toast.show({ tone: "success", title: `同步完成，新增 ${n} 个任务`, body: `当前共 ${e.report.total} 个待办` });
          break;
        }
        case "syncFailed":
          setSyncing(false);
          toast.show({ tone: "error", title: "同步失败", body: e.message.replace(/^\[[A-Z_]+\]\s*/, "") });
          break;
        case "analysisStarted":
          setAnalyzing(e.count);
          break;
        case "analysisFinished":
          setAnalyzing(0);
          bump();
          break;
        case "analysisFailed":
          setAnalyzing(0);
          toast.show({ tone: "error", title: "AI 分析失败", body: e.message.replace(/^\[[A-Z_]+\]\s*/, "") });
          break;
        case "issuesChanged":
          bump();
          break;
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [bump, toast]);

  return (
    <Ctx.Provider
      value={{
        page,
        setPage,
        config,
        reloadConfig,
        secrets,
        reloadSecrets,
        stats,
        mcp,
        reloadMcp,
        syncing,
        analyzing,
        rev,
        bump,
        sync,
        focusIssue,
        setFocusIssue,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}
