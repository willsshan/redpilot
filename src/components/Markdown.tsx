import type { ReactNode } from "react";

/** 轻量 Markdown 渲染：标题、列表、加粗、行内代码、代码块。只生成 React 元素，不注入 HTML。 */
export function Markdown({
  text,
  className,
  onIssueClick,
}: {
  text: string;
  className?: string;
  onIssueClick?: (id: number) => void;
}) {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim().startsWith("```")) {
      const buf: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith("```")) buf.push(lines[i++]);
      i++;
      out.push(<pre key={key++}>{buf.join("\n")}</pre>);
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) {
      const level = Math.min(h[1].length, 3);
      const content = inline(h[2], onIssueClick);
      out.push(level === 1 ? <h1 key={key++}>{content}</h1> : level === 2 ? <h2 key={key++}>{content}</h2> : <h3 key={key++}>{content}</h3>);
      i++;
      continue;
    }
    if (/^\s*([-*+]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const items: ReactNode[] = [];
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) {
        const raw = lines[i].replace(/^\s*([-*+]|\d+[.)])\s+/, "");
        const indent = /^\s{2,}/.test(lines[i]);
        items.push(
          <li key={i} style={indent ? { marginLeft: "1.2em" } : undefined}>
            {inline(raw, onIssueClick)}
          </li>,
        );
        i++;
      }
      out.push(ordered ? <ol key={key++}>{items}</ol> : <ul key={key++}>{items}</ul>);
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,4}\s|\s*([-*+]|\d+[.)])\s+|```)/.test(lines[i])) para.push(lines[i++]);
    out.push(
      <p key={key++}>
        {para.map((p, idx) => (
          <span key={idx}>
            {inline(p, onIssueClick)}
            {idx < para.length - 1 && <br />}
          </span>
        ))}
      </p>,
    );
  }
  return <div className={`prose-rp selectable ${className ?? ""}`}>{out}</div>;
}

function inline(s: string, onIssueClick?: (id: number) => void): ReactNode[] {
  const parts: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|`[^`]+`|#\d+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(s))) {
    if (m.index > last) parts.push(s.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("**")) parts.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    else if (t.startsWith("`")) parts.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else if (onIssueClick) {
      const id = Number(t.slice(1));
      parts.push(
        <button
          key={k++}
          type="button"
          onClick={() => onIssueClick(id)}
          title={`在收件箱查看 ${t}`}
          className="inline rounded px-0.5 font-mono text-[0.95em] text-accent-ink underline decoration-accent/40 underline-offset-2 hover:bg-accent-soft/70"
        >
          {t}
        </button>,
      );
    } else {
      parts.push(
        <span key={k++} className="font-mono text-[0.95em] text-accent-ink">
          {t}
        </span>,
      );
    }
    last = m.index + t.length;
  }
  if (last < s.length) parts.push(s.slice(last));
  return parts;
}
