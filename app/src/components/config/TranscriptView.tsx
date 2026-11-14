import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { api, errorText, type ClaudeCodeSession, type TranscriptItem } from "@/lib/api";
import { cn } from "@/lib/utils";
import { formatTime } from "./shared";

const PAGE = 200;

function Omitted({ count }: { count: number }) {
  return count ? <p className="mt-1 text-[11.5px] text-muted">（内容过长，另有 {count.toLocaleString()} 个字符未显示）</p> : null;
}

function Entry({ item }: { item: TranscriptItem }) {
  const body = <div className="selectable whitespace-pre-wrap break-words">{item.text}</div>;
  switch (item.kind) {
    case "user":
      return <div className="ml-10 rounded-xl bg-pill px-3 py-2 text-[13px]">{body}<Omitted count={item.omitted} /></div>;
    case "assistant":
      return <div className="text-[13px] leading-6">{body}<Omitted count={item.omitted} /></div>;
    case "command":
      return <p className="selectable whitespace-pre-wrap break-words font-mono text-[12px] text-muted">› {item.text}</p>;
    case "compact":
    case "system":
      return (
        <details className={cn("rounded-lg px-3 py-1.5 text-[12px]", item.error ? "bg-danger-soft text-danger" : "bg-card-2 text-muted")}>
          <summary className="cursor-pointer">{item.kind === "compact" ? "上下文已压缩" : "系统消息"}</summary>
          <div className="mt-1 font-mono text-[11.5px]">{body}<Omitted count={item.omitted} /></div>
        </details>
      );
    default: {
      const label = item.kind === "thinking" ? "思考" : item.kind === "tool" ? `调用 ${item.name ?? "工具"}` : item.error ? "工具出错" : "工具结果";
      return (
        <details className={cn("rounded-lg px-3 py-1.5 text-[12px]", item.error ? "bg-danger-soft text-danger" : "bg-card-2 text-muted")}>
          <summary className="cursor-pointer truncate">{label}{item.kind !== "thinking" && item.text ? ` · ${item.text.slice(0, 80).replace(/\s+/g, " ")}` : ""}</summary>
          <div className={cn("mt-1 text-fg-2", item.kind === "thinking" ? "text-[12.5px]" : "font-mono text-[11.5px]")}>{body}<Omitted count={item.omitted} /></div>
        </details>
      );
    }
  }
}

/** Reads a session page by page: prompts and answers in full, thinking and tools folded. */
export function TranscriptView({ folder, session, onClose }: { folder: string; session: ClaudeCodeSession | null; onClose: () => void }) {
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const load = (offset: number) => {
    if (!session) return;
    setLoading(true); setError("");
    api.readCodeSession(folder, session.id, offset, PAGE)
      .then((t) => { setItems((old) => (offset ? [...old, ...t.items] : t.items)); setTotal(t.total); })
      .catch((e) => setError(errorText(e))).finally(() => setLoading(false));
  };
  useEffect(() => { setItems([]); setTotal(0); load(0); }, [folder, session?.id]);
  return (
    <Dialog open={!!session} onClose={onClose} title={session?.title ?? ""} className="h-full max-w-3xl"
      description={session && <span className="selectable">{session.cwd ?? "未记录工作目录"} · {formatTime(session.lastActivityAt)} · {session.model ?? "模型未知"}</span>}>
      <div className="space-y-3 pb-2">
        {items.map((it, i) => <Entry key={i} item={it} />)}
        {error && <p role="alert" className="text-sm text-danger">{error}</p>}
        {loading && <p className="text-sm text-muted">正在读取…</p>}
        {!loading && !error && !items.length && <p className="text-sm text-muted">这个会话没有可显示的内容。</p>}
        {!loading && items.length < total && (
          <div className="flex justify-center"><Button size="sm" variant="outline" onClick={() => load(items.length)}>继续加载（还有 {total - items.length} 条）</Button></div>
        )}
      </div>
    </Dialog>
  );
}
