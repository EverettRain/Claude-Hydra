import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { api, type ClaudeCodeSession, type TranscriptItem } from "@/lib/api";
import { Conversation, Exporting, type ImageSource } from "./ChatView";
import { formatTime } from "./shared";

const PAGE = 500;
/** Images embedded in one page at most (as data URL text), so a session full of screenshots still opens. */
const IMAGE_BUDGET = 60 * 1024 * 1024;

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

function Page({ session, items, images, skipped }: { session: ClaudeCodeSession; items: TranscriptItem[]; images: ImageSource; skipped: number }) {
  return (
    <div className="claude-chat min-h-screen bg-bg text-fg">
      <header className="mx-auto max-w-[760px] border-b-[0.5px] border-line-2 px-5 pb-4 pt-8">
        <h1 className="text-[20px] font-semibold leading-snug">{session.title}</h1>
        <p className="mt-1 text-[12px] text-muted">{session.cwd ?? "未记录工作目录"} · {session.model ?? "模型未知"} · {formatTime(session.lastActivityAt)}</p>
      </header>
      <main className="mx-auto max-w-[760px] space-y-4 px-5 pb-12 pt-6">
        <Conversation items={items} images={images} />
      </main>
      <footer className="mx-auto max-w-[760px] px-5 pb-8 text-[11.5px] text-faint">
        由 Claude-Hydra 导出 · {formatTime(Date.now())}{skipped > 0 && ` · 图片太多，另有 ${skipped} 张没有放进这个文件`}
      </footer>
    </div>
  );
}

/**
 * The session as one self-contained HTML page: the conversation view rendered here with every fold
 * and image included, the app's styles inlined. Folds are `<details>`, so the page needs no script beyond
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
  // Every image is fetched now and embedded, up to the budget.
  const preloaded = new Map<string, string>();
  let size = 0, skipped = 0;
  for (const [index, item] of items.entries()) {
    for (let n = 0; n < item.images; n++) {
      const src = size < IMAGE_BUDGET ? await api.sessionImage(folder, session.id, index, n).catch(() => null) : null;
      if (src && size + src.length <= IMAGE_BUDGET) { preloaded.set(`${index}:${n}`, src); size += src.length; } else skipped++;
    }
  }
  const host = document.createElement("div");
  const root = createRoot(host);
  flushSync(() => root.render(<Exporting.Provider value><Page session={session} items={items} images={{ preloaded }} skipped={skipped} /></Exporting.Provider>));
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
