import { useCallback, useEffect, useState } from "react";

/** 每个任务的处理计时，保存在本机（重启后保留），回派时自动换算为工时 */
type Timers = Record<string, { startedAt: number | null; accMs: number }>;
const KEY = "rp.timers";

function load(): Timers {
  try {
    return JSON.parse(localStorage.getItem(KEY) || "{}") as Timers;
  } catch {
    return {};
  }
}

function save(t: Timers) {
  try {
    localStorage.setItem(KEY, JSON.stringify(t));
  } catch {
    // 存储不可用时计时只在本次会话有效
  }
}

export function timerElapsed(id: number): number {
  const t = load()[id];
  if (!t) return 0;
  return t.accMs + (t.startedAt ? Date.now() - t.startedAt : 0);
}

export function clearTimer(id: number) {
  const all = load();
  delete all[id];
  save(all);
}

/** 毫秒 → 工时（小时，保留 1 位，最少 0.1） */
export function msToHours(ms: number): number | null {
  if (ms < 60000) return null;
  return Math.max(0.1, Math.round((ms / 3600000) * 10) / 10);
}

export function useIssueTimer(id: number | null) {
  const [, force] = useState(0);
  const t = id !== null ? load()[id] : undefined;
  const running = !!t?.startedAt;

  useEffect(() => {
    if (!running) return;
    const h = window.setInterval(() => force((x) => x + 1), 1000);
    return () => window.clearInterval(h);
  }, [running, id]);

  const toggle = useCallback(() => {
    if (id === null) return;
    const all = load();
    const cur = all[id] ?? { startedAt: null, accMs: 0 };
    if (cur.startedAt) {
      all[id] = { startedAt: null, accMs: cur.accMs + (Date.now() - cur.startedAt) };
    } else {
      all[id] = { ...cur, startedAt: Date.now() };
    }
    save(all);
    force((x) => x + 1);
  }, [id]);

  const reset = useCallback(() => {
    if (id === null) return;
    clearTimer(id);
    force((x) => x + 1);
  }, [id]);

  return { running, elapsedMs: id !== null ? timerElapsed(id) : 0, toggle, reset };
}
