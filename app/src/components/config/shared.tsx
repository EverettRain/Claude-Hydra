import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { api, errorText, type ClaudeFolder, type CopyResult } from "@/lib/api";

export const leaf = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() || path;

export function formatTime(ms: number | null) {
  if (ms == null) return "时间未知";
  return new Date(ms).toLocaleString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" });
}

export function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export const kindLabel: Record<ClaudeFolder["kind"], string> = { local: "本机", desktop: "桌面版", container: "容器" };

/** Folders that are really somewhere else than `from`: a desktop account sharing the local folder is that same folder. */
export function otherFolders(folders: ClaudeFolder[], from: ClaudeFolder) {
  const seen = new Set([from.path]);
  return folders.filter((f) => !seen.has(f.path) && seen.add(f.path));
}

/** Location picker grouped by kind; `folders` decides what can be chosen. */
export function FolderSelect({ folders, value, onChange, label, disabled, placeholder }: {
  folders: ClaudeFolder[]; value: string; onChange: (id: string) => void; label: string; disabled?: boolean; placeholder?: string;
}) {
  const groups = (["local", "desktop", "container"] as const).map((k) => [k, folders.filter((f) => f.kind === k)] as const).filter(([, l]) => l.length);
  return (
    <select aria-label={label} className="field" value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
      {placeholder && <option value="" disabled>{placeholder}</option>}
      {groups.map(([kind, list]) => (
        <optgroup key={kind} label={kindLabel[kind]}>
          {list.map((f) => <option key={f.id} value={f.id}>{f.name}{f.sharedWithLocal ? "（共用本机目录）" : ""}</option>)}
        </optgroup>
      ))}
    </select>
  );
}

const statusText: Record<CopyResult["status"], string> = { copied: "已复制", exists: "已有同名，未改动", failed: "失败" };

/** Copies one item to other locations: pick the targets, choose whether to replace, see each result. */
export function CopyDialog({ open, title, folders, onClose, copy }: {
  open: boolean; title: string; folders: ClaudeFolder[]; onClose: () => void;
  copy: (targets: string[], overwrite: boolean) => Promise<CopyResult[]>;
}) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [overwrite, setOverwrite] = useState(false);
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<CopyResult[] | null>(null);
  const [error, setError] = useState("");
  useEffect(() => { if (open) { setPicked(new Set()); setOverwrite(false); setResults(null); setError(""); } }, [open]);
  const toggle = (id: string) => setPicked((old) => { const next = new Set(old); if (!next.delete(id)) next.add(id); return next; });
  const submit = async () => {
    setBusy(true); setError("");
    try { setResults(await copy([...picked], overwrite)); } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  };
  return (
    <Dialog open={open} onClose={() => { if (!busy) onClose(); }} title={title}
      description={results ? undefined : "选择要复制到的位置。目标已有同名内容时，默认保留目标的版本。"}
      footer={results ? <Button onClick={onClose}>完成</Button> : <>
        <Button variant="ghost" disabled={busy} onClick={onClose}>取消</Button>
        <Button disabled={busy || !picked.size} onClick={submit}>{busy ? "复制中…" : `复制到 ${picked.size} 个位置`}</Button>
      </>}>
      {results ? (
        <ul className="space-y-1.5 text-[13px]">
          {results.map((r) => <li key={r.target} className="flex justify-between gap-3"><span className="truncate">{r.target}</span>
            <span className={r.status === "failed" ? "text-danger" : "text-muted"} title={r.error ?? undefined}>{statusText[r.status]}{r.error ? `：${r.error}` : ""}</span></li>)}
        </ul>
      ) : (
        <div className="space-y-3">
          {folders.length ? <div className="list-card">
            {folders.map((f) => (
              <label key={f.id} className="flex cursor-pointer items-center gap-2.5 px-3 py-2 text-[13px]">
                <input type="checkbox" checked={picked.has(f.id)} disabled={busy} onChange={() => toggle(f.id)} />
                <span className="text-muted">{kindLabel[f.kind]}</span><span className="truncate">{f.name}</span>
              </label>
            ))}
          </div> : <p className="text-sm text-muted">没有其他位置可以复制到。</p>}
          <label className="flex items-center justify-between gap-3 text-[13px]">
            <span>替换目标中的同名内容<span className="block text-[12px] text-muted">被替换的内容会移到目标位置的回收站</span></span>
            <Switch checked={overwrite} onCheckedChange={setOverwrite} disabled={busy} label="替换同名内容" />
          </label>
          {error && <p role="alert" className="text-sm text-danger">{error}</p>}
        </div>
      )}
    </Dialog>
  );
}

/** Edits a text file of a data folder; refuses to save over changes made elsewhere since it was loaded. */
export function TextEditor({ folder, path, rows = 14, validate, onSaved }: {
  folder: string; path: string; rows?: number; validate?: (text: string) => string | null; onSaved?: () => void;
}) {
  // `text` is kept with LF line ends, as a text box reports them; `crlf` restores Windows line ends on save.
  const [loaded, setLoaded] = useState<{ text: string; stamp: string; exists: boolean; crlf: boolean } | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const load = () => {
    setError(""); setNote("");
    api.readClaudeFile(folder, path).then((f) => {
      const text = f.text.replace(/\r\n/g, "\n");
      setLoaded({ ...f, text, crlf: text !== f.text }); setText(text);
    }).catch((e) => setError(errorText(e)));
  };
  useEffect(load, [folder, path]);
  const dirty = !!loaded && text !== loaded.text;
  const save = async () => {
    const invalid = validate?.(text);
    if (invalid) { setError(invalid); return; }
    setBusy(true); setError("");
    try {
      const crlf = !!loaded?.crlf;
      const stamp = await api.saveClaudeFile(folder, path, crlf ? text.replace(/\n/g, "\r\n") : text, loaded?.stamp ?? "");
      setLoaded({ text, stamp, exists: true, crlf }); setNote("已保存"); onSaved?.();
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  };
  if (!loaded) return <p className="px-1 py-4 text-sm text-muted">{error || "正在读取…"}</p>;
  return (
    <div className="space-y-2">
      <textarea aria-label={path} className="field h-auto min-h-[12rem] resize-y py-2 font-mono text-[12.5px] leading-5" rows={rows} spellCheck={false}
        value={text} placeholder={loaded.exists ? "" : "文件还不存在，保存后创建"} onChange={(e) => { setText(e.target.value); setNote(""); }} />
      <div className="flex items-center gap-2">
        {error ? <p role="alert" className="min-w-0 flex-1 text-[12.5px] text-danger">{error}</p>
          : <p className="min-w-0 flex-1 text-[12.5px] text-muted">{note || (dirty ? "有未保存的修改" : "")}</p>}
        <Button size="sm" variant="ghost" disabled={busy} onClick={load}>{dirty ? "放弃修改" : "重新载入"}</Button>
        <Button size="sm" disabled={busy || !dirty} onClick={save}>{busy ? "保存中…" : "保存"}</Button>
      </div>
    </div>
  );
}
