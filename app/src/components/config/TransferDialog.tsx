import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { api, errorText, type ClaudeFolder, type TransferOutcome, type TransferPlan } from "@/lib/api";
import { FolderSelect, otherFolders } from "./shared";

export type Transfer =
  | { type: "copy" | "move"; ids: string[] }
  /** Sessions from an export file, into the current location. */
  | { type: "import"; path: string };

const outcomeText = (o: TransferOutcome) =>
  [`完成 ${o.copied} 个`, o.skipped && `目标已有 ${o.skipped} 个（跳过）`, o.failed && `失败 ${o.failed} 个`].filter(Boolean).join("，") + "。";

/**
 * Copies, moves or imports sessions. Claude Code lists a session only under the working folder
 * it ran in, so every working folder of the selection gets a target folder on the other side:
 * filled in when Hydra can translate it through the shared folders, chosen by the user otherwise.
 */
export function TransferDialog({ folder, folders, transfer, onClose, onDone }: {
  folder: ClaudeFolder; folders: ClaudeFolder[]; transfer: Transfer | null; onClose: () => void; onDone: (message: string) => void;
}) {
  const [target, setTarget] = useState("");
  const [plan, setPlan] = useState<TransferPlan | null>(null);
  const [source, setSource] = useState("");
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [outcome, setOutcome] = useState<TransferOutcome | null>(null);
  const importing = transfer?.type === "import";
  const destination = importing ? folder.id : target;

  useEffect(() => { setTarget(""); setPlan(null); setMapping({}); setError(""); setOutcome(null); setSource(""); }, [transfer]);
  useEffect(() => {
    if (!transfer || !destination) return;
    let current = true;
    setPlan(null); setError("");
    const request = transfer.type === "import"
      ? api.previewCodeSessionImport(transfer.path, destination).then((p) => { if (current) setSource(`来自「${p.source}」的 ${p.sessions.length} 个会话`); return p.plan; })
      : api.planCodeSessions(folder.id, transfer.ids, destination);
    request.then((p) => {
      if (!current) return;
      setPlan(p);
      setMapping(Object.fromEntries(p.roots.map((r) => [r.cwd, r.target ?? ""])));
    }).catch((e) => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [transfer, destination, folder.id]);

  const ready = !!plan && plan.roots.every((r) => mapping[r.cwd]?.trim());
  const submit = async () => {
    if (!transfer || !plan) return;
    setBusy(true); setError("");
    const map = Object.fromEntries(Object.entries(mapping).map(([k, v]) => [k, v.trim()]));
    try {
      const o = transfer.type === "import"
        ? await api.importCodeSessions(transfer.path, destination, map)
        : await api.transferCodeSessions(folder.id, transfer.ids, destination, map, transfer.type === "move");
      onDone(outcomeText(o));
      // Problems stay on screen to be read; a clean run just closes.
      if (o.errors.length) setOutcome(o); else onClose();
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  };
  const choose = async (cwd: string) => {
    try {
      const picked = await pickFolder({ directory: true, title: `选择「${cwd}」的会话放到哪个工作目录` });
      if (typeof picked === "string") setMapping((m) => ({ ...m, [cwd]: picked }));
    } catch (e) { setError(errorText(e)); }
  };

  const count = transfer && transfer.type !== "import" ? transfer.ids.length : 0;
  const title = transfer?.type === "import" ? "导入会话" : transfer?.type === "move" ? `移动 ${count} 个会话` : `复制 ${count} 个会话`;
  return (
    <Dialog open={!!transfer} onClose={() => { if (!busy) onClose(); }} title={title} className="max-w-lg"
      description={outcome ? undefined : importing ? `导入到「${folder.name}」。目标里已有的会话会跳过，不会覆盖。`
        : transfer?.type === "move" ? "复制到目标后，原会话移到这个位置的回收站。目标里已有的会话会跳过。" : "原会话保留不动。目标里已有的会话会跳过，不会覆盖。"}
      footer={outcome ? <Button onClick={onClose}>完成</Button> : <>
        <Button variant="ghost" disabled={busy} onClick={onClose}>取消</Button>
        <Button disabled={busy || !ready} onClick={submit}>{busy ? "处理中…" : importing ? "导入" : transfer?.type === "move" ? "移动" : "复制"}</Button>
      </>}>
      {outcome ? (
        <div className="space-y-2 text-[13px]">
          <p>{outcomeText(outcome)}</p>
          <ul className="list-disc space-y-1 pl-5 text-[12.5px] text-danger">{outcome.errors.map((e) => <li key={e}>{e}</li>)}</ul>
        </div>
      ) : (
        <div className="space-y-3">
          {importing ? source && <p className="text-[13px]">{source}</p>
            : <FolderSelect label="目标位置" folders={otherFolders(folders, folder)} value={target} onChange={setTarget} disabled={busy} placeholder="选择目标位置" />}
          {destination && !plan && !error && <p className="text-sm text-muted">正在核对工作目录…</p>}
          {plan && !plan.roots.length && <p className="text-sm text-muted">所选会话都没有记录工作目录，无法迁移。</p>}
          {plan && plan.roots.length > 0 && (
            <div className="space-y-2">
              <p className="text-[12.5px] text-muted">
                Claude Code 按工作目录列出会话（<code>claude --resume</code>），所以每个工作目录都要对应到目标那一侧的目录。
                {plan.hostTarget ? "目标是这台电脑上的文件夹。" : "目标是容器：共享的文件夹在 /workspace 下。"}
              </p>
              <datalist id="transfer-choices">{plan.choices.map((c) => <option key={c} value={c} />)}</datalist>
              <div className="list-card">
                {plan.roots.map((r) => (
                  <div key={r.cwd} className="space-y-1.5 px-3 py-2.5">
                    <div className="flex justify-between gap-2 text-[12.5px]">
                      <span className="selectable min-w-0 truncate font-mono" title={r.cwd}>{r.cwd}</span>
                      <span className="shrink-0 text-muted">{r.sessions} 个会话</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="shrink-0 text-[12px] text-muted">→</span>
                      <input className="field font-mono text-[12.5px]" aria-label={`${r.cwd} 的目标工作目录`} list={plan.hostTarget ? undefined : "transfer-choices"}
                        placeholder={plan.hostTarget ? "选择这台电脑上的文件夹" : "选择或输入 /workspace 下的目录"} disabled={busy}
                        value={mapping[r.cwd] ?? ""} onChange={(e) => setMapping((m) => ({ ...m, [r.cwd]: e.target.value }))} />
                      {plan.hostTarget && <Button size="sm" variant="outline" disabled={busy} onClick={() => choose(r.cwd)}>选择…</Button>}
                    </div>
                    {!r.target && <p className="text-[12px] text-warn">目标那一侧看不到这个目录，请指定放到哪里。</p>}
                  </div>
                ))}
              </div>
            </div>
          )}
          {error && <p role="alert" className="text-sm text-danger">{error}</p>}
        </div>
      )}
    </Dialog>
  );
}
