import { useEffect, useMemo, useState } from "react";
import clsx from "clsx";
import { ExternalLink, ImageOff, X } from "lucide-react";
import { api, attachmentUrl, errorText, type Attachment } from "../lib/api";
import { formatBytes } from "./Attachments";

const IMAGE_EXT = /\.(png|jpe?g|gif|bmp|webp|svg)(\?.*)?$/i;

export function isImage(a: Pick<Attachment, "filename" | "contentType">): boolean {
  return !!a.contentType?.startsWith("image/") || IMAGE_EXT.test(a.filename);
}

/**
 * Redmine 文本里的图片写法：
 * - Markdown：![说明](截图.png)
 * - Textile：!截图.png!、!>截图.png!、!{width:50%}截图.png!
 * - 宏：{{thumbnail(截图.png)}}
 */
const IMAGE_PATTERN =
  /!\[[^\]\n]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"\n]*")?\s*\)|\{\{thumbnail\(([^,)\n]+)[^)\n]*\)\}\}|!(?:[<>=]|\{[^}\n]*\}|\([^)\n]*\))*([^\s!(){}]+)(?:\([^)\n]*\))?!/g;

type Src = { id: number } | { url: string };

function resolveSrc(raw: string, attachments: Attachment[]): Src | null {
  let s = raw.trim();
  try {
    s = decodeURIComponent(s);
  } catch {
    // 保留原样
  }
  const m = /\/attachments\/(?:download\/|thumbnail\/)?(\d+)/.exec(s);
  if (m) return { id: Number(m[1]) };
  if (/^data:image\//i.test(s)) return { url: s };
  if (/^(https?:)?\/\//i.test(s)) return IMAGE_EXT.test(s) ? { url: s } : null;
  // 按文件名匹配本任务的附件；同名时 Redmine 使用最新上传的那个
  const name = (s.split(/[\\/]/).pop() ?? "").toLowerCase();
  const att = [...attachments].reverse().find((a) => a.filename.toLowerCase() === name);
  return att ? { id: att.id } : null;
}

type Segment = { text: string } | { src: Src; alt: string };

function parse(text: string, attachments: Attachment[]): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (const m of text.matchAll(IMAGE_PATTERN)) {
    const raw = m[1] ?? m[2] ?? m[3] ?? "";
    const src = resolveSrc(raw, attachments);
    // 解析不出图片的（比如普通的感叹号），按原文显示
    if (!src) continue;
    const idx = m.index ?? 0;
    if (idx > last) out.push({ text: text.slice(last, idx) });
    out.push({ src, alt: raw.split(/[\\/]/).pop() ?? raw });
    last = idx + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

/** 显示 Redmine 描述/备注：保留原文换行，图片在原位置显示 */
export function RichText({ text, attachments, className }: { text: string; attachments: Attachment[]; className?: string }) {
  const segments = useMemo(() => parse(text, attachments), [text, attachments]);
  return (
    <div className={clsx("selectable whitespace-pre-wrap", className)}>
      {segments.map((seg, i) =>
        "text" in seg ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <span key={i} className="my-2 block">
            <Picture src={seg.src} alt={seg.alt} attachments={attachments} className="max-h-[420px] max-w-full" />
          </span>
        ),
      )}
    </div>
  );
}

/** 附件区的缩略图 */
export function Thumbnail({ att, attachments }: { att: Attachment; attachments: Attachment[] }) {
  return (
    <figure className="w-[168px] overflow-hidden rounded-lg border border-line bg-panel">
      <Picture src={{ id: att.id }} alt={att.filename} attachments={attachments} className="h-[112px] w-full object-cover" bare />
      <figcaption className="flex items-center gap-1 border-t border-line px-2 py-1 text-[11.5px] text-ink-3">
        <span className="min-w-0 flex-1 truncate" title={att.filename}>
          {att.filename}
        </span>
        <span className="shrink-0">{formatBytes(att.filesize)}</span>
      </figcaption>
    </figure>
  );
}

function Picture({ src, alt, attachments, className, bare }: { src: Src; alt: string; attachments: Attachment[]; className?: string; bare?: boolean }) {
  const [url, setUrl] = useState<string | null>("url" in src ? src.url : null);
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(false);
  const id = "id" in src ? src.id : null;
  const att = id !== null ? attachments.find((a) => a.id === id) : undefined;

  useEffect(() => {
    if (id === null) return;
    let alive = true;
    setError(null);
    attachmentUrl(id, att?.contentType)
      .then((u) => alive && setUrl(u))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [id, att?.contentType]);

  if (error) {
    return (
      <span className={clsx("inline-flex items-center gap-1.5 rounded-lg border border-dashed border-line px-2.5 py-1.5 text-[12px] text-ink-3", bare && "h-[112px] w-full justify-center")} title={error}>
        <ImageOff className="size-3.5 shrink-0" />
        <span className="truncate">{alt}：{error}</span>
      </span>
    );
  }
  if (!url) return <span className={clsx("skeleton block", bare ? "h-[112px] w-full" : "h-40 w-64")} />;

  const external = att?.contentUrl ?? ("url" in src ? src.url : null);
  return (
    <>
      <img
        src={url}
        alt={alt}
        loading="lazy"
        onClick={() => setZoom(true)}
        onError={() => setError("图片无法显示")}
        className={clsx("cursor-zoom-in", !bare && "rounded-lg border border-line", className)}
      />
      {zoom && (
        <div className="fixed inset-0 z-[95] flex flex-col bg-black/80 backdrop-blur-sm" onClick={() => setZoom(false)}>
          <div className="flex shrink-0 items-center gap-2 px-4 py-3 text-[13px] text-white/80" onClick={(e) => e.stopPropagation()}>
            <span className="min-w-0 flex-1 truncate">{alt}</span>
            {external && /^https?:/.test(external) && (
              <button className="flex items-center gap-1 rounded-md px-2 py-1 hover:bg-white/10" onClick={() => void api.openUrl(external)}>
                <ExternalLink className="size-3.5" />
                在浏览器中打开
              </button>
            )}
            <button className="rounded-md p-1 hover:bg-white/10" title="关闭 (Esc)" onClick={() => setZoom(false)}>
              <X className="size-4" />
            </button>
          </div>
          <div className="flex min-h-0 flex-1 items-center justify-center p-6 pt-0">
            <img src={url} alt={alt} className="max-h-full max-w-full rounded-lg object-contain shadow-pop" />
          </div>
          <EscToClose onClose={() => setZoom(false)} />
        </div>
      )}
    </>
  );
}

function EscToClose({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [onClose]);
  return null;
}
