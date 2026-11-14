import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { AppShell, type PageId } from "@/components/AppShell";
import { api, errorText, type AppInfo, type Profile } from "@/lib/api";
import { useTheme } from "@/lib/theme";
import { AboutPage } from "@/pages/AboutPage";
import { AccountsPage } from "@/pages/AccountsPage";
import { ConfigPage } from "@/pages/ConfigPage";
import { ContainersPage } from "@/pages/ContainersPage";
import { EnvironmentsPage } from "@/pages/EnvironmentsPage";
import { DashboardPage } from "@/pages/DashboardPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { WebPage } from "@/pages/WebPage";

export default function App() {
  const [page, setPage] = useState<PageId>("dashboard");
  // A session opened in 配置管理 fills the window up to the top edge.
  const [immersive, setImmersive] = useState(false);
  const [theme, setTheme] = useTheme();
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);

  const refresh = useCallback(async () => {
    try {
      setProfiles(await api.listProfiles());
      setLoadError(null);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }, []);

  const reloadInfo = useCallback(async () => {
    try {
      setInfo(await api.appInfo());
    } catch {
      /* shown as missing values */
    }
  }, []);

  useEffect(() => {
    refresh();
    reloadInfo();
    const un = listen<Profile[]>("profiles-changed", (e) => setProfiles(e.payload));
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      un.then((f) => f());
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh, reloadInfo]);

  return (
    <AppShell activePage={page} onPageChange={setPage} theme={theme} onThemeChange={setTheme} appIcon={info?.appIcon}
      status={profiles ? { running: profiles.filter((p) => p.pid).length, total: profiles.length } : null} statusError={loadError} bare={page === "config" && immersive}>
      {page === "accounts" && <AccountsPage profiles={profiles} loadError={loadError} refresh={refresh} />}
      {page === "environments" && <EnvironmentsPage onChanged={refresh} />}
      {page === "containers" && <ContainersPage onOpenSettings={() => setPage("settings")} />}
      {page === "web" && <WebPage profiles={profiles} />}
      {page === "dashboard" && <DashboardPage />}
      {page === "config" && <ConfigPage profiles={profiles} onImmersive={setImmersive} />}
      {page === "settings" && (
        <SettingsPage info={info} reloadInfo={reloadInfo} theme={theme} onThemeChange={setTheme} onInstallChanged={refresh} />
      )}
      {page === "about" && <AboutPage info={info} />}
    </AppShell>
  );
}
