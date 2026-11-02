import clsx from "clsx";
import { Loader2, X } from "lucide-react";
import { useEffect, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { avatarColor, categoryColor, initials } from "../lib/format";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "ai";

export function Button({
  variant = "secondary",
  size = "md",
  loading,
  icon,
  children,
  className,
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  size?: "sm" | "md" | "lg";
  loading?: boolean;
  icon?: ReactNode;
}) {
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      className={clsx(
        "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap transition-colors disabled:opacity-50",
        size === "sm" && "h-7 px-2.5 text-[12.5px]",
        size === "md" && "h-8 px-3 text-[13px]",
        size === "lg" && "h-10 px-4 text-sm",
        variant === "primary" && "bg-accent text-white shadow-sm hover:bg-accent-hover",
        variant === "secondary" && "border border-line bg-panel text-ink shadow-[0_1px_1px_rgb(0_0_0/0.03)] hover:bg-hover",
        variant === "ghost" && "text-ink-2 hover:bg-hover hover:text-ink",
        variant === "danger" && "border border-line bg-panel text-accent-ink hover:bg-accent-soft",
        variant === "ai" &&
          "bg-gradient-to-r from-[#e5484d] to-[#8e4ec6] text-white shadow-sm hover:brightness-110",
        className,
      )}
    >
      {loading ? <Loader2 className="size-3.5 animate-spin" /> : icon}
      {children}
    </button>
  );
}

export function IconButton({ className, title, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      title={title}
      aria-label={title}
      className={clsx(
        "inline-flex size-7 items-center justify-center rounded-md text-ink-3 transition-colors hover:bg-hover hover:text-ink disabled:opacity-40",
        className,
      )}
    >
      {children}
    </button>
  );
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...rest}
      className={clsx(
        "h-8 w-full rounded-lg border border-line bg-panel px-2.5 text-[13px] text-ink placeholder:text-ink-3 focus:border-accent focus:ring-2 focus:ring-accent/15 focus:outline-none",
        className,
      )}
    />
  );
}

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      {...rest}
      className={clsx(
        "w-full resize-none rounded-lg border border-line bg-panel px-3 py-2 text-[13px] leading-relaxed text-ink placeholder:text-ink-3 focus:border-accent focus:ring-2 focus:ring-accent/15 focus:outline-none",
        className,
      )}
    />
  );
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...rest}
      className={clsx(
        "h-8 w-full rounded-lg border border-line bg-panel px-2 text-[13px] text-ink focus:border-accent focus:outline-none",
        className,
      )}
    >
      {children}
    </select>
  );
}

export function Switch({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={clsx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-50",
        checked ? "bg-accent" : "bg-line-strong",
      )}
    >
      <span className={clsx("inline-block size-4 rounded-full bg-white shadow transition-transform", checked ? "translate-x-[18px]" : "translate-x-0.5")} />
    </button>
  );
}

export function Badge({
  children,
  color,
  tone,
  className,
}: {
  children: ReactNode;
  color?: string;
  tone?: "accent" | "ok" | "warn" | "info" | "muted";
  className?: string;
}) {
  const style = color ? { color, backgroundColor: `color-mix(in srgb, ${color} 13%, transparent)` } : undefined;
  return (
    <span
      style={style}
      className={clsx(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded-md px-1.5 text-[11.5px] leading-none font-medium whitespace-nowrap",
        !color && tone === "accent" && "bg-accent-soft text-accent-ink",
        !color && tone === "ok" && "bg-ok-soft text-ok",
        !color && tone === "warn" && "bg-warn-soft text-warn",
        !color && tone === "info" && "bg-info-soft text-info",
        !color && (!tone || tone === "muted") && "bg-sunken text-ink-2",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function CategoryBadge({ category, source }: { category?: string | null; source?: string | null }) {
  if (!category) return <Badge tone="muted">未分类</Badge>;
  const c = categoryColor(category);
  return (
    <Badge color={c}>
      <span className="size-1.5 rounded-full" style={{ backgroundColor: c }} />
      {category}
      {source === "rule" && <span className="opacity-60">·规则</span>}
    </Badge>
  );
}

export function Avatar({ name, size = 22 }: { name: string; size?: number }) {
  return (
    <span
      title={name}
      className="inline-flex shrink-0 items-center justify-center rounded-full font-medium text-white"
      style={{ width: size, height: size, fontSize: size * 0.45, backgroundColor: avatarColor(name || "?") }}
    >
      {initials(name)}
    </span>
  );
}

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={clsx("rounded-xl border border-line bg-panel shadow-card", className)}>{children}</div>;
}

export function SectionTitle({ children, right, icon }: { children: ReactNode; right?: ReactNode; icon?: ReactNode }) {
  return (
    <div className="mb-2 flex items-center justify-between gap-2">
      <div className="flex items-center gap-1.5 text-[12px] font-semibold tracking-wide text-ink-3 uppercase">
        {icon}
        {children}
      </div>
      {right}
    </div>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border border-line bg-sunken px-1 font-sans text-[10.5px] text-ink-3">
      {children}
    </kbd>
  );
}

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={clsx("size-4 animate-spin text-ink-3", className)} />;
}

export function Empty({ icon, title, desc, action }: { icon: ReactNode; title: string; desc?: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex h-full flex-col items-center justify-center px-8 py-16 text-center">
      <div className="mb-4 flex size-12 items-center justify-center rounded-2xl bg-sunken text-ink-3">{icon}</div>
      <div className="text-[15px] font-semibold text-ink">{title}</div>
      {desc && <div className="mt-1.5 max-w-sm text-[13px] text-ink-3">{desc}</div>}
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  width = 760,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-6 backdrop-blur-[2px]" onMouseDown={onClose}>
      <div
        className="animate-in flex max-h-full w-full flex-col overflow-hidden rounded-2xl border border-line bg-panel shadow-pop"
        style={{ maxWidth: width }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-4 border-b border-line px-5 py-4">
          <div className="min-w-0">
            <div className="text-[15px] font-semibold text-ink">{title}</div>
            {subtitle && <div className="mt-0.5 truncate text-[12.5px] text-ink-3">{subtitle}</div>}
          </div>
          <IconButton title="关闭" onClick={onClose}>
            <X className="size-4" />
          </IconButton>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
        {footer && <div className="flex items-center justify-end gap-2 border-t border-line bg-sunken/50 px-5 py-3">{footer}</div>}
      </div>
    </div>
  );
}

export function Field({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block">
      <div className="mb-1 text-[12.5px] font-medium text-ink-2">{label}</div>
      {children}
      {hint && <div className="mt-1 text-[11.5px] leading-snug text-ink-3">{hint}</div>}
    </label>
  );
}
