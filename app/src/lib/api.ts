import { invoke } from "@tauri-apps/api/core";

export interface Profile {
  id: string;
  name: string;
  path: string;
  isDefault: boolean;
  pid: number | null;
  lastUsed: number | null;
  color: number;
  keepRunning: boolean;
  signedIn: boolean | null;
}

export interface ProfileOptions {
  name: string;
  color: number;
  env: string;
  args: string;
  keepRunning: boolean;
}

export interface Install {
  kind: string;
  exe: string | null;
  version: string | null;
  family: string | null;
  aumid: string | null;
  defaultProfile: string;
}

export interface AppInfo {
  version: string;
  platform: string;
  install: Install | null;
  installError: string | null;
  claudePath: string | null;
  storageRoot: string;
  settingsPath: string;
  autostart: boolean;
}

export interface LocalUsage {
  inputTokens: number; outputTokens: number; cacheCreationTokens: number; cacheReadTokens: number;
  messages: number; files: number; unreadableFiles: number; path: string;
}

export const api = {
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  launch: (id: string) => invoke<string>("launch_profile", { id }),
  close: (id: string) => invoke<string>("close_profile", { id }),
  restart: (id: string) => invoke<string>("restart_profile", { id }),
  addAccount: () => invoke<Profile>("add_account"),
  options: (id: string) => invoke<ProfileOptions>("profile_options", { id }),
  saveOptions: (id: string, options: ProfileOptions) => invoke<void>("save_profile_options", { id, options }),
  remove: (id: string) => invoke<void>("delete_profile", { id }),
  openFolder: (id: string) => invoke<void>("open_profile_folder", { id }),
  createShortcut: (id: string) => invoke<string>("create_shortcut", { id }),
  setupLoginRouter: () => invoke<void>("setup_login_router"),
  prepareLogin: (id: string) => invoke<void>("prepare_login", { id }),
  diagnostics: () => invoke<string>("diagnostics"),
  localUsage: () => invoke<LocalUsage>("local_usage"),
  appInfo: () => invoke<AppInfo>("app_info"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setClaudePath: (path: string | null) => invoke<void>("set_claude_path", { path }),
  quit: () => invoke<void>("quit_app"),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
