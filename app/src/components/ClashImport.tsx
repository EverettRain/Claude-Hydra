import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { parseDocument } from "yaml";
import { emptyNode } from "@/lib/node";
import { errorText, type NodeConfig } from "@/lib/api";
import { Button } from "./ui/button";
import { Input, Select } from "./ui/input";

function nodes(raw: string): NodeConfig[] {
  if (raw.length > 4194304) throw new Error("配置超过 4 MB。");
  const doc = parseDocument(raw);
  if (doc.errors.length) throw new Error("无法解析 YAML，请选择 Clash 配置或 Clash 格式订阅。");
  const rows = doc.toJS({ maxAliasCount: 50 })?.proxies;
  if (!Array.isArray(rows)) throw new Error("配置中没有 proxies 节点列表。");
  return rows.flatMap(p => {
    if (!p || typeof p.server !== "string" || typeof p.name !== "string" || typeof p.type !== "string") return [];
    const port = Number(p.port);
    if (!Number.isInteger(port) || port <= 0 || port > 65535) return [];
    return [{ ...emptyNode(), kind: "clash" as const, host: p.server, port, password: JSON.stringify(p) }];
  });
}

export function ClashImport({ node, choose }: { node: NodeConfig | null; choose: (n: NodeConfig) => void }) {
  const input = useRef<HTMLInputElement>(null);
  const [url, setUrl] = useState("");
  const [list, setList] = useState<NodeConfig[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function load(source: Promise<string>) {
    setBusy(true); setError(""); setList([]);
    try {
      const items = nodes(await source);
      if (!items.length) throw new Error("没有可选择的节点。");
      setList(items);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  const name = (n: NodeConfig) => String(JSON.parse(n.password).name);
  return <div className="space-y-2 px-4 py-3">
    <div className="flex items-center gap-2">
      <Button variant="outline" disabled={busy} onClick={() => input.current?.click()}>导入 Clash YAML</Button>
      <input ref={input} hidden type="file" accept=".yaml,.yml,.json" onChange={e => {
        const f = e.target.files?.[0]; e.target.value = ""; if (f) void load(f.size <= 4194304 ? f.text() : Promise.reject(new Error("配置超过 4 MB。")));
      }} />
      <Input aria-label="Clash 订阅地址" type="password" placeholder="Clash 订阅地址" value={url} onChange={e => setUrl(e.target.value)} />
      <Button variant="outline" disabled={busy || !url.trim()} onClick={() => load(invoke<string>("clash_subscription", { url }))}>{busy ? "读取中…" : "读取订阅"}</Button>
    </div>
    {!!list.length && <Select aria-label="选择 Clash 节点" value={list.findIndex(n => n.password === node?.password)} onChange={e => { const n=list[Number(e.target.value)]; if(n) choose(n); }}>
      <option value={-1}>选择节点（{list.length}）</option>
      {list.map((n,i) => <option key={i} value={i}>{name(n)} · {JSON.parse(n.password).type}</option>)}
    </Select>}
    {node?.kind === "clash" && <p className="text-xs">已绑定：{name(node)} · {node.host}:{node.port}</p>}
    {node?.kind === "clash" && <p className="text-xs text-muted">保存环境后，在“账号 → 编辑”中绑定此环境。运行节点需要 Mihomo 内核。</p>}
    {error && <p role="alert" className="text-xs text-danger">{error}</p>}
  </div>;
}
