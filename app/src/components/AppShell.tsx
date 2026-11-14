import { UpdateNotice } from "./UpdateNotice";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Box, FolderCog, Globe2, LayoutDashboard, Info, Minus, Moon, Settings, Square, Sun, SunMoon, Users, X } from "lucide-react";
import type React from "react";
import { useLayoutEffect, useRef, useState } from "react";
import { appIconSrc } from "@/lib/appIcon";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";

export type PageId = "dashboard" | "accounts" | "environments" | "containers" | "config" | "settings" | "about";

type Icon = React.ComponentType<{ className?: string }>;
interface NavItem { id: PageId; label: string; icon: Icon }

const isMac = navigator.userAgent.includes("Mac");
/** Containers run on OrbStack (macOS) or Docker Desktop (Windows). */
export const hasContainers = isMac || navigator.userAgent.includes("Windows");
const main: NavItem[] = [
  { id: "dashboard", label: "仪表板", icon: LayoutDashboard },
  { id: "accounts", label: "账号", icon: Users },
  { id: "environments", label: "环境配置", icon: Globe2 },
  ...(hasContainers ? [{ id: "containers" as const, label: "容器", icon: Box }] : []),
  { id: "config", label: "配置管理", icon: FolderCog },
];
const footer: NavItem[] = [
  { id: "settings", label: "设置", icon: Settings },
  { id: "about", label: "关于", icon: Info },
];
const titles: Record<PageId, string> = {
  dashboard: "仪表板", accounts: "账号", environments: "环境配置", containers: "容器", config: "配置管理", settings: "设置", about: "关于",
};

const nextTheme: Record<ThemeMode, ThemeMode> = { system: "light", light: "dark", dark: "system" };
const themeLabel: Record<ThemeMode, string> = { system: "跟随系统", light: "浅色", dark: "深色" };

interface AppShellProps {
  activePage: PageId;
  onPageChange: (page: PageId) => void;
  theme: ThemeMode;
  onThemeChange: (mode: ThemeMode) => void;
  /** Chosen app icon id. */
  appIcon?: string | null;
  /** Running summary shown at the bottom of the sidebar. */
  status?: { running: number; total: number } | null;
  /**
   * The account list could not be read, typically because Claude.app is not installed (a
   * container-only setup). The summary line is then left out; the accounts page shows why.
   */
  statusError?: string | null;
  /** The page draws its own top bar (with drag region and window controls), flush with the top. */
  bare?: boolean;
  children: React.ReactNode;
}

/** Minimise, maximise and close for the frameless window off macOS (macOS has its traffic lights). */
export function WindowControls() {
  const win = getCurrentWindow();
  const base = "flex h-12 w-11 items-center justify-center text-muted hover:text-fg";
  return (
    <div className="-mr-3 ml-1 flex">
      <button className={cn(base, "hover:bg-pill")} aria-label="最小化" onClick={() => win.minimize()}><Minus className="h-4 w-4" /></button>
      <button className={cn(base, "hover:bg-pill")} aria-label="最大化" onClick={() => win.toggleMaximize()}><Square className="h-3 w-3" /></button>
      <button className={cn(base, "hover:bg-[#e81123] hover:text-white")} aria-label="关闭到托盘" title="关闭窗口（Hydra 留在托盘）" onClick={() => win.close()}><X className="h-4 w-4" /></button>
    </div>
  );
}

function NavRow({ item, active, onSelect, setRef }: { item: NavItem; active: boolean; onSelect: () => void; setRef: (el: HTMLButtonElement | null) => void }) {
  const Icon = item.icon;
  return (
    <button
      ref={setRef}
      onClick={onSelect}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative z-10 flex h-[30px] w-full items-center gap-[9px] rounded-[7px] px-2 text-left text-[13px] active:!scale-100",
        active ? "font-medium text-fg" : "text-fg-2 hover:bg-pill hover:text-fg",
      )}
    >
      <Icon className={cn("h-[15px] w-[15px] shrink-0", active ? "text-fg" : "text-muted")} />
      {item.label}
    </button>
  );
}

export function AppShell({ activePage, onPageChange, theme, onThemeChange, appIcon, status, statusError, bare, children }: AppShellProps) {
  const ThemeIcon = theme === "light" ? Sun : theme === "dark" ? Moon : SunMoon;
  const refs = useRef<Partial<Record<PageId, HTMLButtonElement | null>>>({});
  const nav = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<number | null>(null);

  // Sliding highlight behind the active row.
  useLayoutEffect(() => {
    const measure = () => {
      const el = refs.current[activePage];
      const box = nav.current;
      if (el && box) setThumb(el.getBoundingClientRect().top - box.getBoundingClientRect().top);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [activePage]);

  const row = (item: NavItem) => (
    <NavRow key={item.id} item={item} active={activePage === item.id} onSelect={() => onPageChange(item.id)} setRef={(el) => (refs.current[item.id] = el)} />
  );

  return (
    <div className="flex h-screen overflow-hidden bg-bg text-fg">
      <aside className="flex w-[204px] shrink-0 flex-col border-r-[0.5px] border-line bg-bg">
        <div data-tauri-drag-region className={cn("flex h-12 shrink-0 items-center gap-2", isMac ? "pl-[78px]" : "pl-4")}>
          <img src={appIconSrc(appIcon)} alt="" className="pointer-events-none h-[18px] w-[18px] rounded-[5px]" draggable={false} />
          <span className="pointer-events-none truncate text-[13.5px] font-semibold tracking-[0.01em]">Claude-Hydra</span>
        </div>

        <div ref={nav} className="relative flex min-h-0 flex-1 flex-col px-2.5">
          {thumb !== null && (
            <span
              aria-hidden
              className="absolute inset-x-2.5 h-[30px] rounded-[7px] bg-card shadow-edge transition-[top] duration-300 ease-soft dark:bg-white/[.07]"
              style={{ top: thumb }}
            />
          )}
          <div className="flex-1 space-y-0.5 overflow-y-auto pb-2 pt-1">{main.map(row)}</div>
          <div className="space-y-0.5 border-t-[0.5px] border-line-2 py-2">{footer.map(row)}</div>
        </div>

        {(status || !statusError) && (
          <div className="flex h-9 shrink-0 items-center gap-2 px-[18px] pb-1 text-[11.5px] text-muted">
            <span className={cn("h-[7px] w-[7px] rounded-full", status?.running ? "bg-ok shadow-[0_0_0_3px_rgb(var(--ok-soft))]" : "bg-faint")} />
            {status ? `${status.running} 个运行中 · 共 ${status.total} 个账号` : "读取中…"}
          </div>
        )}
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region className={cn("flex h-12 shrink-0 items-center justify-between gap-3 pl-5 pr-3", bare && "hidden")}>
          <div data-tauri-drag-region className="pointer-events-none min-w-0 truncate text-[13.5px] font-semibold">{titles[activePage]}</div>
          <div className="flex items-center gap-1">
            <UpdateNotice />
            <button
              onClick={() => onThemeChange(nextTheme[theme])}
              title={`主题：${themeLabel[theme]}（点击切换）`}
              aria-label={`主题：${themeLabel[theme]}`}
              className="flex h-[26px] w-[26px] items-center justify-center rounded-[7px] text-muted hover:bg-pill hover:text-fg"
            >
              <ThemeIcon className="h-[15px] w-[15px]" />
            </button>
            {!isMac && <WindowControls />}
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto">{children}</main>
      </div>
    </div>
  );
}
