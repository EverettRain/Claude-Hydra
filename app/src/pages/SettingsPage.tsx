import { Copy, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Group, Page, PageHeader, Row } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AppInfo, type ContainerRuntime, type Environment } from "@/lib/api";
import { appIconSrc } from "@/lib/appIcon";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";

interface Props {
  info: AppInfo | null;
  reloadInfo: () => Promise<void>;
  theme: ThemeMode;
  onThemeChange: (m: ThemeMode) => void;
  onInstallChanged: () => void;
}

function KeyValue({ k, v }: { k: string; v?: string | null }) {
  return (
    <div className="flex gap-4 px-4 py-2.5 text-[12.5px]">
      <span className="w-24 shrink-0 text-muted">{k}</span>
      <span className="selectable min-w-0 break-all font-mono text-[11.5px] leading-[18px] text-fg-2">{v || "—"}</span>
    </div>
  );
}

const themes: { id: ThemeMode; label: string }[] = [
  { id: "system", label: "跟随系统" },
  { id: "light", label: "浅色" },
  { id: "dark", label: "深色" },
];

export function SettingsPage({ info, reloadInfo, theme, onThemeChange, onInstallChanged }: Props) {
  const containerRuntime = info?.platform === "macos" ? "OrbStack" : info?.platform === "windows" ? "Docker Desktop" : null;
  const toast = useToast();
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => setPath(info?.claudePath ?? ""), [info?.claudePath]);

  const [runtime, setRuntime] = useState<ContainerRuntime | null>(null);
  const [dockerPath, setDockerPath] = useState("");
  const loadRuntime = async () => {
    try {
      const rt = await api.containerRuntime();
      setRuntime(rt);
      setDockerPath(rt.dockerPath ?? "");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  useEffect(() => { if (containerRuntime) void loadRuntime(); }, [containerRuntime]);
  const saveRuntime = (key: string, docker: string | null, terminal: string, done: string) =>
    act(key, async () => { await api.setContainerRuntime(docker, terminal); await loadRuntime(); }, done);

  const act = async (key: string, fn: () => Promise<unknown>, done?: string) => {
    setBusy(key);
    try {
      await fn();
      if (done) toast(done);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(null);
      reloadInfo();
    }
  };

  const install = info?.install;

  // The probe runs where Hydra has a protection to test: Seatbelt on macOS, Sandboxie on Windows.
  const protection = info?.platform === "macos" ? "Seatbelt" : info?.platform === "windows" ? "Sandboxie" : null;
  const [probeEnvs, setProbeEnvs] = useState<Environment[]>([]);
  const [probeEnv, setProbeEnv] = useState("");
  // Seatbelt comes with macOS; Sandboxie-Plus has to be installed.
  const [sandboxie, setSandboxie] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    if (protection !== "Sandboxie") return;
    api.sandboxStatus().then((dir) => setSandboxie({ ok: true, text: `已检测到 Sandboxie-Plus：${dir}` }), (e) => setSandboxie({ ok: false, text: errorText(e) }));
  }, [protection]);
  useEffect(() => {
    if (!protection) return;
    api.listEnvironments().then((list) => {
      const routed = list.filter((e) => e.node);
      setProbeEnvs(routed);
      setProbeEnv((current) => routed.some((e) => e.id === current) ? current : routed[0]?.id ?? "");
    }, () => setProbeEnvs([]));
  }, [protection]);

  return (
    <Page>
      <PageHeader title="设置" subtitle="外观、启动方式与 Claude 桌面端的位置。" />
      <div className="space-y-5 px-5">
        <Group label="外观">
          <Row label="主题" hint="跟随系统时会随系统的浅色 / 深色自动切换。">
            <div className="flex rounded-lg bg-pill p-0.5">
              {themes.map((t) => (
                <button
                  key={t.id}
                  onClick={() => onThemeChange(t.id)}
                  className={cn(
                    "h-6 rounded-md px-3 text-[12.5px] active:!scale-100",
                    theme === t.id ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg",
                  )}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </Row>
          <Row
            label="应用图标"
            hint={info?.platform === "macos"
              ? "立即用于窗口、菜单栏与 Hydra 运行时的程序坞；访达、启动台和未运行时的程序坞仍显示安装包内置的图标。"
              : "立即用于窗口、任务栏与托盘；资源管理器中的程序图标仍是安装包内置的图标。"}
          >
            <div role="radiogroup" aria-label="应用图标" className="flex gap-1.5">
              {info?.appIcons.map(([id, name]) => (
                <button
                  key={id}
                  role="radio"
                  aria-checked={info.appIcon === id}
                  aria-label={name}
                  title={name}
                  disabled={busy === "icon"}
                  onClick={() => act("icon", () => api.setAppIcon(id))}
                  className={cn(
                    "flex h-11 w-11 items-center justify-center rounded-[11px] active:!scale-95",
                    info.appIcon === id ? "bg-pill ring-2 ring-fg" : "hover:bg-pill",
                  )}
                >
                  <img src={appIconSrc(id)} alt="" className="h-8 w-8" draggable={false} />
                </button>
              ))}
            </div>
          </Row>
        </Group>

        <Group label="启动">
          <Row label="开机时启动到托盘" hint={info?.platform === "windows" ? "登录系统后在后台运行，托盘图标可一键切换账号。" : "目前仅 Windows 支持。"}>
            <Switch
              checked={!!info?.autostart}
              disabled={!info || info.platform !== "windows" || busy === "auto"}
              onCheckedChange={(v) => act("auto", () => api.setAutostart(v))}
            />
          </Row>
          <Row label="轻量模式" hint="关闭窗口时释放界面占用的内存，再次打开需要重新加载，会慢一些。关闭此项时，关闭窗口只是隐藏，再打开立即显示。托盘菜单里也可以切换。">
            <Switch
              checked={!!info?.lightweight}
              disabled={!info || busy === "light"}
              onCheckedChange={(v) => act("light", () => api.setLightweight(v))}
            />
          </Row>
          {info?.platform === "windows" && (
            <Row label="登录与链接路由" hint="注册后在 Windows 默认应用中把 Claude-Hydra 设为默认浏览器（以及 claude 链接）。之后 Claude 打开的谷歌登录等链接会进入该账号的专属浏览器，与账号同一出口 IP 和时区；其他链接仍交给你原来的浏览器。">
              <Button variant="outline" size="sm" disabled={busy === "router"} onClick={() => act("router", () => api.setupLoginRouter(), "已注册。请在系统设置中把 Claude-Hydra 设为默认浏览器。")}>
                设置
              </Button>
            </Row>
          )}
        </Group>

        <Group label="网络保护">
          <Row label="额外账号必须绑定代理环境" hint="开启后，没有绑定带代理节点的环境的额外账号会拒绝启动，账号专属浏览器也不会打开，避免误用本机网络直连。系统默认账号不受影响。">
            <Switch
              checked={!!info?.requireEnvironment}
              disabled={!info || busy === "require"}
              onCheckedChange={(v) => act("require", () => api.setRequireEnvironment(v))}
            />
          </Row>
        </Group>

        <Group label="Claude 桌面端">
          {info?.installError && <div className="px-4 py-3 text-[12.5px] text-danger">{info.installError}</div>}
          {install && (
            <div className="py-1">
              <KeyValue k="类型" v={install.kind === "Msix" ? "MSIX 版（官网安装包或 Microsoft Store）" : install.kind === "App" ? "Claude.app" : "安装程序版"} />
              {install.version && install.version !== "unknown" && <KeyValue k="版本" v={install.version} />}
              <KeyValue k="程序" v={install.exe ?? install.family} />
              <KeyValue k="默认数据目录" v={install.defaultProfile} />
            </div>
          )}
          <div className="flex gap-2 px-4 py-3">
            <Input
              placeholder={info?.platform === "macos" ? "/Applications/Claude.app（留空为自动查找）" : "C:\\…\\Claude.exe（留空为自动查找）"}
              value={path}
              onChange={(e) => setPath(e.target.value)}
              className="font-mono text-xs"
            />
            <Button
              variant="outline"
              disabled={busy === "path"}
              onClick={() =>
                act(
                  "path",
                  async () => {
                    await api.setClaudePath(path.trim() || null);
                    onInstallChanged();
                  },
                  path.trim() ? "已使用指定的路径。" : "已恢复自动查找。",
                )
              }
            >
              {busy === "path" && <Loader2 className="animate-spin" />}
              应用
            </Button>
          </div>
          {protection && (
            <>
              <Row label="账号页隐私提示" hint="进入「账号」页时，说明 Claude 桌面版有哪些地方 Hydra 无法保护。">
                <Switch
                  checked={!info?.hidePrivacyNotice}
                  disabled={!info || busy === "notice"}
                  onCheckedChange={(v) => act("notice", () => api.setPrivacyNoticeHidden(!v))}
                />
              </Row>
              <div className="px-4 py-3">
                <div className="text-[13px] font-medium">模拟测试</div>
                <div className="mt-0.5 text-[12px] leading-[18px] text-muted">
                  {"用一个 Electron 程序（与 Claude 桌面版同一类应用）按所选环境下账号的启动方式启动：同样的启动检查、中继、环境变量与启动开关，逐项检查每条网络路径实际从哪里出去，本机不需要安装 Claude。"}
                  {protection === "Seatbelt"
                    ? "「使用保护」用 macOS 的 Seatbelt 只放行中继；macOS 不允许沙箱嵌套，测试程序因此会关闭 Chromium 自身的沙箱，Claude 桌面版不能这样运行。"
                    : "「使用保护」在这个环境的 Sandboxie 沙箱中运行（需要安装 Sandboxie-Plus），与开启「沙箱隔离」的账号相同。"}
                  {"首次使用会从 Electron 官方发布页下载约 100 MB。"}
                </div>
                {probeEnvs.length ? (
                  <div className="mt-2.5 flex flex-wrap items-center gap-2">
                    <Select aria-label="测试用的环境" className="w-48" value={probeEnv} onChange={(e) => setProbeEnv(e.target.value)}>
                      {probeEnvs.map((e) => <option key={e.id} value={e.id}>{e.name}</option>)}
                    </Select>
                    <Button variant="outline" size="sm" disabled={!probeEnv || !!busy} onClick={() => act("probe", () => api.launchProbe(probeEnv, false), "已打开测试程序（不使用保护）。")}>
                      {busy === "probe" && <Loader2 className="animate-spin" />}
                      不使用保护
                    </Button>
                    <Button variant="outline" size="sm" disabled={!probeEnv || !!busy || (protection === "Sandboxie" && !sandboxie?.ok)} onClick={() => act("probe-protected", () => api.launchProbe(probeEnv, true), `已打开测试程序（使用 ${protection} 保护）。`)}>
                      {busy === "probe-protected" && <Loader2 className="animate-spin" />}
                      使用保护（{protection}）
                    </Button>
                  </div>
                ) : (
                  <div className="mt-2 text-[12px] text-muted">还没有带代理节点的环境，请先在「环境配置」中新建一个。</div>
                )}
                {sandboxie && <div className={cn("mt-2 text-[12px]", sandboxie.ok ? "text-muted" : "text-warn")}>{sandboxie.text}</div>}
              </div>
            </>
          )}
        </Group>

        {containerRuntime && (
          <Group label={`${containerRuntime} 容器`}>
            <Row
              label="连接状态"
              hint={runtime?.version ? `已连接 ${containerRuntime}（docker ${runtime.version}）。` : runtime?.error ?? "正在检测…"}
            >
              <Button variant="outline" size="sm" disabled={busy === "probe"} onClick={() => act("probe", loadRuntime)}>
                {busy === "probe" && <Loader2 className="animate-spin" />}
                重新检测
              </Button>
            </Row>
            {(runtime?.terminals.length ?? 0) > 1 && <Row label="打开容器使用的终端" hint={runtime?.terminal === "" ? "容器账号在此终端的新窗口中打开。终端（Terminal.app）不支持 Claude Code 在容器里的复制：选中文字时按住 Fn，再用 ⌘C 复制；iTerm2 和 Ghostty 没有这个限制。" : "容器账号在此终端的新窗口中打开。"}>
              <Select
                aria-label="终端"
                className="w-48"
                value={runtime?.terminal ?? ""}
                disabled={!runtime || busy === "terminal"}
                onChange={(e) => saveRuntime("terminal", runtime?.dockerPath ?? null, e.target.value, "已保存。")}
              >
                {runtime?.terminals.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
              </Select>
            </Row>}
            <div className="flex gap-2 px-4 py-3">
              <Input
                placeholder={info?.platform === "windows" ? "docker.exe 路径（留空为自动查找 Docker Desktop 的 docker）" : "docker 命令路径（留空为自动查找 ~/.orbstack/bin/docker 等）"}
                value={dockerPath}
                onChange={(e) => setDockerPath(e.target.value)}
                className="font-mono text-xs"
              />
              <Button
                variant="outline"
                disabled={!runtime || busy === "docker"}
                onClick={() => saveRuntime("docker", dockerPath.trim() || null, runtime?.terminal ?? "", dockerPath.trim() ? "已使用指定的 docker。" : "已恢复自动查找。")}
              >
                {busy === "docker" && <Loader2 className="animate-spin" />}
                应用
              </Button>
            </div>
          </Group>
        )}

        <Group label="数据与诊断">
          <div className="py-1">
            <KeyValue k="根目录" v={info?.storageRoot} />
            <KeyValue k="设置文件" v={info?.settingsPath} />
          </div>
          <Row label="诊断信息" hint="包含版本、路径与各账号运行状态，系统用户名已自动脱敏。">
            <Button
              variant="outline"
              size="sm"
              disabled={busy === "doctor"}
              onClick={() =>
                act(
                  "doctor",
                  async () => {
                    const text = await api.diagnostics();
                    await navigator.clipboard.writeText(text);
                  },
                  "诊断信息已复制到剪贴板。",
                )
              }
            >
              {busy === "doctor" ? <Loader2 className="animate-spin" /> : <Copy />}
              复制
            </Button>
          </Row>
        </Group>
      </div>
    </Page>
  );
}
