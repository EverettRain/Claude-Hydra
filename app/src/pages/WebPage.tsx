import { Activity, Box, ExternalLink, Fingerprint, Globe, Loader2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { exitText } from "@/components/NodeConfigForm";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { ProfileAvatar } from "@/components/ProfileAvatar";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type BrowserCheck, type BrowserFingerprint, type Container, type Profile } from "@/lib/api";
import { cn } from "@/lib/utils";

/** An account whose own browser can be opened. */
interface Target {
  id: string;
  name: string;
  environment: string | null;
  blocked: string | null;
  blockedSoft: boolean;
  profile?: Profile;
}

/** The last route check of a row, from 「测试」 or from opening the browser. */
type Result = { check: BrowserCheck; at: number } | { error: string };

function fromProfile(p: Profile): Target {
  return { id: p.id, name: p.name, environment: p.environment, blocked: p.blocked, blockedSoft: p.blockedSoft, profile: p };
}

function fromContainer(c: Container): Target {
  return { id: c.id, name: c.name, environment: c.environmentName, blocked: c.blocked, blockedSoft: c.blockedSoft };
}

function ResultLine({ r }: { r: Result }) {
  if ("error" in r) return <div className="selectable mt-1 whitespace-pre-line text-[11.5px] leading-[1.5] text-danger">{r.error}</div>;
  const c = r.check;
  const browser = [`浏览器时区 ${c.timezone ?? "跟随系统"}`, `语言 ${c.locale || "跟随系统"}`].join("　");
  return (
    <div className="selectable mt-1 text-[11.5px] leading-[1.5] text-fg-2">
      <div>
        <span className="text-ok">{c.exit ? "出口合格" : "未经代理"}</span>
        {"　"}{c.exit ? exitText(c.exit) : "使用本机网络直连"}
      </div>
      <div className="text-muted">{browser}　检测于 {new Date(r.at).toLocaleTimeString()}</div>
      {(c.quality || c.risk) && <div className="text-muted">{[c.quality, c.risk].filter(Boolean).join("　")}</div>}
    </div>
  );
}

function Row({ t, busy, result, onTest, onConfig, onOpen }: { t: Target; busy?: string; result?: Result; onTest: () => void; onConfig: () => void; onOpen: () => void }) {
  return (
    <div className="flex items-start gap-3 px-4 py-3">
      {t.profile
        ? <ProfileAvatar badge={t.profile.badge} isDefault={t.profile.isDefault} color={t.profile.color} className="h-8 w-8 shrink-0" />
        : <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-pill text-fg-2"><Box className="h-4 w-4" /></span>}
      <div className="min-w-0 flex-1 pt-[7px]">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[13.5px] font-medium">{t.name}</span>
          {t.environment && <Tag tone="env">{t.environment}</Tag>}
          {t.blocked && (t.blockedSoft ? <Tag tone="warn" title={t.blocked}>核验中断网</Tag> : <Tag tone="danger" title={t.blocked}>已断网</Tag>)}
        </div>
        {t.blocked && <div className={cn("mt-0.5 truncate text-[11.5px]", t.blockedSoft ? "text-warn" : "text-danger")} title={t.blocked}>出口保护：{t.blocked}</div>}
        {result && <ResultLine r={result} />}
      </div>
      {busy ? (
        <span className="flex h-8 items-center gap-1.5 px-2 text-[12.5px] text-muted"><Loader2 className="h-3.5 w-3.5 animate-spin" />{busy}</span>
      ) : (
        <div className="flex h-8 shrink-0 items-center gap-1">
          <Button variant="ghost" size="sm" onClick={onTest} title="经该账号的中继检测出口，不打开浏览器"><Activity />测试</Button>
          <Button variant="ghost" size="sm" onClick={onConfig}><Fingerprint />配置</Button>
          <Button variant="outline" size="sm" className="min-w-[72px]" onClick={onOpen}><ExternalLink />打开</Button>
        </div>
      )}
    </div>
  );
}

function Item({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[92px_1fr] gap-3 py-2 text-[12.5px] leading-[1.55]">
      <div className="text-muted">{label}</div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function timezoneText(f: BrowserFingerprint) {
  if (f.timezone === "system" || ((!f.timezone || f.timezone === "proxy") && !f.proxied)) return "跟随系统";
  if (f.timezone && f.timezone !== "proxy") return f.timezone;
  return f.exitTimezone ? `跟随出口（上次检测为 ${f.exitTimezone}）` : "跟随出口（打开时检测）";
}

/** What the account's browser presents to pages. */
function FingerprintDialog({ target, onClose }: { target: Target | null; onClose: () => void }) {
  const toast = useToast();
  const [fp, setFp] = useState<BrowserFingerprint | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setFp(null); setError(null);
    if (target) api.browserFingerprint(target.id).then(setFp, (e) => setError(errorText(e)));
  }, [target]);

  const choose = async (choice: string | null) => {
    if (!target) return;
    setSaving(true);
    try { setFp(await api.setAccountBrowser(target.id, choice)); }
    catch (e) { toast(errorText(e), "error"); }
    finally { setSaving(false); }
  };

  // A choice whose browser was uninstalled stays listed, so the select shows what is set.
  const options = fp ? [...fp.browsers, ...(fp.browserChoice && !fp.browsers.includes(fp.browserChoice) ? [fp.browserChoice] : [])] : [];
  return (
    <Dialog
      open={!!target}
      onClose={onClose}
      className="max-w-lg"
      title={`${target?.name ?? ""} 的浏览器指纹`}
      description="网站从这个账号的浏览器读到的特征。时区与语言来自账号的环境配置，在「环境」中修改。"
      footer={<Button onClick={onClose}>完成</Button>}
    >
      {error && <div className="selectable text-[12.5px] text-danger">{error}</div>}
      {!fp && !error && <div className="flex items-center gap-1.5 py-6 text-[12.5px] text-muted"><Loader2 className="h-3.5 w-3.5 animate-spin" />读取中</div>}
      {fp && (
        <div className="selectable divide-y divide-line">
          <Item label="浏览器">
            {options.length > 0 ? (
              <select
                aria-label="账号浏览器使用的浏览器"
                className="field -my-1 max-w-[260px]"
                value={fp.browserChoice ?? ""}
                disabled={saving}
                onChange={(e) => void choose(e.target.value || null)}
              >
                <option value="">自动（{fp.browsers[0] ?? "未找到"}）</option>
                {options.map((b) => <option key={b} value={b}>{fp.browsers.includes(b) ? b : `${b}（未找到）`}</option>)}
              </select>
            ) : fp.browser && <div>{fp.browser}</div>}
            {fp.browserPath && <div className="mt-1.5 truncate font-mono text-[11px] text-muted" title={fp.browserPath}>{fp.browserPath}</div>}
            {fp.browserError && <div className="mt-1 text-danger">{fp.browserError}</div>}
            {options.length > 0 && (
              <div className="mt-1 text-muted">
                {"不同账号选不同的浏览器，网站读到的浏览器品牌与 User-Agent 就不同；Brave 还会按网站自动变化 Canvas 与音频读数，这是它自带的隐私功能，检测站会识别为 Brave 的正常行为。换用后网站会把它看作另一台设备，需要重新登录；正在运行的账号浏览器关闭后才会切换。"}
              </div>
            )}
          </Item>
          <Item label="时区">{timezoneText(fp)}</Item>
          <Item label="语言">{fp.locale || "跟随系统"}</Item>
          <Item label="网络">
            {fp.proxied
              ? "经账号的节点联网，域名在节点解析；禁用 QUIC，WebRTC 只走代理，不暴露本机地址"
              : <span className="text-warn">未绑定代理节点，使用本机网络直连</span>}
          </Item>
          <Item label="自动化标记">navigator.webdriver 为 false，与手动打开的浏览器一致</Item>
          <Item label="硬件特征">
            <div>Hydra 不改动 Canvas、WebGL、音频、User-Agent、屏幕、CPU、内存、显卡与字体，网站读到的都是浏览器自己的真实值：任何改动过的读数都可能被指纹检测识别为伪装</div>
            <div className="text-muted">因此同一台电脑上各账号的这些特征相同；账号之间靠浏览器、出口 IP、时区、语言与各自独立的 Cookie 区分</div>
          </Item>
          <Item label="数据目录"><span className="break-all font-mono text-[11px] text-fg-2">{fp.dataDir}</span></Item>
        </div>
      )}
    </Dialog>
  );
}

/**
 * 网页端: each account's own browser, leaving through the account's node with its time zone,
 * language and page fingerprint, its cookies kept apart from every other account's.
 */
export function WebPage({ profiles }: { profiles: Profile[] | null }) {
  const toast = useToast();
  const [containers, setContainers] = useState<Container[] | null>(null);
  const [busy, setBusy] = useState<Record<string, string>>({});
  const [results, setResults] = useState<Record<string, Result>>({});
  const [configuring, setConfiguring] = useState<Target | null>(null);

  useEffect(() => {
    // Without a container runtime there are simply no container accounts to list.
    api.listContainers().then(setContainers, () => setContainers([]));
  }, []);

  const check = async (t: Target, label: string, action: () => Promise<BrowserCheck>, done?: string) => {
    setBusy((b) => ({ ...b, [t.id]: label }));
    try {
      const c = await action();
      setResults((r) => ({ ...r, [t.id]: { check: c, at: Date.now() } }));
      if (done) toast(done);
    } catch (e) {
      setResults((r) => ({ ...r, [t.id]: { error: errorText(e) } }));
    } finally {
      setBusy((b) => { const { [t.id]: _, ...rest } = b; return rest; });
    }
  };

  // The browser leaves through the account's node, so an account needs an environment.
  const desktop = (profiles ?? []).filter((p) => p.environment).map(fromProfile);
  // The default account is meant to stay without a node (see README), so it is not suggested.
  const unbound = (profiles ?? []).filter((p) => !p.environment && !p.isDefault);
  const boxes = (containers ?? []).map(fromContainer);
  const row = (t: Target) => (
    <Row
      key={t.id}
      t={t}
      busy={busy[t.id]}
      result={results[t.id]}
      onTest={() => void check(t, "测试中", () => api.testAccountBrowser(t.id))}
      onConfig={() => setConfiguring(t)}
      onOpen={() => void check(t, "检测出口", () => api.openAccountBrowser(t.id), `已打开 ${t.name} 的浏览器`)}
    />
  );

  return (
    <Page>
      <PageHeader title="网页端" subtitle="为每个账号打开独立的浏览器：经该账号的节点联网，时区与语言跟随账号，Cookie 与登录状态互不相通。" />
      <div className="space-y-5 px-5">
        {desktop.length > 0 && <Group label="桌面版账号"><div className="list-card">{desktop.map(row)}</div></Group>}
        {boxes.length > 0 && <Group label="容器账号"><div className="list-card">{boxes.map(row)}</div></Group>}
        {profiles && containers && !desktop.length && !boxes.length && (
          <div className="flex flex-col items-center gap-1.5 rounded-xl border border-dashed border-line px-4 py-10 text-muted">
            <Globe className="h-5 w-5" />
            <span className="text-[13px] font-medium">还没有可以打开浏览器的账号</span>
            <span className="text-[12px]">在「账号」中为账号选择带代理节点的环境，或在「容器」中新建容器账号</span>
          </div>
        )}
        {unbound.length > 0 && (
          <p className="px-1 text-[11.5px] text-muted">未绑定环境的账号不在此列出：{unbound.map((p) => p.name).join("、")}。浏览器只经账号的节点联网，请先在「账号」中为它们选择环境。</p>
        )}
        <p className="px-1 text-[11.5px] leading-relaxed text-muted">
          {"使用本机已安装的 Google Chrome、Microsoft Edge、Brave 或 Chromium，每个账号单独运行一个实例，和你日常使用的浏览器分开；在「配置」中可以为每个账号选用不同的浏览器。「打开」先经该账号的中继检测出口，出口不合格则不打开，通过后进入检测页 who.1ms.dog，查看网站看到的出口 IP、位置、时区与 WebRTC 等；「测试」只做同样的出口检测，并查询 IP 类型与风险记录，不打开浏览器。出口保护切断账号时，浏览器同样断网。要登录网页版 Claude，或打开容器里 claude /login 给出的登录链接，在账号浏览器的地址栏中粘贴即可。"}
        </p>
      </div>

      <FingerprintDialog target={configuring} onClose={() => setConfiguring(null)} />
    </Page>
  );
}
