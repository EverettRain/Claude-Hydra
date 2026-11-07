import { getCurrentWindow } from "@tauri-apps/api/window";
import { CircleHelp, Minus, Moon, Square, Sun, SunMoon, Users, X, SlidersHorizontal } from "lucide-react";
import type React from "react";
import appIcon from "../assets/claude-hydra-icon-256.png";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";

export type PageId = "accounts" | "settings" | "about";

const navItems: Array<{ id: PageId; label: string; icon: React.ComponentType<{ className?: string }> }> = [
  { id: "accounts", label: "账号", icon: Users },
  { id: "settings", label: "设置", icon: SlidersHorizontal },
  { id: "about", label: "关于", icon: CircleHelp },
];

const isMac = navigator.userAgent.includes("Mac");

const nextTheme: Record<ThemeMode, ThemeMode> = { system: "light", light: "dark", dark: "system" };
const themeLabel: Record<ThemeMode, string> = { system: "跟随系统", light: "浅色", dark: "深色" };

interface AppShellProps {
  activePage: PageId;
  onPageChange: (page: PageId) => void;
  theme: ThemeMode;
  onThemeChange: (mode: ThemeMode) => void;
  children: React.ReactNode;
}

function WindowControls() {
  const win = getCurrentWindow();
  const base = "flex h-8 w-11 items-center justify-center text-muted-foreground hover:text-foreground";
  return (
    <div className="flex">
      <button className={cn(base, "hover:bg-gray-100 dark:hover:bg-gray-800")} aria-label="最小化" onClick={() => win.minimize()}>
        <Minus className="h-4 w-4" />
      </button>
      <button className={cn(base, "hover:bg-gray-100 dark:hover:bg-gray-800")} aria-label="最大化" onClick={() => win.toggleMaximize()}>
        <Square className="h-3 w-3" />
      </button>
      <button
        className={cn(base, "hover:bg-red-500 hover:text-white")}
        aria-label="关闭到托盘"
        title="关闭窗口（Hydra 留在托盘）"
        onClick={() => win.close()}
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}

export function AppShell({ activePage, onPageChange, theme, onThemeChange, children }: AppShellProps) {
  const ThemeIcon = theme === "light" ? Sun : theme === "dark" ? Moon : SunMoon;
  return (
    <div className="relative flex h-screen overflow-hidden bg-background text-foreground">
      <div className="absolute inset-x-0 top-0 z-30 flex h-8">
        <div data-tauri-drag-region className="w-24 shrink-0 border-r border-border bg-[hsl(var(--sidebar))]" />
        <div data-tauri-drag-region className="min-w-0 flex-1 bg-background" />
        {!isMac && <WindowControls />}
      </div>

      <aside className="flex w-24 shrink-0 flex-col border-r border-border bg-[hsl(var(--sidebar))] pt-8">
        <div className="flex h-[64px] items-center justify-center">
          <img src={appIcon} alt="Claude-Hydra" className="h-14 w-14 rounded-xl" draggable={false} />
        </div>

        <nav className="flex flex-1 flex-col items-center gap-3 px-2.5 py-4">
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = activePage === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onPageChange(item.id)}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "flex h-16 w-full flex-col items-center justify-center rounded-lg px-1 py-2.5 text-center",
                  active
                    ? "bg-blue-600 text-white hover:bg-blue-700"
                    : "text-muted-foreground hover:bg-gray-100 hover:text-foreground dark:hover:bg-gray-800",
                )}
              >
                <Icon className="h-5 w-5 shrink-0" />
                <span className="mt-1 text-[14px] font-medium leading-[15px]">{item.label}</span>
              </button>
            );
          })}
        </nav>

        <div className="px-3 pb-3">
          <button
            onClick={() => onThemeChange(nextTheme[theme])}
            title={`主题：${themeLabel[theme]}（点击切换）`}
            aria-label={`主题：${themeLabel[theme]}`}
            className="flex w-full flex-col items-center justify-center gap-1 rounded-lg px-1 py-2 text-muted-foreground hover:bg-gray-100 hover:text-foreground dark:hover:bg-gray-800"
          >
            <ThemeIcon className="h-5 w-5" />
            <span className="text-[11px] leading-none">{themeLabel[theme]}</span>
          </button>
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto bg-background pt-8">{children}</main>
    </div>
  );
}
