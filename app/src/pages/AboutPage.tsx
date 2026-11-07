import { Cpu, Feather, Layers } from "lucide-react";
import appIcon from "../assets/claude-hydra-icon-256.png";
import { PageHeader } from "@/components/PageHeader";
import type { AppInfo } from "@/lib/api";

const points = [
  {
    icon: Layers,
    title: "账号互不干扰",
    text: "每个账号使用独立的 Claude 数据目录（--user-data-dir），登录、Cookie 和设置彼此隔离，可以同时打开。",
  },
  {
    icon: Feather,
    title: "托盘里几乎不占资源",
    text: "关闭窗口即销毁 WebView 界面，仅保留轻量的 Rust 后台进程驻留系统托盘并实时感知进程状态。",
  },
  {
    icon: Cpu,
    title: "快捷方式秒开",
    text: "桌面快捷方式通过 --launch 直接启动或唤醒目标账号窗口，不加载任何界面，只需几毫秒。",
  },
];

export function AboutPage({ info }: { info: AppInfo | null }) {
  return (
    <div className="pb-8">
      <PageHeader title="关于" />
      <div className="space-y-4 px-6">
        <div className="flex items-center gap-5 rounded-lg border border-border bg-card p-5">
          <img src={appIcon} alt="" className="h-20 w-20 rounded-2xl" draggable={false} />
          <div>
            <div className="text-2xl font-semibold">Claude-Hydra</div>
            <div className="mt-1 text-sm text-muted-foreground">同时运行多个 Claude 桌面端账号</div>
            <div className="mt-2 inline-flex rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
              v{info?.version ?? "…"} · {info?.platform ?? ""}
            </div>
          </div>
        </div>
        <div className="grid gap-3 md:grid-cols-3">
          {points.map((p) => (
            <div key={p.title} className="rounded-lg border border-border bg-card p-4">
              <p.icon className="h-5 w-5 text-blue-500" />
              <div className="mt-2 text-sm font-semibold">{p.title}</div>
              <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{p.text}</p>
            </div>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          命令行：<span className="selectable font-mono">claude-hydra --launch &lt;账号&gt; | --close | --restart | --list | --add-account | --tray</span>
        </p>
      </div>
    </div>
  );
}
