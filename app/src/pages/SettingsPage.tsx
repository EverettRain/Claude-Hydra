import { Copy, Loader2 } from "lucide-react";
import type React from "react";
import { useEffect, useState } from "react";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AppInfo } from "@/lib/api";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";

interface Props {
  info: AppInfo | null;
  reloadInfo: () => Promise<void>;
  theme: ThemeMode;
  onThemeChange: (m: ThemeMode) => void;
  onInstallChanged: () => void;
}

function Section({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-border bg-card p-4">
      <h2 className="text-base font-semibold">{title}</h2>
      {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
      <div className="mt-3 space-y-3">{children}</div>
    </section>
  );
}

function Row({ label, hint, children }: { label: string; hint?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div className="min-w-0">
        <div className="text-sm font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function KeyValue({ k, v }: { k: string; v?: string | null }) {
  return (
    <div className="flex gap-3 text-xs">
      <span className="w-24 shrink-0 text-muted-foreground">{k}</span>
      <span className="selectable min-w-0 break-all font-mono">{v || "—"}</span>
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
    <div className="pb-8">
      <PageHeader title="设置" />
      <div className="space-y-4 px-6">
        <Section title="外观">
          <Row label="主题">
            <div className="flex rounded-lg bg-muted p-0.5">
              {themes.map((t) => (
                <button
                  key={t.id}
                  onClick={() => onThemeChange(t.id)}
                  className={cn(
                    "rounded-md px-3 py-1 text-sm",
                    theme === t.id ? "bg-card text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {t.label}
                </button>
              ))}
            </div>
          </Row>
        </Section>

        <Section title="启动">
          <Row label="开机时启动到托盘" hint="登录系统后在后台运行，托盘图标可一键切换账号。关闭窗口后仅保留极小的后台进程。">
            <Switch
              checked={!!info?.autostart}
              disabled={!info || info.platform !== "windows" || busy === "auto"}
              onCheckedChange={(v) => act("auto", () => api.setAutostart(v))}
            />
          </Row>
        </Section>

        <Section title="Claude 桌面端" description="自动查找本机安装的 Claude；若安装在非默认位置可手动指定。">
          {info?.installError && <p className="text-sm text-red-500">{info.installError}</p>}
          {install && (
            <div className="space-y-1.5 rounded-md bg-muted/60 p-3">
              <KeyValue k="类型" v={install.kind === "Msix" ? "Microsoft Store 版" : install.kind === "App" ? "Claude.app" : "安装程序版"} />
              {install.version && install.version !== "unknown" && <KeyValue k="版本" v={install.version} />}
              <KeyValue k="程序" v={install.exe ?? install.family} />
              <KeyValue k="默认数据目录" v={install.defaultProfile} />
            </div>
          )}
          <div className="flex gap-2">
            <Input
              placeholder={info?.platform === "macos" ? "/Applications/Claude.app" : "C:\\…\\Claude.exe（留空为自动查找）"}
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
              {busy === "path" && <Loader2 className="h-4 w-4 animate-spin" />}
              应用
            </Button>
          </div>
        </Section>

        <Section title="数据与诊断" description="所有额外账号与配置文件均独立保存在以下目录。">
          <div className="space-y-1.5">
            <KeyValue k="根目录" v={info?.storageRoot} />
            <KeyValue k="设置文件" v={info?.settingsPath} />
          </div>
          <Row label="诊断信息" hint="包含版本、路径与各账号运行状态（系统用户名已自动脱敏）。">
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
              {busy === "doctor" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Copy className="h-3.5 w-3.5" />}
              复制诊断信息
            </Button>
          </Row>
        </Section>
      </div>
    </div>
  );
}
