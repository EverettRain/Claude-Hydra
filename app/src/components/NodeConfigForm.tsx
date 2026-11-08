import { Copy, Info, Loader2, RefreshCw, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { api, errorText, type ExitInfo, type NodeConfig } from "@/lib/api";
import { COMMON_TIMEZONES, emptyNode, formatNode, parseNodeText } from "@/lib/node";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { Input } from "./ui/input";

interface Props {
  node: NodeConfig | null;
  onNodeChange: (node: NodeConfig | null) => void;
  timezone: string;
  onTimezoneChange: (tz: string) => void;
  lastExit?: ExitInfo | null;
  idPrefix?: string;
}

const select = "h-9 w-full rounded-lg border border-input bg-background px-3 text-sm";

function Row({ label, required, hint, children }: { label: string; required?: boolean; hint?: string; children: React.ReactNode }) {
  return <div className="grid grid-cols-[96px_1fr] items-center gap-3">
    <span className="text-sm text-foreground">{required && <span className="mr-0.5 text-red-500">*</span>}{label}</span>
    <div className="flex min-w-0 items-center gap-2">
      <div className="min-w-0 flex-1">{children}</div>
      {hint && <span title={hint} className="shrink-0 text-muted-foreground"><Info className="h-4 w-4" /></span>}
    </div>
  </div>;
}

function Radio({ name, value, current, label, onChange }: { name: string; value: string; current: string; label: string; onChange: (v: string) => void }) {
  return <label className="inline-flex cursor-pointer items-center gap-1.5 text-sm">
    <input type="radio" name={name} className="h-4 w-4 accent-blue-600" checked={current === value} onChange={() => onChange(value)} />{label}
  </label>;
}

export function exitText(e: ExitInfo) {
  return [e.ip, [e.country, e.region, e.city].filter(Boolean).join(" · "), e.timezone].filter(Boolean).join("　");
}

export function NodeConfigForm({ node, onNodeChange, timezone, onTimezoneChange, lastExit, idPrefix = "node" }: Props) {
  const [checking, setChecking] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const n = node ?? emptyNode();
  const set = (patch: Partial<NodeConfig>) => { setResult(null); onNodeChange({ ...n, ...patch }); };
  const tzMode = timezone === "system" ? "system" : !timezone || timezone === "proxy" ? "proxy" : "custom";

  // Split "type://host:port:user:pass" style strings on paste or when the field loses focus.
  const trySplit = (value: string) => {
    const parsed = value.includes(":") ? parseNodeText(value) : null;
    if (parsed) set(parsed);
    return !!parsed;
  };
  const check = async () => {
    setChecking(true); setResult(null);
    try { const info = await api.checkNode(n); setResult({ ok: true, text: exitText(info) }); }
    catch (e) { setResult({ ok: false, text: errorText(e) }); }
    finally { setChecking(false); }
  };
  const refresh = async () => {
    setRefreshing(true); setResult(null);
    try { setResult({ ok: true, text: await api.refreshNodeIp(n.refreshUrl) }); }
    catch (e) { setResult({ ok: false, text: errorText(e) }); }
    finally { setRefreshing(false); }
  };
  const copy = () => { void navigator.clipboard?.writeText(formatNode(n)); setResult({ ok: true, text: "已复制节点字符串（含密码）。" }); };

  return <div className="space-y-4">
    <Row label="代理方式" required>
      <div className="flex flex-wrap gap-4">
        <Radio name={`${idPrefix}-mode`} value="none" current={node ? "custom" : "none"} label="不使用代理" onChange={() => { setResult(null); onNodeChange(null); }} />
        <Radio name={`${idPrefix}-mode`} value="custom" current={node ? "custom" : "none"} label="自定义代理" onChange={() => onNodeChange(n)} />
      </div>
    </Row>

    {node && <>
      <Row label="IP查询渠道" hint="检测出口 IP、地区和时区所用的服务；请求经由代理节点发出。">
        <div className="flex gap-2">
          <select aria-label="IP查询渠道" className={select} value={n.geoChannel} onChange={e => set({ geoChannel: e.target.value as NodeConfig["geoChannel"] })}>
            <option value="ipinfo">ipinfo.io</option><option value="ipapi">ipapi.co</option><option value="ip-api">ip-api.com</option>
          </select>
          <Button variant="outline" disabled={checking || !n.host || !n.port} onClick={check}>{checking ? <Loader2 className="h-4 w-4 animate-spin" /> : null}代理检测</Button>
          <Button variant="ghost" size="icon" aria-label="复制节点字符串" disabled={!n.host || !n.port} onClick={copy}><Copy className="h-4 w-4" /></Button>
        </div>
      </Row>
      <Row label="代理类型" required>
        <select aria-label="代理类型" className={select} value={n.kind} onChange={e => set({ kind: e.target.value as NodeConfig["kind"] })}>
          <option value="socks5">Socks5</option><option value="http">HTTP</option><option value="https">HTTPS</option>
        </select>
      </Row>
      <Row label="IP协议" hint="检测并要求的出口 IP 版本。选 IPv6 时节点必须提供 IPv6 出口，否则无法启动。">
        <div className="flex gap-4">
          <Radio name={`${idPrefix}-ipv`} value="ipv4" current={n.ipVersion} label="IPv4" onChange={v => set({ ipVersion: v as NodeConfig["ipVersion"] })} />
          <Radio name={`${idPrefix}-ipv`} value="ipv6" current={n.ipVersion} label="IPv6" onChange={v => set({ ipVersion: v as NodeConfig["ipVersion"] })} />
        </div>
      </Row>
      <Row label="代理主机" required hint="支持粘贴“代理类型://主机:端口:账号:密码”或“账号:密码@主机:端口”，自动拆分。">
        <Input aria-label="代理主机" className="font-mono text-xs" value={n.host} placeholder="支持“代理类型://主机:端口:账号:密码”自动拆分" onChange={e => set({ host: e.target.value })}
          onPaste={e => { if (trySplit(e.clipboardData.getData("text"))) e.preventDefault(); }}
          onBlur={e => trySplit(e.target.value)} />
      </Row>
      <Row label="代理端口" required>
        <Input aria-label="代理端口" inputMode="numeric" className="font-mono text-xs" value={n.port || ""} placeholder="请输入端口"
          onChange={e => { const v = e.target.value.replace(/\D/g, "").slice(0, 5); set({ port: v ? Math.min(Number(v), 65535) : 0 }); }} />
      </Row>
      <Row label="代理账号" hint="选填。账号和密码只保存在本机（Windows 使用系统加密），不会出现在 Claude 的启动参数或环境变量中。">
        <Input aria-label="代理账号" value={n.username} maxLength={200} placeholder="请输入账号（选填）" autoComplete="off" onChange={e => set({ username: e.target.value })} />
      </Row>
      <Row label="代理密码">
        <Input aria-label="代理密码" type="password" value={n.password} maxLength={200} placeholder="请输入密码（选填）" autoComplete="new-password" onChange={e => set({ password: e.target.value })} />
      </Row>
      <Row label="刷新URL" hint="代理服务商提供的换 IP 链接。点击右侧按钮会直接请求此链接（不经过代理）。">
        <div className="flex gap-2">
          <Input aria-label="刷新URL" className="font-mono text-xs" value={n.refreshUrl} placeholder="刷新URL" onChange={e => set({ refreshUrl: e.target.value })} />
          <Button variant="ghost" size="icon" aria-label="请求刷新URL" disabled={!n.refreshUrl.trim() || refreshing} onClick={refresh}><RefreshCw className={cn("h-4 w-4", refreshing && "animate-spin")} /></Button>
        </div>
      </Row>
      {result && <p role="status" className={cn("selectable rounded-md p-2 text-xs", result.ok ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400" : "bg-red-500/10 text-red-600 dark:text-red-400")}>{result.text}</p>}
      <div className="flex gap-2 rounded-lg bg-muted/70 p-3 text-xs leading-5 text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600" />
        <div>
          已自动启用防泄漏：Claude 只连接本机中继 127.0.0.1，由中继带认证转发到节点；域名全部在节点解析（禁止本地 DNS），禁用 QUIC，UDP 一律拒绝，WebRTC 仅允许走代理；
          子进程（Claude Code、MCP）通过代理环境变量走同一节点。每次启动前会经此路径检测出口，失败则不启动；节点断开或 Claude-Hydra 退出时该账号断网，不会回退直连。
          无法拦截的情况：不读取代理环境变量、自行直连网络的子进程，以及系统浏览器中的登录页。
        </div>
      </div>
    </>}

    <Row label="时区" hint="通过 TZ 环境变量为该账号设置时区。macOS 上 Claude 窗口与子进程均生效；Windows 上 Claude 窗口可能仍使用系统时区，仅子进程确定生效。">
      <div className="flex flex-wrap items-center gap-4">
        <Radio name={`${idPrefix}-tz`} value="proxy" current={tzMode} label="跟随代理 IP" onChange={() => onTimezoneChange("proxy")} />
        <Radio name={`${idPrefix}-tz`} value="system" current={tzMode} label="跟随系统" onChange={() => onTimezoneChange("system")} />
        <Radio name={`${idPrefix}-tz`} value="custom" current={tzMode} label="自定义" onChange={() => onTimezoneChange(lastExit?.timezone || "Asia/Tokyo")} />
      </div>
    </Row>
    {tzMode === "custom" && <Row label="">
      <Input aria-label="自定义时区" list={`${idPrefix}-tz-list`} className="font-mono text-xs" value={timezone} placeholder="IANA 时区，例如 Asia/Tokyo" onChange={e => onTimezoneChange(e.target.value)} />
      <datalist id={`${idPrefix}-tz-list`}>{COMMON_TIMEZONES.map(t => <option key={t} value={t} />)}</datalist>
    </Row>}
    {tzMode === "proxy" && !node && <p className="text-xs text-muted-foreground">未设置代理时，“跟随代理 IP”等同于跟随系统。</p>}
    {lastExit && <p className="text-xs text-muted-foreground">上次启动检测：{exitText(lastExit)}（{new Date(lastExit.checkedAt).toLocaleString()}）</p>}
  </div>;
}
