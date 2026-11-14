import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { api, type ClaudeCodeSession, type TranscriptItem } from "@/lib/api";
import { Conversation, Exporting } from "./ChatView";
import { formatTime } from "./shared";

const PAGE = 500;

const escape = (text: string) => text.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

/**
 * The app's own style sheets, so the page looks as it does in Hydra; less the rules that hide
 * scrollbars, which suit the app window but not a page in a browser.
 */
function styles(): string {
  return [...document.styleSheets].map((sheet) => {
    try {
      return [...sheet.cssRules].map((r) => r.cssText).filter((t) => !t.includes("::-webkit-scrollbar") && !/scrollbar-width:\s*none/.test(t)).join("\n");
    } catch { return ""; }
  }).join("\n");
}

function Page({ session, items }: { session: ClaudeCodeSession; items: TranscriptItem[] }) {
  return (
    <div className="claude-chat min-h-screen bg-bg text-fg">
      <header className="mx-auto max-w-[760px] border-b-[0.5px] border-line-2 px-5 pb-4 pt-8">
        <h1 className="text-[20px] font-semibold leading-snug">{session.title}</h1>
        <p className="mt-1 text-[12px] text-muted">{session.cwd ?? "未记录工作目录"} · {session.model ?? "模型未知"} · {formatTime(session.lastActivityAt)}</p>
      </header>
      <main className="mx-auto max-w-[760px] space-y-4 px-5 pb-12 pt-6">
        <Conversation items={items} />
      </main>
      <footer className="mx-auto max-w-[760px] px-5 pb-8 text-[11.5px] text-faint">由 Claude-Hydra 导出 · {formatTime(Date.now())}</footer>
    </div>
  );
}

/**
 * The session as one self-contained HTML page: the conversation view rendered here with every fold
 * included, the app's styles inlined. Folds are `<details>`, so the page needs no script beyond
 * choosing light or dark like the system; messages are text, never markup.
 */
export async function sessionHtml(folder: string, session: ClaudeCodeSession): Promise<string> {
  const items: TranscriptItem[] = [];
  for (let total = Infinity; items.length < total;) {
    const page = await api.readCodeSession(folder, session.id, items.length, PAGE);
    total = page.total;
    if (!page.items.length) break;
    items.push(...page.items);
  }
  const host = document.createElement("div");
  const root = createRoot(host);
  flushSync(() => root.render(<Exporting.Provider value><Page session={session} items={items} /></Exporting.Provider>));
  const body = host.innerHTML;
  root.unmount();
  return `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(session.title)}</title>
<script>if (matchMedia("(prefers-color-scheme: dark)").matches) document.documentElement.classList.add("dark")</script>
<style>
${styles()}
/* The app pins itself to its window; a saved page scrolls and selects like any page. */
html, body, #root { height: auto; overflow: visible; }
body { -webkit-user-select: text; user-select: text; }
/* Copying needs the app; a saved page selects text instead. */
.md-code-bar button { display: none; }
.md .md-link { cursor: text; }
</style>
</head>
<body>
${body}
</body>
</html>
`;
}
