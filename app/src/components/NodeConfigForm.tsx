import { ClashImport } from "./ClashImport";
import { Check, CircleAlert, CircleCheck, Clock, Copy, Info, Loader2, RefreshCw, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { api, errorText, type ExitInfo, type NodeCheck, type NodeConfig } from "@/lib/api";
import { COMMON_TIMEZONES, emptyNode, formatNode, parseNodeText } from "@/lib/node";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { Input, Select } from "./ui/input";

interface Props {
  node: NodeConfig | null;
  onNodeChange: (node: NodeConfig | null) => void;
  timezone: string;
  onTimezoneChange: (tz: string) => void;
  lastExit?: ExitInfo | null;
  /** Last successful connectivity check, saved with the environment. */
  lastCheck?: NodeCheck | null;
  /** Exit timezone confirmed earlier for this exact node (e.g. by an account's launch). */
  detectedTimezone?: string | null;
  onLastCheckChange?: (check: NodeCheck | null) => void;
  idPrefix?: string;
}

function Row({ label, required, hint, children }: { label: string; required?: boolean; hint?: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[88px_1fr] items-center gap-3 px-4 py-2">
      <span className="flex items-center gap-1 text-[12.5px] text-fg-2">
        {label}
        {required && <span className="text-danger">*</span>}
        {hint && (
          <span title={hint} className="text-faint hover:text-muted">
            <Info className="h-3 w-3" />
          </span>
        )}
      </span>
      <div className="flex min-w-0 items-center gap-2">{children}</div>
    </div>
  );
}

/** Segmented control: quiet track, white thumb on the selected option. */
function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: [T, string][]; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex rounded-lg bg-pill p-0.5">
      {options.map(([v, text]) => (
        <button
          key={v}
          role="radio"
          aria-checked={value === v}
          onClick={() => onChange(v)}
          className={cn("h-6 rounded-md px-2.5 text-[12.5px] active:!scale-100", value === v ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg")}
        >
          {text}
        </button>
      ))}
    </div>
  );
}

export function exitText(e: ExitInfo) {
  return [e.ip, [e.country, e.region, e.city].filter(Boolean).join(" · "), e.timezone].filter(Boolean).join("　");
}

export function checkText(c: NodeCheck) {
  return `已连通　${exitText(c.exit)}\n${c.quality ?? "家宽 / 机房可能性：未知（未取得分类依据）"}\n${c.risk}`;
}

export function NodeConfigForm({ node, onNodeChange, timezone, onTimezoneChange, lastExit, lastCheck, onLastCheckChange, detectedTimezone, idPrefix = "node" }: Props) {
  const [checking, setChecking] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [copied, setCopied] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const n = node ?? emptyNode();
  // Any change to the node makes earlier check results stale.
  const changeNode = (next: NodeConfig | null) => { setResult(null); onLastCheckChange?.(null); onNodeChange(next); };
  const set = (patch: Partial<NodeConfig>) => changeNode({ ...n, ...patch });
  const [pickedMode, setPickedMode] = useState<"clash" | "custom">("clash");
  const mode = node ? (node.kind === "clash" ? "clash" : "custom") : pickedMode;
  const switchMode = (v: "clash" | "custom") => {
    setResult(null);
    setPickedMode(v);
    if (v === mode) return;
    // Clash nodes come only from a subscription; custom proxies start from a blank form.
    changeNode(v === "custom" ? emptyNode() : null);
  };
  // A fresh message (error, IP refresh) takes precedence over the saved check.
  const shown = result ?? (lastCheck ? { ok: true, text: `${checkText(lastCheck)}\n检测时间：${new Date(lastCheck.exit.checkedAt).toLocaleString()}` } : null);
  // The zone "follow proxy" resolves to, once known: a check in this form wins over earlier ones.
  const checkedTz = node ? lastCheck?.exit.timezone || detectedTimezone || lastExit?.timezone || undefined : undefined;
  const tzMode = timezone === "system" ? "system" : !timezone || timezone === "proxy" ? "proxy" : "custom";

  // Split "type://host:port:user:pass" style strings on paste or when the field loses focus.
  const trySplit = (value: string) => {
    const parsed = value.includes(":") ? parseNodeText(value) : null;
    if (parsed) set(parsed);
    return !!parsed;
  };
  const check = async () => {
    setChecking(true); setResult(null);
    try {
      const [exit, quality, risk] = await api.checkNode(n);
      const c: NodeCheck = { exit, quality, risk };
      if (onLastCheckChange) onLastCheckChange(c);
      else setResult({ ok: true, text: checkText(c) });
    }
    catch (e) { setResult({ ok: false, text: errorText(e) }); }
    finally { setChecking(false); }
  };
  const refresh = async () => {
    setRefreshing(true); setResult(null);
    try { setResult({ ok: true, text: await api.refreshNodeIp(n.refreshUrl) }); }
    catch (e) { setResult({ ok: false, text: errorText(e) }); }
    finally { setRefreshing(false); }
  };
  const copy = () => {
    void navigator.clipboard?.writeText(formatNode(n));
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };

  return (
    <fieldset disabled={checking || refreshing} className="list-card min-w-0">
      <Row label="代理方式" required>
        <Segmented label="代理方式" value={mode} options={[["clash", "Clash 代理"], ["custom", "自定义代理"]]} onChange={switchMode} />
      </Row>

      {mode === "clash" && <ClashImport node={node} choose={changeNode} />}
      {node && (
        <>
          {mode === "custom" && <>
          <Row label="代理类型" required>
            <Select aria-label="代理类型" className="w-36" value={n.kind} onChange={(e) => set({ kind: e.target.value as NodeConfig["kind"] })}>
              <option value="socks5">SOCKS5</option>
              <option value="http">HTTP</option>
              <option value="https">HTTPS</option>
            </Select>
            <span className="flex-1" />
            <Segmented label="IP协议" value={n.ipVersion} options={[["ipv4", "IPv4"], ["ipv6", "IPv6"]]} onChange={(v) => set({ ipVersion: v })} />
          </Row>
          <Row label="代理主机" required hint="可粘贴“类型://主机:端口:账号:密码”或“账号:密码@主机:端口”，自动拆分到各栏。">
            <Input aria-label="代理主机" className="font-mono text-xs" value={n.host} placeholder="粘贴 类型://主机:端口:账号:密码 自动拆分"
              onChange={(e) => set({ host: e.target.value })}
              onPaste={(e) => { if (trySplit(e.clipboardData.getData("text"))) e.preventDefault(); }}
              onBlur={(e) => trySplit(e.target.value)} />
            <Input aria-label="代理端口" inputMode="numeric" className="w-24 shrink-0 font-mono text-xs" value={n.port || ""} placeholder="端口"
              onChange={(e) => { const v = e.target.value.replace(/\D/g, "").slice(0, 5); set({ port: v ? Math.min(Number(v), 65535) : 0 }); }} />
          </Row>
          <Row label="账号密码" hint="选填。只保存在本机（Windows 使用系统加密），不会出现在 Claude 的启动参数或环境变量中。">
            <Input aria-label="代理账号" value={n.username} maxLength={200} placeholder="账号（选填）" autoComplete="off" onChange={(e) => set({ username: e.target.value })} />
            <Input aria-label="代理密码" type="password" value={n.password} maxLength={200} placeholder="密码（选填）" autoComplete="new-password" onChange={(e) => set({ password: e.target.value })} />
          </Row>
          </>}
          <Row label="出口检测" hint="经节点访问所选服务，检测出口 IP、地区与时区。IP 协议选 IPv6 时节点须提供 IPv6 出口。">
            <Select aria-label="IP查询渠道" value={n.geoChannel} onChange={(e) => set({ geoChannel: e.target.value as NodeConfig["geoChannel"] })}>
              <option value="ipinfo">ipinfo.io</option>
              <option value="ipapi">ipapi.co</option>
              <option value="ip-api">ip-api.com</option>
            </Select>
            <Button variant="outline" disabled={checking || !n.host || !n.port} onClick={check}>
              {checking && <Loader2 className="animate-spin" />}
              连通检测
            </Button>
            {mode === "custom" && <Button variant="ghost" size="icon" aria-label="复制节点字符串（含密码）" title="复制节点字符串（含密码）" disabled={!n.host || !n.port} onClick={copy}>
              {copied ? <Check className="text-ok" /> : <Copy />}
            </Button>}
          </Row>
          {mode === "custom" && <Row label="刷新URL" hint="代理服务商提供的换 IP 链接。点击右侧按钮会直接请求此链接（不经过代理）。">
            <Input aria-label="刷新URL" className="font-mono text-xs" value={n.refreshUrl} placeholder="https://…" onChange={(e) => set({ refreshUrl: e.target.value })} />
            <Button variant="ghost" size="icon" aria-label="请求刷新URL" disabled={!n.refreshUrl.trim() || refreshing} onClick={refresh}>
              <RefreshCw className={cn(refreshing && "animate-spin")} />
            </Button>
          </Row>}
          {(shown) && (
            <div className={cn("flex gap-2.5 px-4 py-3 text-[11.5px] leading-[18px]", shown.ok ? "bg-ok-soft/70 text-ok" : "bg-danger-soft/70 text-danger")} role="status">
              {shown.ok ? <CircleCheck className="mt-px h-4 w-4 shrink-0" /> : <CircleAlert className="mt-px h-4 w-4 shrink-0" />}
              <p className="selectable min-w-0 whitespace-pre-line">{shown.text}</p>
            </div>
          )}
          <div className="flex gap-2.5 px-4 py-3 text-[11.5px] leading-[18px] text-muted">
            <ShieldCheck className="mt-px h-4 w-4 shrink-0 text-ok" />
            <p>
              已自动启用防泄漏：Claude 只连本机中继，由中继带认证转发到节点；域名在节点解析，禁用 QUIC，拒绝 UDP，WebRTC 仅走代理，子进程通过代理环境变量走同一节点。
              启动前会检测出口，失败则不启动；节点断开或 Hydra 退出时该账号断网，不回退直连。
            </p>
          </div>
        </>
      )}

      <Row label="时区" hint="通过 TZ 环境变量设置。macOS 上窗口与子进程均生效；Windows 上 Claude 窗口可能仍显示系统时区。">
        <Segmented label="时区" value={tzMode} options={[["proxy", checkedTz ?? "跟随代理 IP"], ["system", "跟随系统"], ["custom", "自定义"]]}
          onChange={(v) => onTimezoneChange(v === "custom" ? checkedTz || lastExit?.timezone || "Asia/Tokyo" : v)} />
      </Row>
      {tzMode === "custom" && (
        <Row label="">
          <Input aria-label="自定义时区" list={`${idPrefix}-tz-list`} className="font-mono text-xs" value={timezone} placeholder="IANA 时区，例如 Asia/Tokyo" onChange={(e) => onTimezoneChange(e.target.value)} />
          <datalist id={`${idPrefix}-tz-list`}>{COMMON_TIMEZONES.map((t) => <option key={t} value={t} />)}</datalist>
        </Row>
      )}
      {tzMode === "proxy" && node && checkedTz && (
        <div className="flex items-center gap-2.5 px-4 py-2 text-[11.5px] text-fg-2">
          <Clock className="h-4 w-4 shrink-0 text-muted" />
          <p>跟随代理 IP，已匹配时区：<span className="selectable font-mono">{checkedTz}</span></p>
        </div>
      )}
      {(lastExit || (tzMode === "proxy" && !node)) && (
        <div className="px-4 py-2 text-[11.5px] text-muted">
          {tzMode === "proxy" && !node && <p>未设置代理时，“跟随代理 IP”等同于跟随系统。</p>}
          {lastExit && <p>上次启动检测：{exitText(lastExit)}（{new Date(lastExit.checkedAt).toLocaleString()}）</p>}
        </div>
      )}
    </fieldset>
  );
}
