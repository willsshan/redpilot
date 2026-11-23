import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import clsx from "clsx";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  ChevronDown,
  Maximize2,
  Minimize2,
  PlugZap,
  RotateCcw,
  SendHorizontal,
  ShieldCheck,
  Sparkles,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import {
  api,
  errorText,
  type ActionExecutionResult,
  type ChatHistoryItem,
  type ChatToolStep,
  type PendingAction,
} from "../lib/api";
import { modKey } from "../lib/format";
import { getRecentProjects, recordRecentProject } from "../lib/recentProjects";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { Markdown } from "./Markdown";
import { Badge, Button, IconButton, Spinner } from "./ui";

const AUTO_WRITE_KEY = "rp.chat.autoWrite";

interface UiMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  /** 发送给下一轮模型时的补充历史（例如用户已确认执行或取消了操作） */
  historyNote?: string;
  steps?: ChatToolStep[];
  pendingActions?: PendingAction[];
  actionState?: "pending" | "executing" | "done" | "cancelled";
  actionResults?: ActionExecutionResult[];
  error?: boolean;
}

const SUGGESTIONS: { label: string; prompt: string; fillOnly?: boolean }[] = [
  { label: "查看我参与的项目", prompt: "列出我参与的所有项目" },
  { label: "最近项目的问题", prompt: "查看我最近使用的项目中未关闭的问题" },
  { label: "逾期与高优先级", prompt: "汇总我收件箱里已逾期和高优先级的任务" },
  { label: "待分派任务建议", prompt: "看看分派台有哪些待分派的任务，并给出推荐负责人" },
  { label: "关闭指定问题…", prompt: "帮我关闭 #", fillOnly: true },
];

function loadAutoWrite(): boolean {
  try {
    return localStorage.getItem(AUTO_WRITE_KEY) === "1";
  } catch {
    return false;
  }
}

function uid(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

/** 全局 AI 对话框：可通过自然语言直接查询和操作 Redmine（查看项目问题、关闭问题、指派、备注、新建等） */
export function AiChatDrawer() {
  const { chatOpen, setChatOpen, toggleChat, config, secrets, bump, setPage, setFocusIssue, dispatch } = useApp();
  const toast = useToast();
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [wide, setWide] = useState(false);
  const [autoWrite, setAutoWrite] = useState<boolean>(loadAutoWrite);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  const redmineReady = !!config?.redmine.baseUrl && secrets.redmine;
  const aiReady = !!config?.ai.enabled && (secrets.ai || !config.ai.baseUrl.includes("deepseek.com"));

  // 全局快捷键：⌘J / Ctrl+J 开关 AI 对话框；打开时按 Esc 关闭
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "j") {
        e.preventDefault();
        toggleChat();
      } else if (e.key === "Escape" && chatOpen && !e.isComposing) {
        setChatOpen(false);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [chatOpen, setChatOpen, toggleChat]);

  useEffect(() => {
    if (chatOpen) window.setTimeout(() => inputRef.current?.focus(), 60);
  }, [chatOpen]);

  useEffect(() => {
    if (chatOpen) bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, busy, chatOpen]);

  function toggleAutoWrite() {
    const next = !autoWrite;
    setAutoWrite(next);
    try {
      localStorage.setItem(AUTO_WRITE_KEY, next ? "1" : "0");
    } catch {
      // 忽略存储异常
    }
  }

  function toHistory(list: UiMessage[]): ChatHistoryItem[] {
    return list
      .filter((m) => !m.error)
      .map((m) => ({
        role: m.role,
        content: m.historyNote ? `${m.content}\n\n[${m.historyNote}]` : m.content,
      }));
  }

  async function send(text?: string) {
    const q = (text ?? input).trim();
    if (!q || busy) return;
    const userMsg: UiMessage = { id: uid(), role: "user", content: q };
    const nextHistory = [...messages, userMsg];
    setMessages(nextHistory);
    setInput("");
    setBusy(true);
    try {
      const reply = await api.chatSend({
        messages: toHistory(nextHistory),
        autoExecuteWrites: autoWrite,
        recentProjectIds: getRecentProjects(),
      });
      reply.touchedProjectIds.forEach((pid) => recordRecentProject(pid));
      if (reply.steps.some((s) => s.write && s.ok)) bump();
      setMessages((prev) => [
        ...prev,
        {
          id: uid(),
          role: "assistant",
          content: reply.content || (reply.pendingActions.length ? "请核对以下操作并点击确认执行。" : "已完成。"),
          steps: reply.steps,
          pendingActions: reply.pendingActions,
          actionState: reply.pendingActions.length > 0 ? "pending" : undefined,
        },
      ]);
    } catch (e) {
      const msg = errorText(e);
      setMessages((prev) => [
        ...prev,
        {
          id: uid(),
          role: "assistant",
          content: `请求失败：${msg}`,
          error: true,
        },
      ]);
    } finally {
      setBusy(false);
      window.setTimeout(() => inputRef.current?.focus(), 20);
    }
  }

  async function confirmActions(msgId: string, actions: PendingAction[]) {
    setMessages((prev) => prev.map((m) => (m.id === msgId ? { ...m, actionState: "executing" } : m)));
    try {
      const results = await api.chatExecuteActions(actions);
      const okCount = results.filter((r) => r.ok).length;
      const failCount = results.length - okCount;
      results.forEach((r) => r.projectIds.forEach((pid) => recordRecentProject(pid)));
      if (okCount > 0) bump();
      const note = results.map((r) => `${r.ok ? "已执行" : "失败"}：${r.title}（${r.summary}）`).join("；");
      setMessages((prev) =>
        prev.map((m) =>
          m.id === msgId
            ? { ...m, actionState: "done", actionResults: results, historyNote: `用户已确认执行：${note}` }
            : m,
        ),
      );
      toast.show({
        tone: failCount > 0 ? (okCount > 0 ? "warn" : "error") : "success",
        title: failCount === 0 ? `已执行 ${okCount} 项操作` : `成功 ${okCount} 项，失败 ${failCount} 项`,
        body: results.map((r) => r.summary).join("\n"),
      });
    } catch (e) {
      const msg = errorText(e);
      setMessages((prev) => prev.map((m) => (m.id === msgId ? { ...m, actionState: "pending" } : m)));
      toast.show({ tone: "error", title: "执行操作失败", body: msg });
    }
  }

  function cancelActions(msgId: string) {
    setMessages((prev) =>
      prev.map((m) => (m.id === msgId ? { ...m, actionState: "cancelled", historyNote: "用户取消了上述待确认操作" } : m)),
    );
  }

  function openIssue(id: number) {
    setFocusIssue(id);
    setPage("inbox");
    setChatOpen(false);
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  if (!chatOpen) return null;

  return (
    <div className="fixed inset-0 z-[70] flex justify-end">
      <div className="absolute inset-0 bg-black/15 backdrop-blur-[1px]" onClick={() => setChatOpen(false)} />
      <aside
        className={clsx(
          "animate-slide-in relative flex h-full flex-col border-l border-line bg-canvas shadow-pop transition-[width] duration-150",
          wide ? "w-[min(860px,88vw)]" : "w-[min(560px,92vw)]",
        )}
      >
        {/* 顶栏 */}
        <header className="flex h-[52px] shrink-0 items-center gap-2 border-b border-line px-4">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-gradient-to-br from-[#e5484d] to-[#8e4ec6] text-white shadow-sm">
            <Sparkles className="size-3.5" />
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 text-[14px] leading-tight font-semibold text-ink">
              AI 对话助手
            </div>
            <div className="truncate text-[11px] text-ink-3">通过对话直接查询和操作 Redmine</div>
          </div>
          <div className="flex-1" />
          <button
            type="button"
            onClick={toggleAutoWrite}
            title={
              autoWrite
                ? "当前：AI 可直接执行关闭、指派、改状态等写操作（点击切换为执行前确认）"
                : "当前：关闭、指派、改状态等写操作会先展示确认卡片（点击切换为自动执行）"
            }
            className={clsx(
              "inline-flex h-7 items-center gap-1 rounded-full border px-2.5 text-[11.5px] font-medium transition-colors",
              autoWrite
                ? "border-warn/50 bg-warn-soft text-warn"
                : "border-line bg-panel text-ink-2 hover:bg-hover",
            )}
          >
            {autoWrite ? <Zap className="size-3" /> : <ShieldCheck className="size-3 text-ok" />}
            {autoWrite ? "自动执行写操作" : "写操作需确认"}
          </button>
          {messages.length > 0 && (
            <IconButton title="清空对话" disabled={busy} onClick={() => setMessages([])}>
              <RotateCcw className="size-3.5" />
            </IconButton>
          )}
          <IconButton title={wide ? "收窄窗口" : "加宽窗口"} onClick={() => setWide((w) => !w)}>
            {wide ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
          </IconButton>
          <IconButton title="关闭 (Esc)" onClick={() => setChatOpen(false)}>
            <X className="size-4" />
          </IconButton>
        </header>

        {/* 消息区 */}
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
          {!redmineReady || !aiReady ? (
            <div className="flex h-full flex-col items-center justify-center px-6 text-center">
              <div className="mb-3 flex size-11 items-center justify-center rounded-2xl bg-sunken text-ink-3">
                <PlugZap className="size-5" />
              </div>
              <div className="text-[14.5px] font-semibold text-ink">
                {!redmineReady ? "请先连接 Redmine" : "请先配置并启用 AI 模型"}
              </div>
              <div className="mt-1 max-w-xs text-[12.5px] text-ink-3">
                {!redmineReady
                  ? "填写 Redmine 地址和 API Key 后，即可在这里用自然语言查询和操作项目问题。"
                  : "在设置中启用 AI 并填写 API Key（如 DeepSeek 或本地 Ollama）后即可使用对话助手。"}
              </div>
              <Button
                variant="primary"
                className="mt-4"
                onClick={() => {
                  setChatOpen(false);
                  dispatch("settings", { type: "settingsSection", id: !redmineReady ? "redmine" : "ai" });
                }}
              >
                前往设置
              </Button>
            </div>
          ) : messages.length === 0 ? (
            <div className="flex h-full flex-col justify-center px-2">
              <div className="mb-5 text-center">
                <div className="mx-auto mb-3 flex size-11 items-center justify-center rounded-2xl bg-gradient-to-br from-[#e5484d]/15 to-[#8e4ec6]/15 text-accent">
                  <Sparkles className="size-5" />
                </div>
                <div className="text-[15px] font-semibold text-ink">想对 Redmine 做什么？</div>
                <div className="mt-1 text-[12.5px] text-ink-3">
                  可以直接查任意项目的问题、筛选逾期/未指派任务，或直接关闭、指派、备注和新建问题。
                </div>
              </div>
              <div className="grid grid-cols-1 gap-2">
                {SUGGESTIONS.map((s) => (
                  <button
                    key={s.label}
                    type="button"
                    onClick={() => {
                      if (s.fillOnly) {
                        setInput(s.prompt);
                        inputRef.current?.focus();
                      } else {
                        void send(s.prompt);
                      }
                    }}
                    className="flex items-center justify-between rounded-xl border border-line bg-panel px-3.5 py-2.5 text-left text-[13px] text-ink-2 transition-colors hover:border-accent/40 hover:bg-hover hover:text-ink"
                  >
                    <span>{s.prompt}</span>
                    <span className="ml-2 shrink-0 text-[11px] text-ink-3">{s.fillOnly ? "填入" : "发送"}</span>
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              {messages.map((m) => (
                <div key={m.id} className={clsx("flex flex-col", m.role === "user" ? "items-end" : "items-start")}>
                  {m.role === "user" ? (
                    <div className="selectable max-w-[85%] rounded-2xl rounded-tr-sm bg-accent-soft px-3.5 py-2.5 text-[13px] leading-relaxed whitespace-pre-wrap text-ink">
                      {m.content}
                    </div>
                  ) : (
                    <div className="w-full space-y-2">
                      {m.steps && m.steps.length > 0 && <ToolSteps steps={m.steps} />}
                      <div
                        className={clsx(
                          "rounded-2xl rounded-tl-sm border px-3.5 py-2.5 text-[13px]",
                          m.error
                            ? "border-accent/40 bg-accent-soft/50 text-accent-ink"
                            : "border-line bg-panel text-ink",
                        )}
                      >
                        {m.error ? (
                          <div className="flex items-start gap-2">
                            <AlertCircle className="mt-0.5 size-4 shrink-0" />
                            <span className="whitespace-pre-wrap">{m.content}</span>
                          </div>
                        ) : (
                          <Markdown text={m.content} onIssueClick={openIssue} />
                        )}
                      </div>
                      {m.pendingActions && m.pendingActions.length > 0 && (
                        <PendingActionCard
                          actions={m.pendingActions}
                          state={m.actionState ?? "pending"}
                          results={m.actionResults}
                          onConfirm={() => void confirmActions(m.id, m.pendingActions!)}
                          onCancel={() => cancelActions(m.id)}
                        />
                      )}
                    </div>
                  )}
                </div>
              ))}
              {busy && (
                <div className="flex items-center gap-2 rounded-xl border border-line bg-panel px-3.5 py-2.5 text-[12.5px] text-ink-3">
                  <Spinner className="size-3.5" />
                  <span>AI 正在查询 Redmine 并整理结果…</span>
                </div>
              )}
              <div ref={bottomRef} />
            </div>
          )}
        </div>

        {/* 输入区 */}
        <div className="shrink-0 border-t border-line bg-panel p-3">
          <div className="flex items-end gap-2 rounded-xl border border-line bg-canvas px-3 py-2 focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/15">
            <textarea
              ref={inputRef}
              rows={2}
              value={input}
              disabled={!redmineReady || !aiReady || busy}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="例如：查看 ERP 项目的逾期问题 / 关闭 #1234 并备注已修复…"
              className="max-h-36 min-h-[42px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed text-ink placeholder:text-ink-3 focus:outline-none disabled:opacity-50"
            />
            <Button
              variant="ai"
              size="sm"
              loading={busy}
              disabled={!redmineReady || !aiReady || !input.trim()}
              icon={<SendHorizontal className="size-3.5" />}
              onClick={() => void send()}
            >
              发送
            </Button>
          </div>
          <div className="mt-1.5 flex items-center justify-between px-0.5 text-[11px] text-ink-3">
            <span>Enter 发送 · Shift+Enter 换行 · 点击 #任务号 可直接打开</span>
            <span>{modKey}J 随时呼出</span>
          </div>
        </div>
      </aside>
    </div>
  );
}

/** 折叠展示本轮 AI 调用的 Redmine 工具步骤 */
function ToolSteps({ steps }: { steps: ChatToolStep[] }) {
  const [open, setOpen] = useState(false);
  const failed = steps.filter((s) => !s.ok).length;
  return (
    <div className="rounded-xl border border-line bg-sunken/60 text-[12px]">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-ink-2 hover:text-ink"
      >
        <Wrench className="size-3 shrink-0 text-ink-3" />
        <span className="font-medium">
          调用了 {steps.length} 次 Redmine 操作
          {failed > 0 && <span className="ml-1 text-accent">（{failed} 次失败）</span>}
        </span>
        <span className="ml-auto flex items-center gap-1 truncate text-[11.5px] text-ink-3">
          {!open && steps.map((s) => s.label).join(" → ")}
          <ChevronDown className={clsx("size-3.5 shrink-0 transition-transform", open && "rotate-180")} />
        </span>
      </button>
      {open && (
        <ul className="divide-y divide-line border-t border-line px-3 py-1">
          {steps.map((s, idx) => (
            <li key={idx} className="flex items-start gap-2 py-1.5">
              <span className={clsx("mt-1 size-1.5 shrink-0 rounded-full", s.ok ? "bg-ok" : "bg-accent")} />
              <span className="shrink-0 font-medium text-ink">{s.label}</span>
              {s.write && <Badge tone="warn">写</Badge>}
              <span className="min-w-0 flex-1 break-all text-ink-3">{s.summary}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 写操作待确认卡片 */
function PendingActionCard({
  actions,
  state,
  results,
  onConfirm,
  onCancel,
}: {
  actions: PendingAction[];
  state: "pending" | "executing" | "done" | "cancelled";
  results?: ActionExecutionResult[];
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className={clsx(
        "rounded-xl border p-3.5",
        state === "done"
          ? "border-ok/40 bg-ok-soft/30"
          : state === "cancelled"
            ? "border-line bg-sunken/50 opacity-75"
            : "border-warn/50 bg-warn-soft/35",
      )}
    >
      <div className="flex items-center gap-2">
        <ShieldCheck
          className={clsx(
            "size-4 shrink-0",
            state === "done" ? "text-ok" : state === "cancelled" ? "text-ink-3" : "text-warn",
          )}
        />
        <span className="text-[13px] font-semibold text-ink">
          {state === "done"
            ? "已执行 Redmine 写操作"
            : state === "cancelled"
              ? "已取消写操作"
              : `待确认的 Redmine 操作（${actions.length} 项）`}
        </span>
        <div className="flex-1" />
        {state === "done" && <Badge tone="ok">已完成</Badge>}
        {state === "cancelled" && <Badge tone="muted">已取消</Badge>}
      </div>

      <div className="mt-2.5 space-y-2">
        {actions.map((a) => {
          const res = results?.find((r) => r.id === a.id);
          return (
            <div key={a.id} className="rounded-lg border border-line bg-panel px-3 py-2 text-[12.5px]">
              <div className="flex items-center gap-1.5 font-medium text-ink">
                {res &&
                  (res.ok ? (
                    <CheckCircle2 className="size-3.5 shrink-0 text-ok" />
                  ) : (
                    <AlertCircle className="size-3.5 shrink-0 text-accent" />
                  ))}
                <span>{a.title}</span>
              </div>
              {a.details.length > 0 && (
                <ul className="mt-1 space-y-0.5 text-[12px] text-ink-2">
                  {a.details.map((d, i) => (
                    <li key={i} className="break-words">
                      · {d}
                    </li>
                  ))}
                </ul>
              )}
              {res && (
                <div className={clsx("mt-1.5 text-[12px]", res.ok ? "text-ok" : "text-accent-ink")}>
                  结果：{res.summary}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {(state === "pending" || state === "executing") && (
        <div className="mt-3 flex items-center justify-end gap-2">
          <Button size="sm" variant="ghost" disabled={state === "executing"} onClick={onCancel}>
            取消
          </Button>
          <Button size="sm" variant="primary" loading={state === "executing"} icon={<Check className="size-3.5" />} onClick={onConfirm}>
            确认执行
          </Button>
        </div>
      )}
    </div>
  );
}
