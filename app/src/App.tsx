import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { AppShell, type PageId } from "@/components/AppShell";
import { api, errorText, type AppInfo, type Profile } from "@/lib/api";
import { useTheme } from "@/lib/theme";
import { AboutPage } from "@/pages/AboutPage";
import { AccountsPage } from "@/pages/AccountsPage";
import { SessionsPage } from "@/pages/SessionsPage";
import { UsagePage } from "@/pages/UsagePage";
import { SettingsPage } from "@/pages/SettingsPage";

export default function App() {
  const [page, setPage] = useState<PageId>("accounts");
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
    <AppShell activePage={page} onPageChange={setPage} theme={theme} onThemeChange={setTheme}>
      {page === "accounts" && <AccountsPage profiles={profiles} loadError={loadError} refresh={refresh} />}
      {page === "usage" && <UsagePage profiles={profiles} />}
      {page === "sessions" && <SessionsPage profiles={profiles} />}
      {page === "settings" && (
        <SettingsPage info={info} reloadInfo={reloadInfo} theme={theme} onThemeChange={setTheme} onInstallChanged={refresh} />
      )}
      {page === "about" && <AboutPage info={info} />}
    </AppShell>
  );
}
