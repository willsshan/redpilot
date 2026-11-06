import { useEffect, useState } from "react";
import { api, errorText, type DispatchQueue, type SplitPart } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { Modal, Spinner } from "./ui";
import { DispatchPanel, outcomeToast } from "./DispatchPanel";
import { SplitDialog } from "./SplitDialog";

/** 转派单个任务（收件箱、项目总览中使用）：与分派台相同的派单栏，可以同时派给前端和后端 */
export function DispatchDialog({ issueId, onClose, onDone }: { issueId: number | null; onClose: () => void; onDone: () => void }) {
  const { config } = useApp();
  const toast = useToast();
  const [q, setQ] = useState<DispatchQueue | null>(null);
  const [splitParts, setSplitParts] = useState<SplitPart[] | null>(null);
  const [custom, setCustom] = useState<SplitPart[] | null>(null);
  const open = issueId !== null;

  useEffect(() => {
    if (issueId === null) return;
    setQ(null);
    setCustom(null);
    api
      .dispatchPreview(issueId)
      .then(setQ)
      .catch((e) => {
        toast.show({ tone: "error", title: "读取成员失败", body: errorText(e) });
        onClose();
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issueId]);

  const item = q?.items[0];
  const project = q?.projects[0];

  return (
    <>
      <Modal
        open={open}
        onClose={() => splitParts === null && onClose()}
        width={900}
        title={item ? `转派 #${item.issue.id}` : "转派"}
        subtitle={item ? `${item.issue.subject} · ${item.issue.projectName} · 当前负责人：${item.issue.assignedToName || "未指派"}` : undefined}
      >
        {item ? (
          <DispatchPanel
            item={item}
            project={project}
            config={config}
            meId={q?.meId}
            active={open && splitParts === null}
            customParts={custom}
            onClearCustom={() => setCustom(null)}
            onEditSplit={(parts) => setSplitParts(parts)}
            onSubmit={(req, title) => {
              onClose();
              toast.withUndo(`#${req.issueId} ${title}`, async () => {
                try {
                  const t = outcomeToast(await api.dispatchIssue(req));
                  toast.show({ tone: t.warn ? "warn" : "success", title: t.title, body: t.body });
                } catch (e) {
                  toast.show({ tone: "error", title: `#${req.issueId} 转派失败`, body: errorText(e) });
                }
                onDone();
              });
            }}
          />
        ) : (
          <div className="flex h-40 items-center justify-center gap-2 text-[12.5px] text-ink-3">
            <Spinner /> 正在读取项目成员、分工和负载…
          </div>
        )}
      </Modal>
      <SplitDialog
        issueId={splitParts !== null ? issueId : null}
        initialParts={splitParts ?? undefined}
        onClose={() => setSplitParts(null)}
        onDone={() => {}}
        onSubmitParts={setCustom}
      />
    </>
  );
}
