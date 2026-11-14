import { useEffect, useState } from "react";
import { CodeSessions } from "@/components/config/CodeSessions";
import { Extensions, McpAndPlugins, MemoryAndSettings, Trash } from "@/components/config/ConfigTabs";
import { DesktopSessions } from "@/components/config/DesktopSessions";
import { Collapse, LocationHeader, LocationPicker } from "@/components/config/LocationPanel";
import { Page } from "@/components/PageHeader";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type ClaudeFolderSummary, type Profile } from "@/lib/api";
import { cn } from "@/lib/utils";

type Tab = "sessions" | "desktop" | "memory" | "extensions" | "mcp" | "trash";
const tabs: { id: Tab; label: string }[] = [
  { id: "sessions", label: "会话" },
  { id: "desktop", label: "桌面版会话" },
  { id: "memory", label: "记忆与设置" },
  { id: "extensions", label: "技能与命令" },
  { id: "mcp", label: "MCP 与插件" },
  { id: "trash", label: "回收站" },
];

const REMEMBER = "hydra.config.folder";
function remembered() {
  try { return localStorage.getItem(REMEMBER) ?? ""; } catch { return ""; }
}

/**
 * Browse and manage every Claude Code data folder: this computer's own, desktop accounts' and containers'.
 * The page opens on a picker of all locations; choosing one folds it into a header above the tabs.
 */
export function ConfigPage({ profiles }: { profiles: Profile[] | null }) {
  const toast = useToast();
  const [folders, setFolders] = useState<ClaudeFolderSummary[] | null>(null);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState(remembered);
  // Only a location picked on this visit counts as chosen; the remembered one is just highlighted.
  const [chosen, setChosen] = useState(false);
  const [picking, setPicking] = useState(true);
  const [tab, setTab] = useState<Tab>("sessions");

  // Accounts come and go with the account list; reload the locations with it.
  useEffect(() => {
    let current = true;
    api.claudeFolders().then((f) => { if (current) { setFolders(f); setError(""); } }).catch((e) => { if (current) setError(errorText(e)); });
    return () => { current = false; };
  }, [profiles]);

  const folder = folders?.find((f) => f.id === selected);
  const account = folder?.profile ? profiles?.find((p) => p.id === folder.profile) : undefined;
  const shown = tabs.filter((t) => t.id !== "desktop" || account);
  const active = shown.some((t) => t.id === tab) ? tab : "sessions";
  const choose = (id: string) => {
    setSelected(id); setChosen(true); setPicking(false);
    try { localStorage.setItem(REMEMBER, id); } catch { /* remembered for this visit only */ }
  };
  const open = chosen && !picking && folder;

  return (
    <Page>
      <div className="space-y-3 px-5">
        {error && <p role="alert" className="text-sm text-danger">{error}</p>}
        {!folders && !error && <p className="text-sm text-muted">正在读取…</p>}
        {folders && (
          <div className="rounded-xl bg-card shadow-card">
            <Collapse open={!open}>
              <LocationPicker folders={folders} current={folder?.id ?? ""} onPick={choose} onCancel={chosen && folder ? () => setPicking(false) : undefined} />
            </Collapse>
            <Collapse open={!!open}>
              {folder && <>
                <LocationHeader folder={folder} onSwitch={() => setPicking(true)}
                  onReveal={() => api.openClaudeFolder(folder.id).catch((e) => toast(errorText(e), "error"))} />
                <div role="tablist" aria-label="内容" className="mx-3 mb-3 flex gap-0.5 overflow-x-auto rounded-lg bg-pill p-0.5">
                  {shown.map((t) => (
                    <button key={t.id} role="tab" aria-selected={t.id === active} onClick={() => setTab(t.id)}
                      className={cn("h-7 shrink-0 flex-1 whitespace-nowrap rounded-md px-2.5 text-[12.5px] active:!scale-100",
                        t.id === active ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg")}>
                      {t.label}
                    </button>
                  ))}
                </div>
              </>}
            </Collapse>
          </div>
        )}
        {open && folders && (
          <div key={`${folder.id}/${active}`} className="view-in">
            {active === "sessions" && <CodeSessions folder={folder} folders={folders} />}
            {active === "desktop" && account && profiles && (
              <DesktopSessions account={account} profiles={profiles} select={(id) => choose(`desktop:${id}`)} />
            )}
            {active === "memory" && <MemoryAndSettings folder={folder} />}
            {active === "extensions" && <Extensions folder={folder} folders={folders} />}
            {active === "mcp" && <McpAndPlugins folder={folder} folders={folders} />}
            {active === "trash" && <Trash folder={folder} />}
          </div>
        )}
      </div>
    </Page>
  );
}
