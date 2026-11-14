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
  /** Why the exit guard cut this account's network, while it is cut. */
  blocked: string | null;
  /** The cut is temporary: checks kept failing and Hydra keeps re-checking by itself. */
  blockedSoft: boolean;
}

export interface ProfileOptions {
  name: string;
  color: number;
  keepRunning: boolean;
  /** Assigned environment id; "" = none. */
  environment: string;
  lastExit: ExitInfo | null;
}

export interface ClashSubscription { url: string; nodes: NodeConfig[] }

export interface EnvironmentInput {
  clashSubscription?: ClashSubscription | null;
  name: string;
  node: NodeConfig | null;
  /** "proxy" | "system" | IANA name */
  timezone: string;
  locale: string;
  args: string;
  /** One NAME=value per line. */
  env: string;
  /** Last successful connectivity check of the node. */
  lastCheck: NodeCheck | null;
  /** "" | "static" | "city" | "region" | "country" */
  exitRule: ExitRule;
  /** Allow mainland-China exits (blocked by default). */
  allowCn: boolean;
  /** "" | "sandboxie" */
  isolation: "" | "sandboxie";
  /** Each account gets its own Claude Code login and configuration directory. */
  separateCode: boolean;
}
export type ExitRule = "" | "static" | "city" | "region" | "country";
export interface Environment extends EnvironmentInput {
  id: string;
  usedBy: string[];
  /** Exit timezone last confirmed for the node (connectivity or launch check). */
  detectedTimezone: string | null;
  /** Exit the rule is bound to (set by the first verified check). */
  exitBaseline: ExitInfo | null;
}

export type NodeKind = "clash" | "http" | "https" | "socks5";
export interface NodeConfig {
  kind: NodeKind; host: string; port: number; username: string; password: string;
  ipVersion: "ipv4" | "ipv6"; geoChannel: "ipinfo" | "ipapi" | "ip-api"; refreshUrl: string;
  /** Front (transit) proxy this node is reached through; may itself have one. */
  via?: NodeConfig | null;
}
export interface NodeCheck { exit: ExitInfo; quality: string | null; risk: string }
/** A published release and its markdown notes. */
export interface ReleaseNote { version: string; date: string; body: string }
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
  /** Extra accounts refuse to start without a proxy environment. */
  requireEnvironment: boolean;
  /** Closing the window destroys it (frees memory) instead of hiding it. */
  lightweight: boolean;
  /** Quota is re-read in the background for the trend. */
  quotaSampling: boolean;
  /** Chosen app icon id. */
  appIcon: string;
  /** Selectable icons: id, display name. */
  appIcons: [string, string][];
}

/** A usage window as the service names it: "five_hour", "seven_day", "seven_day_opus", … */
export interface QuotaWindow { id: string; utilization: number; resetsAt: string | null }
export interface Quota { windows: QuotaWindow[]; fetchedAt: number }
/** Consecutive unchanged readings of one cycle (times in ms). */
export interface QuotaRun { utilization: number; resetsAt: number | null; from: number; to: number; samples: number }
export interface QuotaHistory { windows: Record<string, QuotaRun[]> }

export interface CodeSession {
  id: string; title: string; cwd: string | null; cwdExists: boolean; lastActivityAt: number | null; model: string | null; turns: number | null; inTarget: boolean;
}
export interface SessionPreview { sessions: CodeSession[]; targetProblem: string | null; running: boolean }
export interface TransferResult { copied: number; skipped: number; failed: number }

/** A Claude Code data folder (`CLAUDE_CONFIG_DIR`). */
export interface ClaudeFolder {
  /** `local`, `desktop:<account>` or `container:<account>`. */
  id: string;
  kind: "local" | "desktop" | "container";
  name: string;
  path: string;
  /** A desktop account without a Claude Code folder of its own uses the Mac's `~/.claude`. */
  sharedWithLocal: boolean;
  /** Desktop account the folder belongs to. */
  profile: string | null;
}
/** A location as the picker shows it: session count and latest activity come from file names and times. */
export interface ClaudeFolderSummary extends ClaudeFolder { exists: boolean; sessions: number; lastActiveAt: number | null }
export interface ClaudeCodeSession {
  /** `<project folder>/<session id>`. */
  id: string; title: string; cwd: string | null; lastActivityAt: number | null; turns: number; model: string | null; size: number;
}
export type TranscriptKind = "user" | "assistant" | "thinking" | "tool" | "result" | "command" | "compact" | "system";
export interface TranscriptItem {
  kind: TranscriptKind; text: string; name: string | null; time: number | null; omitted: number; error: boolean;
  /** Pairs a tool call with its result. */
  id: string | null;
  /** A tool call's input, long strings cut short. */
  input: Record<string, unknown> | null;
  images: number;
}
export interface Transcript { items: TranscriptItem[]; total: number }
/** A working folder of the selected sessions and, when Hydra can tell, the same folder on the target side. */
export interface TransferRoot { cwd: string; sessions: number; target: string | null }
export interface TransferPlan { roots: TransferRoot[]; choices: string[]; hostTarget: boolean }
export interface TransferOutcome { copied: number; skipped: number; failed: number; errors: string[] }
export interface ImportPreview { source: string; sessions: ClaudeCodeSession[]; plan: TransferPlan }
export interface TrashedItem { id: string; kind: "session" | "skill" | "agent" | "command"; name: string; detail: string | null; deletedAt: number }
export interface ClaudeTextFile { text: string; stamp: string; exists: boolean }
export interface ClaudeExtension { kind: "skill" | "agent" | "command"; name: string; description: string | null; path: string; file: string }
export interface CopyResult { target: string; status: "copied" | "exists" | "failed"; error: string | null }
export interface McpServer { name: string; project: string | null; transport: string; summary: string; config: unknown }
export interface ClaudePlugin { id: string; name: string; marketplace: string | null; version: string | null; enabled: boolean; installed: boolean }

/** What a container terminal opened from Hydra runs after the exit check passes. */
export interface ContainerLaunch { cwd: string; resume?: string | null }
export interface ContainerProject { cwd: string; host: string; sessions: number; lastActiveAt: number | null }
export interface ContainerProjects { recent: ContainerProject[]; shared: ContainerProject[] }
export interface ContainerSubfolder { name: string; cwd: string }

export interface ContainerMount {
  /** Absolute folder on the Mac. */
  source: string;
  /** Folder name under /workspace in the container. */
  name: string;
}
export interface ContainerInput {
  name: string;
  /** Environment id; it must have a proxy node. */
  environment: string;
  /** "" = Hydra's built-in image. */
  image: string;
  mounts: ContainerMount[];
  sshAgent: boolean;
  /** Mac folder for Claude Code's data; "" = the account's folder in Hydra's data directory. */
  claudeDir: string;
}
export interface Container extends ContainerInput {
  id: string;
  environmentName: string | null;
  /** The Mac folder that actually holds Claude Code's data. */
  claudeDirPath: string;
  state: "running" | "stopped" | "absent" | "unknown";
  /** Why the container must be rebuilt before it can be opened. */
  rebuild: string | null;
  imageUpdate: boolean;
  blocked: string | null;
  /** The cut lifts by itself once the exit verifies again. */
  blockedSoft: boolean;
  lastExit: ExitInfo | null;
}
export interface ContainerRuntime {
  supported: boolean;
  dockerPath: string | null;
  /** OrbStack version when connected. */
  version: string | null;
  error: string | null;
  terminal: string;
  terminals: [string, string][];
}

export interface Tokens { input: number; output: number; cacheCreation: number; cacheRead: number }
export interface CodeAccount {
  signedIn: boolean;
  /** e.g. "pro" | "max" */
  plan: string | null;
  /** e.g. "default_claude_max_20x" */
  tier: string | null;
  email: string | null;
  organization: string | null;
  expiresAt: number | null;
}
export interface CodeUsage {
  /** Start of each UTC hour (ms) with recorded usage; `cost` is API-equivalent USD. */
  hours: { start: number; tokens: Tokens; cost: number }[];
  /** `cost` is null for models without a known price. */
  models: { model: string; tokens: Tokens; cost: number | null }[];
  total: Tokens;
  responses: number;
  /** API-equivalent USD of the priced responses. */
  cost: number;
  /** Tokens of models without a known price (not in `cost`). */
  unpriced: number;
  activeMs: number;
}
export interface ContainerUsage { id: string; name: string; account: CodeAccount; usage: CodeUsage; blocked: string | null; blockedSoft: boolean }
export interface UsageDashboard { containers: ContainerUsage[]; total: CodeUsage; /** When it was built (ms). */ updatedAt: number }

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
  openAccountBrowser: (id: string, url?: string) => invoke<void>("open_account_browser", { id, url: url ?? null }),
  resumeAccount: (id: string) => invoke<string>("resume_account", { id }),
  rebindExit: (id: string) => invoke<string>("rebind_exit", { id }),
  sandboxStatus: () => invoke<string>("sandbox_status"),
  configureQuietSandbox: () => invoke<void>("configure_quiet_sandbox"),
  containerRuntime: () => invoke<ContainerRuntime>("container_runtime"),
  setContainerRuntime: (dockerPath: string | null, terminal: string) => invoke<void>("set_container_runtime", { dockerPath, terminal }),
  listContainers: () => invoke<Container[]>("list_containers"),
  saveContainer: (id: string | null, input: ContainerInput) => invoke<string>("save_container", { id, input }),
  openContainer: (id: string, launch?: ContainerLaunch) => invoke<void>("open_container", { id, launch: launch ?? null }),
  containerProjects: (id: string) => invoke<ContainerProjects>("container_projects", { id }),
  containerSubfolders: (id: string, cwd: string) => invoke<ContainerSubfolder[]>("container_subfolders", { id, cwd }),
  stopContainer: (id: string) => invoke<void>("stop_container", { id }),
  rebuildContainer: (id: string, keepChanges: boolean) => invoke<void>("rebuild_container", { id, keepChanges }),
  deleteContainer: (id: string) => invoke<string | null>("delete_container", { id }),
  createContainerShortcut: (id: string) => invoke<string>("create_container_shortcut", { id }),
  usageDashboard: () => invoke<UsageDashboard>("usage_dashboard"),
  usageDashboardCached: () => invoke<UsageDashboard | null>("usage_dashboard_cached"),
  containerQuota: (id: string) => invoke<Quota>("container_quota", { id }),
  quotaHistory: (id: string) => invoke<QuotaHistory>("quota_history", { id }),
  setQuotaSampling: (enabled: boolean) => invoke<void>("set_quota_sampling", { enabled }),
  diagnostics: () => invoke<string>("diagnostics"),
  saveTranscript: (text: string) => invoke<string>("save_transcript", { text }),
  revealTranscript: (path: string) => invoke<void>("reveal_transcript", { path }),
  checkNode: (node: NodeConfig) => invoke<[ExitInfo, string | null, string]>("check_node", { node }),
  refreshNodeIp: (url: string) => invoke<string>("refresh_node_ip", { url }),
  profileQuota: (id: string) => invoke<Quota>("profile_quota", { id }),
  recycleSession: (account: string, action: "delete" | "restore" | "empty", id = "") => invoke<void>("recycle_session", { account, action, id }),
  importChats: (account: string, raw: string) => invoke<TransferResult>("import_chats", { account, raw }),
  readImport: (account: string, id: string) => invoke<string>("read_import", { account, id }),
  sessionPreview: (source: string, target: string, trash = false) => invoke<SessionPreview>("session_preview", { source, target, trash }),
  copySessions: (source: string, target: string, ids: string[], restart = false) => invoke<TransferResult>("copy_sessions", { source, target, ids, restart }),
  claudeFolders: () => invoke<ClaudeFolderSummary[]>("claude_folder_list"),
  openClaudeFolder: (folder: string) => invoke<void>("claude_folder_open", { folder }),
  codeSessions: (folder: string) => invoke<ClaudeCodeSession[]>("code_session_list", { folder }),
  readCodeSession: (folder: string, id: string, offset: number, limit: number) => invoke<Transcript>("code_session_read", { folder, id, offset, limit }),
  trashCodeSessions: (folder: string, ids: string[]) => invoke<number>("code_session_trash", { folder, ids }),
  planCodeSessions: (source: string, ids: string[], target: string) => invoke<TransferPlan>("code_session_plan", { source, ids, target }),
  transferCodeSessions: (source: string, ids: string[], target: string, mapping: Record<string, string>, remove: boolean) =>
    invoke<TransferOutcome>("code_session_transfer", { source, ids, target, mapping, remove }),
  exportCodeSessions: (folder: string, ids: string[], path: string) => invoke<number>("code_session_export", { folder, ids, path }),
  saveHtml: (path: string, html: string) => invoke<void>("save_html", { path, html }),
  previewCodeSessionImport: (path: string, target: string) => invoke<ImportPreview>("code_session_import_preview", { path, target }),
  importCodeSessions: (path: string, target: string, mapping: Record<string, string>) => invoke<TransferOutcome>("code_session_import", { path, target, mapping }),
  claudeTrash: (folder: string) => invoke<TrashedItem[]>("claude_trash_list", { folder }),
  restoreClaudeTrash: (folder: string, id: string) => invoke<void>("claude_trash_restore", { folder, id }),
  emptyClaudeTrash: (folder: string) => invoke<void>("claude_trash_empty", { folder }),
  readClaudeFile: (folder: string, path: string) => invoke<ClaudeTextFile>("claude_file_read", { folder, path }),
  saveClaudeFile: (folder: string, path: string, text: string, stamp: string) => invoke<string>("claude_file_save", { folder, path, text, stamp }),
  claudeExtensions: (folder: string) => invoke<ClaudeExtension[]>("claude_extension_list", { folder }),
  deleteClaudeExtension: (folder: string, path: string) => invoke<void>("claude_extension_delete", { folder, path }),
  copyClaudeExtension: (folder: string, path: string, targets: string[], overwrite: boolean) => invoke<CopyResult[]>("claude_extension_copy", { folder, path, targets, overwrite }),
  mcpServers: (folder: string) => invoke<McpServer[]>("claude_mcp_list", { folder }),
  deleteMcpServer: (folder: string, project: string | null, name: string) => invoke<void>("claude_mcp_delete", { folder, project, name }),
  copyMcpServer: (folder: string, name: string, targets: string[], overwrite: boolean) => invoke<CopyResult[]>("claude_mcp_copy", { folder, name, targets, overwrite }),
  claudePlugins: (folder: string) => invoke<ClaudePlugin[]>("claude_plugin_list", { folder }),
  setClaudePlugin: (folder: string, id: string, enabled: boolean) => invoke<void>("claude_plugin_set", { folder, id, enabled }),
  releaseNotes: (after: string, upto: string) => invoke<ReleaseNote[]>("release_notes", { after, upto }),
  openReleasePage: (version: string) => invoke<void>("open_release_page", { version }),
  restartApp: () => invoke<void>("restart_app"),
  appInfo: () => invoke<AppInfo>("app_info"),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setRequireEnvironment: (enabled: boolean) => invoke<void>("set_require_environment", { enabled }),
  setLightweight: (enabled: boolean) => invoke<void>("set_lightweight", { enabled }),
  setAppIcon: (id: string) => invoke<void>("set_app_icon", { id }),
  setClaudePath: (path: string | null) => invoke<void>("set_claude_path", { path }),
  quit: () => invoke<void>("quit_app"),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
