import type { TextFormat } from "./api";

export function relativeTime(iso?: string | null): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const diff = (Date.now() - t) / 1000;
  if (diff < 45) return "刚刚";
  if (diff < 3600) return `${Math.round(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.round(diff / 3600)} 小时前`;
  if (diff < 86400 * 7) return `${Math.round(diff / 86400)} 天前`;
  return new Date(iso).toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
}

export function dateTime(iso?: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

export function dueLabel(due?: string | null): { text: string; tone: "danger" | "warn" | "muted" } | null {
  if (!due) return null;
  const d = new Date(`${due}T23:59:59`);
  const days = Math.ceil((d.getTime() - Date.now()) / 86400000);
  if (days < 0) return { text: `逾期 ${-days} 天`, tone: "danger" };
  if (days === 0) return { text: "今天到期", tone: "danger" };
  if (days <= 2) return { text: `${days} 天后到期`, tone: "warn" };
  return { text: `${due.slice(5)} 到期`, tone: "muted" };
}

const CATEGORY_TONES: Record<string, string> = {
  缺陷: "#e5484d",
  需求: "#8e4ec6",
  咨询: "#0b8fd8",
  数据处理: "#d6830b",
  权限配置: "#12a594",
  环境问题: "#e2591b",
  其他: "#8b8b94",
};
const EXTRA = ["#3e63dd", "#d6409f", "#29a383", "#978365", "#7c66dc"];

export function categoryColor(c?: string | null): string {
  if (!c) return "#8b8b94";
  if (CATEGORY_TONES[c]) return CATEGORY_TONES[c];
  let h = 0;
  for (const ch of c) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return EXTRA[h % EXTRA.length];
}

export function priorityTone(name: string): "high" | "normal" | "low" {
  const n = name.toLowerCase();
  if (/(高|紧急|立刻|urgent|high|immediate)/.test(n)) return "high";
  if (/(低|low)/.test(n)) return "low";
  return "normal";
}

export function urgencyColor(u?: string | null): string {
  if (u === "高") return "var(--accent)";
  if (u === "低") return "var(--ink-3)";
  return "var(--warn)";
}

export function initials(name: string): string {
  const n = name.trim();
  if (!n) return "?";
  // 中文名取最后一个字，英文取首字母
  return /[一-龥]/.test(n) ? n.slice(-1) : n[0].toUpperCase();
}

export function avatarColor(name: string): string {
  const palette = ["#e5484d", "#8e4ec6", "#0b8fd8", "#12a594", "#d6830b", "#3e63dd", "#d6409f"];
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return palette[h % palette.length];
}

/** 本地生成处置说明模板 */
export function noteTemplate(sections: string[], format: TextFormat, keyPoints = ""): string {
  const bold = (s: string) => (format === "markdown" ? `**${s}**` : `*${s}*`);
  const lines = ["您好，该问题已处理完成，说明如下：", ""];
  for (const s of sections) {
    lines.push(bold(s));
    lines.push(s === "处理方法" && keyPoints.trim() ? keyPoints.trim() : "");
    lines.push("");
  }
  return lines.join("\n").trim() + "\n";
}

export function formatHours(ms: number): string {
  const totalMin = Math.floor(ms / 60000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  const s = Math.floor((ms % 60000) / 1000);
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}` : `${m}:${String(s).padStart(2, "0")}`;
}

export const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
export const modKey = isMac ? "⌘" : "Ctrl";
