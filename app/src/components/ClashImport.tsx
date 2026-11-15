import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { parseDocument } from "yaml";
import { MAX_FRONT, emptyNode, fronts, hopLabel } from "@/lib/node";
import { api, errorText, type ClashSubscription, type NodeConfig } from "@/lib/api";
import { Button } from "./ui/button";
import { Input, Select } from "./ui/input";

/**
 * Nodes in a subscription or pasted text: a Clash configuration (its `proxies`, a bare list of
 * proxies, or one proxy), else share links (one, several, or base64 as v2ray subscriptions are).
 */
async function readNodes(raw: string): Promise<{ items: NodeConfig[]; skipped: number }> {
  if (raw.length > 4194304) throw new Error("内容超过 4 MB。");
  const doc = parseDocument(raw);
  const value = doc.errors.length ? null : doc.toJS({ maxAliasCount: 50 });
  const rows = Array.isArray(value?.proxies) ? value.proxies : Array.isArray(value) ? value
    : value && typeof value === "object" && "type" in value && "server" in value ? [value] : null;
  if (rows) return clashNodes(rows);
  const links = await api.parseNodeLinks(raw);
  return { items: links.nodes, skipped: links.skipped };
}

function clashNodes(rows: any[]): { items: NodeConfig[]; skipped: number } {
  const usable = (p: any) => {
    if (!p || typeof p.server !== "string" || typeof p.name !== "string" || typeof p.type !== "string") return false;
    const port = Number(p.port);
    return Number.isInteger(port) && port > 0 && port <= 65535;
  };
  const byName = new Map<string, any>(rows.filter(usable).map((p: any) => [p.name, p]));
  // A node with `dialer-proxy` is reached through another node of the same subscription:
  // that node is saved with it as its front proxy. Proxy groups and loops cannot be saved.
  const build = (p: any, seen: string[]): NodeConfig | null => {
    const node: NodeConfig = { ...emptyNode(), kind: "clash", host: p.server, port: Number(p.port), password: JSON.stringify(p) };
    const front = p["dialer-proxy"];
    if (front == null || front === "") return node;
    const next = typeof front === "string" ? byName.get(front) : undefined;
    if (!next || seen.includes(front) || seen.length >= MAX_FRONT) return null;
    const via = build(next, [...seen, front]);
    return via && { ...node, via };
  };
  let skipped = 0;
  const items = rows.filter(usable).flatMap((p: any) => {
    const node = build(p, [p.name]);
    if (!node) skipped++;
    return node ? [node] : [];
  });
  return { items, skipped };
}

/**
 * Reads a Clash subscription and picks one of its nodes: the exit node, or with `asFront` a front
 * proxy the exit is reached through (e.g. an airport node before a residential proxy).
 */
export function ClashImport({ subscription, onSubscriptionChange, node, choose, asFront }: { subscription?: ClashSubscription | null; onSubscriptionChange: (s: ClashSubscription) => void; node: NodeConfig | null; choose: (n: NodeConfig) => void; asFront?: boolean }) {
  const [url, setUrl] = useState<string | null>(null);
  const address = url ?? subscription?.url ?? "";
  const list = subscription?.nodes ?? [];
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const [pasted, setPasted] = useState("");
  async function load(source: Promise<string>) {
    setBusy(true); setError(""); setNote("");
    try {
      const { items, skipped } = await readNodes(await source);
      if (!items.length) throw new Error(skipped ? "订阅中的节点都依赖代理组或缺失的前置节点，无法单独使用。" : "没有可选择的节点。");
      if (skipped) setNote(`已略过 ${skipped} 个节点：它们的前置节点（dialer-proxy）是代理组、不存在、成环或超过 ${MAX_FRONT} 级。`);
      onSubscriptionChange({ url: address.trim(), nodes: items });
      setUrl(null);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  // One pasted node is used at once; several become the list to pick from, like a subscription.
  async function usePasted(text: string) {
    if (!text.trim()) return;
    setPasted(""); setBusy(true); setError(""); setNote("");
    try {
      const { items, skipped } = await readNodes(text);
      if (!items.length) throw new Error("没有可识别的节点。");
      const left = skipped ? `已略过 ${skipped} 个无法识别的节点。` : "";
      if (items.length === 1) { choose(items[0]); setNote(left); }
      else { onSubscriptionChange({ url: "", nodes: items }); setNote(`读取到 ${items.length} 个节点，请在下方选择。${left}`); }
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  }
  const name = (n: NodeConfig) => hopLabel(n);
  const chain = (n: NodeConfig) => fronts(n).map((v) => ` ← ${hopLabel(v)}`).join("");
  const same = (a: NodeConfig, b: NodeConfig | null) => !!b && a.password === b.password && JSON.stringify(a.via ?? null) === JSON.stringify(b.via ?? null);
  return <div className="space-y-2 px-4 py-2">
    <div className="flex items-center gap-2">
      <Input aria-label="Clash 订阅地址" type="password" placeholder="订阅地址（Clash 配置或节点链接列表）" value={address} disabled={busy} onChange={e => setUrl(e.target.value)} />
      <Button variant="outline" disabled={busy || !address.trim()} onClick={() => load(invoke<string>("clash_subscription", { url: address }))}>{busy ? "读取中…" : "读取订阅"}</Button>
    </div>
    {/* Share links carry their secrets, so the field never keeps what was pasted. */}
    <Input aria-label={asFront ? "粘贴前置节点" : "粘贴节点"} disabled={busy} value={pasted}
      placeholder="或粘贴节点链接（vless://、trojan://、ss:// 等）或 Clash 节点配置"
      onChange={(e) => setPasted(e.target.value)}
      onPaste={(e) => { e.preventDefault(); void usePasted(e.clipboardData.getData("text")); }}
      onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); void usePasted(pasted); } }} />
    {!list.length && node?.kind === "clash" && <p className="text-xs text-muted">此环境只保存了{asFront ? "所选的前置节点" : "绑定节点"}。读取订阅或粘贴其他节点即可切换。</p>}
    {!!list.length && <Select disabled={busy} aria-label={asFront ? "选择前置节点" : "选择 Clash 节点"} value={Math.max(list.findIndex(n => same(n, node)), list.findIndex(n => n.password === node?.password))} onChange={e => { const n=list[Number(e.target.value)]; if(n) choose(n); }}>
      <option disabled value={-1}>{asFront ? "选择前置节点" : "选择节点"}（{list.length}）</option>
      {list.map((n,i) => <option key={i} value={i}>{name(n)} · {JSON.parse(n.password).type}{n.via ? `（经 ${hopLabel(n.via)} 中转）` : ""}</option>)}
    </Select>}
    {!asFront && node?.kind === "clash" && <p className="text-xs">已绑定：{name(node)} · {node.host}:{node.port}{chain(node)}</p>}
    {!asFront && node?.kind === "clash" && <p className="text-xs text-muted">保存环境后，在“账号 → 编辑”中绑定此环境。</p>}
    {note && <p className="text-xs text-muted">{note}</p>}
    {error && <p role="alert" className="text-xs text-danger">{error}</p>}
  </div>;
}
