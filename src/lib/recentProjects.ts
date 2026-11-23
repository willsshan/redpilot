import { useSyncExternalStore } from "react";

const KEY = "rp.recentProjects";
const MAX_RECENT = 20;
/** 下拉框「最近使用」分组和「最近」标记展示的最大项目数 */
export const RECENT_GROUP_LIMIT = 5;

const listeners = new Set<() => void>();
let cached: number[] | null = null;
let initializedFromStorage = false;

function validId(v: unknown): v is number {
  return typeof v === "number" && Number.isInteger(v) && v > 0;
}

function normalize(list: unknown[]): number[] {
  const seen = new Set<number>();
  const out: number[] = [];
  for (const x of list) {
    if (validId(x) && !seen.has(x)) {
      seen.add(x);
      out.push(x);
      if (out.length >= MAX_RECENT) break;
    }
  }
  return out;
}

/** 首次升级时，从已记住的项目总览 / 分派台项目中继承最近使用记录 */
function seedLegacy(): number[] {
  const seed: number[] = [];
  for (const k of ["rp.project.id", "rp.dispatch.project"]) {
    try {
      const v = Number(localStorage.getItem(k));
      if (validId(v)) seed.push(v);
    } catch {
      // 忽略存储读取失败
    }
  }
  return normalize(seed);
}

function readStorage(): number[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return seedLegacy();
    initializedFromStorage = true;
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? normalize(parsed) : [];
  } catch {
    return [];
  }
}

function saveStorage(ids: number[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(ids));
    initializedFromStorage = true;
  } catch {
    // 存储不可用时只在本次会话生效
  }
}

function emit() {
  for (const fn of listeners) fn();
}

export function getRecentProjects(): number[] {
  if (cached === null) cached = readStorage();
  return cached;
}

/** 记录最近使用的项目（放到最前） */
export function recordRecentProject(id: number | null | undefined) {
  if (!validId(id)) return;
  const prev = getRecentProjects();
  if (prev[0] === id && initializedFromStorage) return;
  const next = [id, ...prev.filter((x) => x !== id)].slice(0, MAX_RECENT);
  cached = next;
  saveStorage(next);
  emit();
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  const onStorage = (e: StorageEvent) => {
    if (e.key === KEY) {
      cached = readStorage();
      emit();
    }
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("storage", onStorage);
  };
}

export function useRecentProjects(): number[] {
  return useSyncExternalStore(subscribe, getRecentProjects);
}

/** 稳定排序：最近使用过的项目排在前面（按最近程度倒序），其余保持原有顺序 */
export function sortByRecent<T>(items: readonly T[], recentIds: readonly number[], getId: (item: T) => number): T[] {
  if (recentIds.length === 0 || items.length <= 1) return [...items];
  const rank = new Map(recentIds.map((id, idx) => [id, idx]));
  return [...items].sort((a, b) => {
    const ra = rank.get(getId(a));
    const rb = rank.get(getId(b));
    if (ra !== undefined && rb !== undefined) return ra - rb;
    if (ra !== undefined) return -1;
    if (rb !== undefined) return 1;
    return 0;
  });
}

/** 拆分为「最近使用」（最多 limit 个，按最近程度倒序）和「其余项目」（保持原有顺序，无重复） */
export function partitionRecent<T>(
  items: readonly T[],
  recentIds: readonly number[],
  getId: (item: T) => number,
  limit = RECENT_GROUP_LIMIT,
): { recent: T[]; rest: T[] } {
  if (recentIds.length === 0 || items.length === 0) return { recent: [], rest: [...items] };
  const byId = new Map<number, T>();
  for (const item of items) byId.set(getId(item), item);
  const recent: T[] = [];
  const used = new Set<number>();
  for (const id of recentIds) {
    const found = byId.get(id);
    if (found !== undefined && !used.has(id)) {
      recent.push(found);
      used.add(id);
      if (recent.length >= limit) break;
    }
  }
  const rest = items.filter((item) => !used.has(getId(item)));
  return { recent, rest };
}
