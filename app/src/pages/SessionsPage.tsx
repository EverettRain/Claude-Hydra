import { CheckCircle2, ClipboardCopy, FileJson, FolderSearch, Loader2, Search, Upload } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { AccountPicker } from "@/components/AccountPicker";
import { CodeSessionMigration } from "@/components/CodeSessionMigration";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile } from "@/lib/api";
import { readChats } from "@/lib/chatExport";
import { cn } from "@/lib/utils";

const PROMPT = "请阅读附件中的旧对话，理解上下文后从最后一个未完成的问题继续。";

type Chat = ReturnType<typeof readChats>[number];
interface Done { path: string; account: string; title: string }

/** Turns one exported chat into an attachment, then opens the chosen account ready to continue it. */
function ChatImport({ profiles }: { profiles: Profile[] | null }) {
  const [chats, setChats] = useState<Chat[]>([]);
  const [fileName, setFileName] = useState("");
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState<number | null>(null);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState<Done | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return chats.map((c, i) => ({ c, i })).filter(({ c }) => !q || c.title.toLowerCase().includes(q));
  }, [chats, query]);
  const chat = index === null ? null : chats[index];
  const targetName = profiles?.find((p) => p.id === target)?.name;

  async function read(file?: File) {
    setChats([]); setIndex(null); setError(""); setDone(null); setQuery(""); setFileName(file?.name ?? "");
    if (!file) return;
    setBusy(true);
    try {
      const list = readChats(await file.text());
      setChats(list);
      if (list.length === 1) setIndex(0);
    } catch (e) {
      setError(e instanceof SyntaxError ? "JSON 文件无法读取。请解压导出包并选择 conversations.json。" : errorText(e));
    } finally {
      setBusy(false);
      if (input.current) input.current.value = "";
    }
  }

  async function importChat() {
    if (!chat || !target) return;
    setBusy(true); setError(""); setDone(null);
    try {
      const path = await api.saveTranscript([`# ${chat.title}`, ...chat.turns].join("\n\n"));
      await navigator.clipboard.writeText(PROMPT).catch(() => undefined);
      await api.launch(target);
      await api.revealTranscript(path).catch(() => undefined);
      setDone({ path, account: targetName ?? target, title: chat.title });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Group label="普通聊天续聊">
      <div className="px-4 py-3">
        <input ref={input} aria-label="选择普通聊天导出文件" type="file" accept=".json,application/json" className="hidden" disabled={busy} onChange={(e) => void read(e.target.files?.[0])} />
        <button
          onClick={() => input.current?.click()}
          disabled={busy}
          className="flex w-full items-center gap-3 rounded-lg border border-dashed border-line bg-card-2 px-3.5 py-3 text-left hover:border-faint hover:bg-pill active:!scale-100"
        >
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-card text-fg-2 shadow-card">
            {fileName ? <FileJson className="h-4 w-4" /> : <Upload className="h-4 w-4" />}
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-medium">{fileName ? fileName : "选择 conversations.json"}</span>
            <span className="block truncate text-[11.5px] text-muted">
              {fileName && chats.length ? `${chats.length} 个可续聊的聊天 · 点击更换文件` : "在来源账号的 Claude 中打开“设置 → 隐私 → 导出数据”，解压后选择其中的 conversations.json"}
            </span>
          </span>
        </button>
      </div>

      {chats.length > 0 && (
        <div className="px-4 py-3">
          {chats.length > 6 && (
            <div className="relative mb-2">
              <Search className="pointer-events-none absolute left-2.5 top-2 h-4 w-4 text-faint" />
              <input aria-label="搜索聊天" className="field pl-8" placeholder="搜索聊天标题" value={query} onChange={(e) => setQuery(e.target.value)} />
            </div>
          )}
          <div role="radiogroup" aria-label="选择聊天" className="max-h-56 space-y-0.5 overflow-y-auto">
            {shown.map(({ c, i }) => (
              <button
                key={i}
                role="radio"
                aria-checked={index === i}
                onClick={() => { setIndex(i); setDone(null); }}
                className={cn("flex w-full items-center gap-2.5 rounded-[7px] px-2.5 py-1.5 text-left active:!scale-100", index === i ? "bg-pill" : "hover:bg-card-2")}
              >
                <span className={cn("flex h-4 w-4 shrink-0 items-center justify-center rounded-full border", index === i ? "border-fg bg-fg" : "border-faint")}>
                  {index === i && <span className="h-1.5 w-1.5 rounded-full bg-card" />}
                </span>
                <span className="min-w-0 flex-1 truncate text-[13px]">{c.title}</span>
                <span className="shrink-0 text-[11px] tabular-nums text-muted">{c.turns.length} 条</span>
              </button>
            ))}
            {!shown.length && <p className="px-2.5 py-2 text-[12px] text-muted">没有匹配的聊天。</p>}
          </div>
        </div>
      )}

      {chat && (
        <div className="grid grid-cols-[56px_1fr] items-start gap-3 px-4 py-3">
          <span className="pt-1.5 text-[12.5px] text-fg-2">导入到</span>
          <AccountPicker label="导入到账号" profiles={profiles} value={target} disabled={busy} onChange={(id) => { setTarget(id); setDone(null); }} />
        </div>
      )}

      {chat && (
        <div className="flex items-center gap-3 px-4 py-3">
          <p className="min-w-0 flex-1 text-[11.5px] leading-[18px] text-muted">
            将保存所选聊天的文本到下载目录、复制续聊提示，并打开目标账号；原上传的文件需另行添加。
          </p>
          <Button disabled={busy || !target} onClick={importChat}>
            {busy && <Loader2 className="animate-spin" />}
            {targetName ? `导入到 ${targetName}` : "选择目标账号"}
          </Button>
        </div>
      )}

      {done && (
        <div className="space-y-2.5 bg-ok-soft/50 px-4 py-3">
          <div className="flex items-center gap-2 text-[13px] font-medium text-ok">
            <CheckCircle2 className="h-4 w-4" />
            已打开 {done.account}，还差两步
          </div>
          <ol className="list-decimal space-y-1 pl-9 text-[12.5px] leading-5 text-fg-2">
            <li>在 Claude 中新建聊天，把文件管理器里选中的附件拖进去。</li>
            <li>粘贴已复制的续聊提示（{navigator.userAgent.includes("Mac") ? "⌘V" : "Ctrl+V"}）并发送。</li>
          </ol>
          <div className="flex flex-wrap gap-1.5 pl-6">
            <Button variant="outline" size="sm" onClick={() => api.revealTranscript(done.path).catch((e) => setError(errorText(e)))}>
              <FolderSearch />
              再次显示附件
            </Button>
            <Button variant="outline" size="sm" onClick={() => navigator.clipboard.writeText(PROMPT).catch(() => setError("无法写入剪贴板。"))}>
              <ClipboardCopy />
              重新复制提示
            </Button>
          </div>
        </div>
      )}

      {error && <p role="alert" className="selectable px-4 py-2.5 text-[12.5px] text-danger">{error}</p>}
    </Group>
  );
}

export function SessionsPage({ profiles }: { profiles: Profile[] | null }) {
  const [source, setSource] = useState("");
  return (
    <Page>
      <PageHeader title="会话迁移" subtitle="在账号之间复制本地 Claude Code 会话，或把普通聊天带到另一个账号继续。" />
      <div className="space-y-5 px-5">
        <CodeSessionMigration profiles={profiles} source={source} setSource={setSource} />
        {!source && <ChatImport profiles={profiles} />}
      </div>
    </Page>
  );
}
