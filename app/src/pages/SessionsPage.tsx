import { useState } from "react";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { api, errorText, type Profile } from "@/lib/api";
import { readChats } from "@/lib/chatExport";

export function SessionsPage({ profiles }: { profiles: Profile[] | null }) {
  const [chats, setChats] = useState<ReturnType<typeof readChats>>([]);
  const [index, setIndex] = useState(0);
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [savedPath, setSavedPath] = useState("");
  const chat = chats[index];

  async function read(file?: File) {
    setChats([]); setIndex(0); setSavedPath(""); setStatus("");
    if (!file) return;
    setBusy(true);
    try { setChats(readChats(await file.text())); }
    catch (e) { setStatus(e instanceof SyntaxError ? "JSON 文件无法读取。请解压导出包并选择 conversations.json。" : errorText(e)); }
    finally { setBusy(false); }
  }
  async function save() {
    if (!chat) return;
    setBusy(true); setStatus("");
    try { setSavedPath(await api.saveTranscript([chat.title, ...chat.turns].join("\n\n"))); }
    catch (e) { setStatus(errorText(e)); }
    finally { setBusy(false); }
  }
  async function openTarget() {
    setBusy(true); setStatus("");
    try {
      await api.launch(target);
      setStatus("请确认 Claude Desktop 显示的是目标账号，在新聊天中附上已保存的文本并发送续聊请求。");
    } catch (e) { setStatus(errorText(e)); }
    finally { setBusy(false); }
  }
  const field = "w-full rounded-md border border-border bg-background p-2";
  return <div className="pb-8"><PageHeader title="普通聊天续聊" /><div className="space-y-4 px-6">
    <p className="text-sm text-muted-foreground">在来源账号的 Claude Desktop 中打开「设置 → 隐私 → 导出数据」。下载并解压导出包，然后选择 conversations.json。</p>
    <input aria-label="选择普通聊天导出文件" type="file" accept=".json" disabled={busy} onChange={e => void read(e.target.files?.[0])} />
    {chat && <section className="space-y-3 rounded-lg border border-border bg-card p-4">
      <label>来源聊天（{chats.length}）<select className={field} disabled={busy} value={index} onChange={e => { setIndex(Number(e.target.value)); setSavedPath(""); setStatus(""); }}>
        {chats.map((c,i) => <option key={i} value={i}>{c.title} · {c.turns.length} 条文本</option>)}</select></label>
      <details><summary>预览聊天开头</summary><p className="whitespace-pre-wrap break-words text-sm">{chat.turns[0].slice(0,800)}</p></details>
      <p className="text-sm text-muted-foreground">附件包含所选聊天的可读文本。原上传文件需要另行添加。目标账号收到的是新聊天。</p>
      <Button disabled={busy} onClick={save}>保存续聊附件到下载目录</Button>
      {savedPath && <p className="selectable break-all text-sm">已保存：{savedPath}</p>}
      <label className="block">目标账号<select className={field} value={target} disabled={busy} onChange={e => setTarget(e.target.value)}>
        <option value="">选择账号</option>{profiles?.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select></label>
      <Button disabled={busy || !target || !savedPath} onClick={openTarget}>打开目标账号</Button>
      {savedPath && <p className="text-sm">在新聊天中附上文件，然后发送「请阅读这段旧对话，从最后一个未完成的问题继续。」</p>}
    </section>}
    {status && <p role="status" className="text-sm">{status}</p>}
  </div></div>;
}
