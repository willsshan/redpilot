import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CheckCircle2, AlertTriangle, XCircle, Info, Undo2, X } from "lucide-react";
import clsx from "clsx";

type Tone = "success" | "error" | "warn" | "info";

interface ToastAction {
  label: string;
  onClick: () => void;
}

interface ToastItem {
  id: number;
  tone: Tone;
  title: string;
  body?: string;
  action?: ToastAction;
  /** 撤销倒计时（毫秒） */
  countdown?: number;
  createdAt: number;
}

interface ToastApi {
  show: (t: Omit<ToastItem, "id" | "createdAt">, durationMs?: number) => number;
  dismiss: (id: number) => void;
  /** 延迟执行写操作，期间可撤销 */
  withUndo: (title: string, run: () => Promise<void>, delayMs?: number) => void;
}

const Ctx = createContext<ToastApi | null>(null);

export function useToast(): ToastApi {
  const c = useContext(Ctx);
  if (!c) throw new Error("ToastProvider missing");
  return c;
}

let seq = 1;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    setItems((xs) => xs.filter((x) => x.id !== id));
    const t = timers.current.get(id);
    if (t) window.clearTimeout(t);
    timers.current.delete(id);
  }, []);

  const show = useCallback(
    (t: Omit<ToastItem, "id" | "createdAt">, durationMs?: number) => {
      const id = seq++;
      setItems((xs) => [...xs.slice(-4), { ...t, id, createdAt: Date.now() }]);
      const d = durationMs ?? (t.tone === "error" ? 8000 : 4000);
      if (d > 0) timers.current.set(id, window.setTimeout(() => dismiss(id), d));
      return id;
    },
    [dismiss],
  );

  const withUndo = useCallback(
    (title: string, run: () => Promise<void>, delayMs = 5000) => {
      let cancelled = false;
      const id = show(
        {
          tone: "info",
          title,
          body: "即将提交到 Redmine",
          countdown: delayMs,
          action: {
            label: "撤销",
            onClick: () => {
              cancelled = true;
              dismiss(id);
              show({ tone: "info", title: "已撤销，未做任何修改" });
            },
          },
        },
        0,
      );
      window.setTimeout(() => {
        if (cancelled) return;
        dismiss(id);
        void run();
      }, delayMs);
    },
    [show, dismiss],
  );

  const value = useMemo(() => ({ show, dismiss, withUndo }), [show, dismiss, withUndo]);

  return (
    <Ctx.Provider value={value}>
      {children}
      <div className="pointer-events-none fixed right-4 bottom-4 z-[100] flex w-[360px] flex-col gap-2">
        {items.map((t) => (
          <ToastCard key={t.id} item={t} onClose={() => dismiss(t.id)} />
        ))}
      </div>
    </Ctx.Provider>
  );
}

const ICONS = { success: CheckCircle2, error: XCircle, warn: AlertTriangle, info: Info };
const COLORS = { success: "text-ok", error: "text-accent", warn: "text-warn", info: "text-info" };

function ToastCard({ item, onClose }: { item: ToastItem; onClose: () => void }) {
  const Icon = item.countdown ? Undo2 : ICONS[item.tone];
  const [left, setLeft] = useState(item.countdown ?? 0);
  useEffect(() => {
    if (!item.countdown) return;
    const t = window.setInterval(() => setLeft(Math.max(0, item.countdown! - (Date.now() - item.createdAt))), 100);
    return () => window.clearInterval(t);
  }, [item.countdown, item.createdAt]);

  return (
    <div className="animate-in pointer-events-auto relative overflow-hidden rounded-xl border border-line bg-panel shadow-pop">
      <div className="flex items-start gap-3 px-4 py-3">
        <Icon className={clsx("mt-0.5 size-4 shrink-0", COLORS[item.tone])} />
        <div className="min-w-0 flex-1">
          <div className="font-medium text-ink">{item.title}</div>
          {item.body && <div className="selectable mt-0.5 text-[12.5px] whitespace-pre-wrap text-ink-2">{item.body}</div>}
        </div>
        {item.action && (
          <button
            onClick={item.action.onClick}
            className="shrink-0 rounded-md bg-sunken px-2.5 py-1 text-xs font-medium text-ink hover:bg-hover"
          >
            {item.action.label}
            {item.countdown ? ` ${Math.ceil(left / 1000)}s` : ""}
          </button>
        )}
        {!item.countdown && (
          <button onClick={onClose} className="shrink-0 rounded p-0.5 text-ink-3 hover:bg-hover hover:text-ink">
            <X className="size-3.5" />
          </button>
        )}
      </div>
      {item.countdown ? (
        <div className="absolute bottom-0 left-0 h-0.5 bg-accent transition-[width] duration-100 ease-linear" style={{ width: `${(left / item.countdown) * 100}%` }} />
      ) : null}
    </div>
  );
}
