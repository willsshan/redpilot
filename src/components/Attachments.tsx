import { useCallback, useRef, useState, type ClipboardEvent, type DragEvent } from "react";
import clsx from "clsx";
import { FileText, ImageIcon, Paperclip, X } from "lucide-react";
import { api, errorText, type StagedFile, type TextFormat } from "../lib/api";
import { useToast } from "../lib/toast";
import { Spinner } from "./ui";

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** 截图文件名：截图-20261102-153012.png */
function screenshotName(type: string, index: number): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  const stamp = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
  const ext = type.split("/")[1]?.replace("jpeg", "jpg") || "png";
  return `截图-${stamp}${index ? `-${index + 1}` : ""}.${ext}`;
}

/** 处置说明里引用附件图片的语法（Redmine 按文件名匹配本任务的附件） */
export function imageRef(filename: string, format: TextFormat): string {
  return format === "markdown" ? `![](${filename})` : `!${filename}!`;
}

/** 暂存待上传的附件。文件会先复制到 RedPilot 数据目录，提交回派时再上传到每个任务。 */
export function useStagedFiles() {
  const toast = useToast();
  const [files, setFiles] = useState<StagedFile[]>([]);
  const [busy, setBusy] = useState(0);

  const add = useCallback(
    async (list: File[], { screenshot = false } = {}): Promise<StagedFile[]> => {
      if (list.length === 0) return [];
      setBusy((b) => b + list.length);
      const out: StagedFile[] = [];
      for (const [i, f] of list.entries()) {
        const name = screenshot || !f.name ? screenshotName(f.type, i) : f.name;
        try {
          out.push(await api.stageAttachment(f, name));
        } catch (e) {
          toast.show({ tone: "error", title: `无法添加附件 ${name}`, body: errorText(e) });
        } finally {
          setBusy((b) => b - 1);
        }
      }
      setFiles((xs) => [...xs, ...out]);
      return out;
    },
    [toast],
  );

  const remove = useCallback((path: string) => setFiles((xs) => xs.filter((x) => x.path !== path)), []);
  const reset = useCallback(() => setFiles([]), []);

  /** 粘贴截图；返回 null 表示剪贴板里没有文件，交给默认粘贴行为 */
  const handlePaste = useCallback(
    (e: ClipboardEvent): Promise<StagedFile[]> | null => {
      const pasted = [...e.clipboardData.items].filter((it) => it.kind === "file").map((it) => it.getAsFile()).filter((f): f is File => !!f);
      if (pasted.length === 0) return null;
      e.preventDefault();
      const allImages = pasted.every((f) => f.type.startsWith("image/"));
      return add(pasted, { screenshot: allImages });
    },
    [add],
  );

  return { files, busy, add, remove, reset, handlePaste };
}

export type StagedFiles = ReturnType<typeof useStagedFiles>;

/** 附件栏：选择文件、拖入文件，显示已添加的附件 */
export function AttachmentBar({ att, hint }: { att: StagedFiles; hint?: string }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <div>
      <div className="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          onClick={() => input.current?.click()}
          className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-dashed border-line-strong px-2.5 text-[12.5px] text-ink-2 hover:border-accent hover:text-accent"
        >
          <Paperclip className="size-3.5" />
          添加附件
        </button>
        {att.files.map((f) => {
          const Icon = f.contentType?.startsWith("image/") ? ImageIcon : FileText;
          return (
            <span key={f.path} className="inline-flex h-7 max-w-[260px] items-center gap-1.5 rounded-lg border border-line bg-panel pr-1 pl-2 text-[12.5px]">
              <Icon className="size-3.5 shrink-0 text-ink-3" />
              <span className="truncate">{f.filename}</span>
              <span className="shrink-0 text-[11.5px] text-ink-3">{formatBytes(f.size)}</span>
              <button type="button" title="移除" onClick={() => att.remove(f.path)} className="rounded p-0.5 text-ink-3 hover:bg-hover hover:text-ink">
                <X className="size-3" />
              </button>
            </span>
          );
        })}
        {att.busy > 0 && <Spinner className="size-3.5" />}
        <span className="text-[11.5px] text-ink-3">{hint ?? "可以直接粘贴截图，或把文件拖到这里"}</span>
      </div>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          void att.add([...(e.target.files ?? [])]);
          e.target.value = "";
        }}
      />
    </div>
  );
}

/** 让一块区域可以拖入文件 */
export function useFileDrop(onFiles: (files: File[]) => void) {
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  const hasFiles = (e: DragEvent) => [...e.dataTransfer.types].includes("Files");
  return {
    over,
    props: {
      onDragEnter: (e: DragEvent) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current += 1;
        setOver(true);
      },
      onDragOver: (e: DragEvent) => {
        if (hasFiles(e)) e.preventDefault();
      },
      onDragLeave: (e: DragEvent) => {
        if (!hasFiles(e)) return;
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) setOver(false);
      },
      onDrop: (e: DragEvent) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        depth.current = 0;
        setOver(false);
        onFiles([...e.dataTransfer.files]);
      },
    },
  };
}

export function DropOverlay({ show }: { show: boolean }) {
  return (
    <div
      className={clsx(
        "pointer-events-none absolute inset-2 z-10 flex items-center justify-center rounded-xl border-2 border-dashed border-accent bg-accent-soft/80 text-[13px] font-medium text-accent-ink transition-opacity",
        show ? "opacity-100" : "opacity-0",
      )}
    >
      松开即可添加为附件
    </div>
  );
}
