import { Copy, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Group, Page, PageHeader, Row } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AppInfo, type ContainerRuntime } from "@/lib/api";
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
  useEffect(() => { if (info?.platform === "macos") void loadRuntime(); }, [info?.platform]);
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
        </Group>

        <Group label="启动">
          <Row label="开机时启动到托盘" hint={info?.platform === "windows" ? "登录系统后在后台运行，托盘图标可一键切换账号。" : "目前仅 Windows 支持。"}>
            <Switch
              checked={!!info?.autostart}
              disabled={!info || info.platform !== "windows" || busy === "auto"}
              onCheckedChange={(v) => act("auto", () => api.setAutostart(v))}
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
              <KeyValue k="类型" v={install.kind === "Msix" ? "Microsoft Store 版" : install.kind === "App" ? "Claude.app" : "安装程序版"} />
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
        </Group>

        {info?.platform === "macos" && (
          <Group label="OrbStack 容器">
            <Row
              label="连接状态"
              hint={runtime?.version ? `已连接 OrbStack（docker ${runtime.version}，上下文 orbstack）。` : runtime?.error ?? "正在检测…"}
            >
              <Button variant="outline" size="sm" disabled={busy === "probe"} onClick={() => act("probe", loadRuntime)}>
                {busy === "probe" && <Loader2 className="animate-spin" />}
                重新检测
              </Button>
            </Row>
            <Row label="打开容器使用的终端" hint="容器账号在此终端的新窗口中打开。">
              <Select
                aria-label="终端"
                className="w-48"
                value={runtime?.terminal ?? ""}
                disabled={!runtime || busy === "terminal"}
                onChange={(e) => saveRuntime("terminal", runtime?.dockerPath ?? null, e.target.value, "已保存。")}
              >
                {runtime?.terminals.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
              </Select>
            </Row>
            <div className="flex gap-2 px-4 py-3">
              <Input
                placeholder="docker 命令路径（留空为自动查找 ~/.orbstack/bin/docker 等）"
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

        <Group label="用量">
          <Row
            label="后台记录剩余额度"
            hint={`手动刷新过一次额度的账号，Hydra 运行期间每 10 分钟经该账号的代理节点再读一次，用于额度趋势。${info?.platform === "macos" ? "macOS 桌面账号读取凭据需要钥匙串授权，只在手动刷新时记录。" : ""}`}
          >
            <Switch
              checked={!!info?.quotaSampling}
              disabled={!info || busy === "sampling"}
              onCheckedChange={(v) => act("sampling", () => api.setQuotaSampling(v))}
            />
          </Row>
        </Group>

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
