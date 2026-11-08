import { invoke } from "@tauri-apps/api/core";

export interface Profile {
  badge: string;
  id: string;
  name: string;
  path: string;
  isDefault: boolean;
  pid: number | null;
  lastUsed: number | null;
  color: number;
  keepRunning: boolean;
  signedIn: boolean | null;
  /** Name of the assigned environment config. */
  environment: string | null;
}

export interface ProfileOptions {
  name: string;
  color: number;
  keepRunning: boolean;
  /** Assigned environment id; "" = none. */
  environment: string;
  lastExit: ExitInfo | null;
}

export interface EnvironmentInput {
  name: string;
  node: NodeConfig | null;
  /** "proxy" | "system" | IANA name */
  timezone: string;
  locale: string;
  args: string;
  /** One NAME=value per line. */
  env: string;
}
export interface Environment extends EnvironmentInput { id: string; usedBy: string[] }

export type NodeKind = "http" | "https" | "socks5";
export interface NodeConfig {
  kind: NodeKind; host: string; port: number; username: string; password: string;
  ipVersion: "ipv4" | "ipv6"; geoChannel: "ipinfo" | "ipapi" | "ip-api"; refreshUrl: string;
}
export interface ExitInfo { ip: string; country: string; region: string; city: string; timezone: string; checkedAt: number }

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

export interface QuotaWindow { utilization: number; resetsAt: string | null }
export interface Quota { fiveHour: QuotaWindow | null; sevenDay: QuotaWindow | null; fetchedAt: number }

export interface CodeSession {
  id: string; title: string; cwd: string | null; cwdExists: boolean; lastActivityAt: number | null; inTarget: boolean;
}
export interface SessionPreview { sessions: CodeSession[]; targetProblem: string | null; running: boolean }
export interface TransferResult { copied: number; skipped: number; failed: number }

export const api = {
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  launch: (id: string) => invoke<string>("launch_profile", { id }),
  close: (id: string) => invoke<string>("close_profile", { id }),
  restart: (id: string) => invoke<string>("restart_profile", { id }),
  addAccount: () => invoke<Profile>("add_account"),
  nextAccountName: () => invoke<string>("next_account_name"),
  createConfiguredAccount: (name: string, environment: string) => invoke<Profile>("create_configured_account", { name, environment }),
  listEnvironments: () => invoke<Environment[]>("list_environments"),
  saveEnvironment: (id: string | null, input: EnvironmentInput) => invoke<string>("save_environment", { id, input }),
  deleteEnvironment: (id: string) => invoke<void>("delete_environment", { id }),
  options: (id: string) => invoke<ProfileOptions>("profile_options", { id }),
  saveOptions: (id: string, options: ProfileOptions) => invoke<void>("save_profile_options", { id, options }),
  remove: (id: string) => invoke<void>("delete_profile", { id }),
  openFolder: (id: string) => invoke<void>("open_profile_folder", { id }),
  createShortcut: (id: string) => invoke<string>("create_shortcut", { id }),
  setupLoginRouter: () => invoke<void>("setup_login_router"),
  prepareLogin: (id: string) => invoke<void>("prepare_login", { id }),
  diagnostics: () => invoke<string>("diagnostics"),
  saveTranscript: (text: string) => invoke<string>("save_transcript", { text }),
  revealTranscript: (path: string) => invoke<void>("reveal_transcript", { path }),
  checkNode: (node: NodeConfig) => invoke<[ExitInfo, string | null, string]>("check_node", { node }),
  refreshNodeIp: (url: string) => invoke<string>("refresh_node_ip", { url }),
  profileQuota: (id: string) => invoke<Quota>("profile_quota", { id }),
  sessionPreview: (source: string, target: string) => invoke<SessionPreview>("session_preview", { source, target }),
  copySessions: (source: string, target: string, ids: string[]) => invoke<TransferResult>("copy_sessions", { source, target, ids }),
  appInfo: () => invoke<AppInfo>("app_info"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setClaudePath: (path: string | null) => invoke<void>("set_claude_path", { path }),
  quit: () => invoke<void>("quit_app"),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
