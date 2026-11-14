import { AlertCircle, ArrowLeft, Check, ChevronRight, Copy, Download, FileText, ImageIcon, MoveRight, Play, Wrench } from "lucide-react";
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { WindowControls } from "@/components/AppShell";
import { Button } from "@/components/ui/button";
import { api, errorText, type ClaudeCodeSession, type TranscriptItem } from "@/lib/api";
import { cn } from "@/lib/utils";
import { CodeBlock, Markdown } from "./Markdown";
import { formatTime, leaf } from "./shared";

const PAGE = 300;
const isMac = navigator.userAgent.includes("Mac");

interface Call { tool?: TranscriptItem; result?: TranscriptItem }
type Step = { type: "thinking"; item: TranscriptItem } | { type: "tool"; call: Call };
/** What the conversation shows: messages, and between them the work (thinking and tools) folded into one block. */
type Block =
  | { type: "user" | "assistant" | "note"; item: TranscriptItem }
  | { type: "work"; steps: Step[] }
  /** A question Claude asked with its choices, shown on its own like the app does. */
  | { type: "question"; call: Call };

const QUESTION_TOOL = "AskUserQuestion";

/** True while rendering a file to save: every fold's content is included, as nothing loads later. */
export const Exporting = createContext(false);

/**
 * A native `<details>` fold, so a saved page opens and closes without scripts. In the app its
 * content is rendered only once opened, keeping long sessions light. Folds nest, so each kind
 * names its own group (`group/work`, `group/tool`) for styles that follow its open state.
 */
function Fold({ summary, className, summaryClass, children }: { summary: ReactNode; className?: string; summaryClass?: string; children: ReactNode }) {
  const all = useContext(Exporting);
  const [open, setOpen] = useState(false);
  return (
    <details className={className} onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary className={cn("cursor-pointer list-none [&::-webkit-details-marker]:hidden", summaryClass)}>{summary}</summary>
      {(open || all) && children}
    </details>
  );
}

function build(items: TranscriptItem[]): Block[] {
  const out: Block[] = [];
  const calls = new Map<string, Call>();
  const work = () => {
    const last = out[out.length - 1];
    if (last?.type === "work") return last.steps;
    const steps: Step[] = [];
    out.push({ type: "work", steps });
    return steps;
  };
  for (const item of items) {
    if (item.kind === "thinking") work().push({ type: "thinking", item });
    else if (item.kind === "tool") {
      const call: Call = { tool: item };
      if (item.id) calls.set(item.id, call);
      if (item.name === QUESTION_TOOL) out.push({ type: "question", call });
      else work().push({ type: "tool", call });
    } else if (item.kind === "result") {
      const call = item.id ? calls.get(item.id) : undefined;
      if (call) call.result = item; else work().push({ type: "tool", call: { result: item } });
    } else out.push({ type: item.kind === "user" ? "user" : item.kind === "assistant" ? "assistant" : "note", item });
  }
  return out;
}

const str = (v: unknown) => (typeof v === "string" ? v : "");

/** The one argument that says what a tool call did: a file, a command, a search. */
function summary(name: string, input: Record<string, unknown> | null): string {
  const i = input ?? {};
  const path = str(i.file_path) || str(i.notebook_path) || str(i.path);
  switch (name) {
    case "Read": case "Write": case "Edit": case "MultiEdit": case "NotebookEdit": return path ? leaf(path) : "";
    case "Bash": return str(i.description) || str(i.command).split("\n")[0];
    case "Grep": case "Glob": return str(i.pattern) + (path ? ` · ${leaf(path)}` : "");
    case "WebFetch": return str(i.url);
    case "WebSearch": return str(i.query);
    case "Task": case "Agent": return str(i.description);
    case "TodoWrite": return "更新待办";
    default: return Object.values(i).find((v): v is string => typeof v === "string")?.split("\n")[0] ?? "";
  }
}

function Diff({ before, after }: { before: string; after: string }) {
  const lines = (text: string, sign: string, cls: string) => text.split("\n").map((l, i) => <div key={sign + i} className={cls}>{sign} {l}</div>);
  return (
    <pre className="selectable overflow-x-auto rounded-md bg-card px-2 py-1.5 font-mono text-[11.5px] leading-[1.55]">
      {lines(before, "-", "bg-danger-soft text-danger")}
      {lines(after, "+", "bg-ok-soft text-ok")}
    </pre>
  );
}

/** A tool's input the way a person reads it; anything unfamiliar as JSON. */
function Input({ tool }: { tool: TranscriptItem }) {
  const i = tool.input ?? {};
  const path = str(i.file_path);
  const pathLine = path && <div className="selectable truncate font-mono text-[11.5px] text-muted" title={path}>{path}</div>;
  switch (tool.name) {
    case "Bash":
      return <CodeBlock code={str(i.command)} lang={str(i.description) || "bash"} />;
    case "Edit":
      return <div className="space-y-1">{pathLine}<Diff before={str(i.old_string)} after={str(i.new_string)} /></div>;
    case "MultiEdit":
      return <div className="space-y-1">{pathLine}{(Array.isArray(i.edits) ? i.edits : []).map((e, n) =>
        <Diff key={n} before={str((e as Record<string, unknown>).old_string)} after={str((e as Record<string, unknown>).new_string)} />)}</div>;
    case "Write":
      return <div className="space-y-1">{pathLine}<CodeBlock code={str(i.content)} lang={leaf(path)} /></div>;
    case "TodoWrite":
      return <ul className="space-y-0.5 text-[12.5px]">{(Array.isArray(i.todos) ? i.todos : []).map((t, n) => {
        const todo = t as Record<string, unknown>;
        return <li key={n} className={cn(todo.status === "completed" && "text-muted line-through")}>{todo.status === "completed" ? "☑" : todo.status === "in_progress" ? "◐" : "☐"} {str(todo.content)}</li>;
      })}</ul>;
    default:
      return <CodeBlock code={tool.text} lang="输入" />;
  }
}

function ToolRow({ call }: { call: Call }) {
  const { tool, result } = call;
  const name = tool?.name ?? "工具结果";
  const failed = !!result?.error;
  return (
    <Fold className="group/tool" summaryClass="flex w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-left text-[12.5px] hover:bg-pill" summary={<>
      <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted transition-transform group-open/tool:rotate-90" />
      <span className="shrink-0 font-medium">{name}</span>
      <span className="min-w-0 truncate text-muted">{tool ? summary(name, tool.input) : ""}</span>
      {failed && <AlertCircle className="ml-auto h-3.5 w-3.5 shrink-0 text-danger" aria-label="出错" />}
    </>}>
        <div className="ml-5 space-y-2 border-l border-line py-1.5 pl-3">
          {tool && <Input tool={tool} />}
          {result && (
            <div>
              <div className={cn("mb-1 text-[11px]", failed ? "text-danger" : "text-muted")}>{failed ? "出错" : "结果"}{result.images ? ` · ${result.images} 张图片` : ""}</div>
              <pre className={cn("selectable max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md px-2 py-1.5 font-mono text-[11.5px] leading-[1.55]",
                failed ? "bg-danger-soft text-danger" : "bg-card")}>{result.text || "（没有输出）"}</pre>
              {result.omitted > 0 && <p className="mt-1 text-[11px] text-muted">另有 {result.omitted.toLocaleString()} 个字符未显示</p>}
            </div>
          )}
          {tool && !result && <p className="text-[11.5px] text-muted">没有记录结果</p>}
        </div>
    </Fold>
  );
}

/** Two lines of the thinking until opened; the whole of it is in the summary, so it needs no content. */
function Thinking({ item }: { item: TranscriptItem }) {
  return (
    <details className="group/think">
      <summary className="cursor-pointer list-none rounded-md px-1.5 py-1 text-[12.5px] italic text-muted hover:bg-pill [&::-webkit-details-marker]:hidden">
        <span className="selectable line-clamp-2 whitespace-pre-wrap group-open/think:line-clamp-none">{item.text}</span>
      </summary>
    </details>
  );
}

/** Thinking and tool calls between two messages, folded like the Claude app does. */
function Work({ steps }: { steps: Step[] }) {
  const tools = steps.filter((s): s is Extract<Step, { type: "tool" }> => s.type === "tool");
  const failed = tools.filter((s) => s.call.result?.error).length;
  const names = [...new Set(tools.map((s) => s.call.tool?.name).filter(Boolean))].slice(0, 3).join("、");
  const label = tools.length ? `${steps.length > tools.length ? "思考并" : ""}使用了 ${tools.length} 个工具` : "思考";
  return (
    <Fold className="group/work rounded-xl bg-card-2" summaryClass="flex w-full items-center gap-2 px-3 py-2 text-left text-[12.5px] text-fg-2" summary={<>
      <Wrench className="h-3.5 w-3.5 shrink-0 text-muted" />
      <span className="shrink-0">{label}</span>
      {names && <span className="min-w-0 truncate text-muted">{names}</span>}
      {failed > 0 && <span className="shrink-0 text-danger">{failed} 个出错</span>}
      <ChevronRight className="ml-auto h-3.5 w-3.5 shrink-0 text-muted transition-transform group-open/work:rotate-90" />
    </>}>
      <div className="space-y-0.5 px-2 pb-2">
        {steps.map((s, i) => s.type === "thinking" ? <Thinking key={i} item={s.item} /> : <ToolRow key={i} call={s.call} />)}
      </div>
    </Fold>
  );
}

function Note({ item }: { item: TranscriptItem }) {
  if (item.kind === "command") return <div className="flex justify-end"><span className="selectable rounded-full bg-pill px-2.5 py-0.5 font-mono text-[11.5px] text-muted">{item.text}</span></div>;
  if (item.kind === "compact") {
    return (
      <details className="group text-[12px] text-muted">
        <summary className="flex cursor-pointer list-none items-center gap-3"><span className="h-px flex-1 bg-line" />上下文已压缩<span className="h-px flex-1 bg-line" /></summary>
        {item.text.length > 20 && <div className="mt-2 rounded-xl bg-card-2 px-3 py-2"><Markdown text={item.text} /></div>}
      </details>
    );
  }
  return <p className={cn("selectable whitespace-pre-wrap text-center text-[12px]", item.error ? "text-danger" : "text-muted")}>{item.text}</p>;
}

interface Question { question: string; header?: string; multiSelect?: boolean; options?: { label: string; description?: string }[] }

/**
 * The answer to `question` in the tool result, which reads `"question"="answer", "next"="…"`.
 * An answer may contain quotes, so it runs to the quote that closes the pair.
 */
function answerTo(result: string, question: string): string | null {
  const start = result.indexOf(`"${question}"="`);
  if (start < 0) return null;
  const from = start + question.length + 4;
  const ends = ['", "', '". ', '"\n'].map((m) => result.indexOf(m, from)).filter((i) => i >= 0);
  const end = ends.length ? Math.min(...ends) : result.lastIndexOf('"');
  return end > from ? result.slice(from, end) : null;
}

function QuestionCard({ call }: { call: Call }) {
  const questions = (Array.isArray(call.tool?.input?.questions) ? call.tool!.input!.questions : []) as Question[];
  const result = call.result;
  return (
    <div className="space-y-3 rounded-2xl bg-card px-4 py-3 shadow-card">
      {questions.map((q, i) => {
        const answer = result && !result.error ? answerTo(result.text, q.question) : null;
        const chosen = new Set((answer ?? "").split(/,\s*/));
        const known = (q.options ?? []).some((o) => chosen.has(o.label));
        return (
          <div key={i} className="space-y-2">
            <div className="flex items-start gap-2">
              {q.header && <span className="mt-0.5 shrink-0 rounded-md bg-pill px-1.5 py-0.5 text-[11px] text-fg-2">{q.header}</span>}
              <p className="selectable text-[13.5px] font-medium leading-6">{q.question}</p>
            </div>
            <div className="space-y-1">
              {(q.options ?? []).map((o) => {
                const on = chosen.has(o.label);
                return (
                  <div key={o.label} className={cn("rounded-lg px-3 py-1.5", on ? "bg-pill shadow-[inset_0_0_0_1.5px_rgb(var(--accent))]" : "bg-card-2")}>
                    <div className="flex items-center gap-1.5 text-[13px]">
                      {on && <Check className="h-3.5 w-3.5 shrink-0 text-accent" />}
                      <span className={cn("selectable", on && "font-medium")}>{o.label}</span>
                    </div>
                    {o.description && <p className="selectable text-[12px] leading-5 text-muted">{o.description}</p>}
                  </div>
                );
              })}
              {answer && !known && (
                <div className="rounded-lg bg-pill px-3 py-1.5 text-[13px] shadow-[inset_0_0_0_1.5px_rgb(var(--accent))]">
                  <span className="text-muted">其他：</span><span className="selectable whitespace-pre-wrap">{answer}</span>
                </div>
              )}
            </div>
          </div>
        );
      })}
      {!questions.length && <pre className="selectable whitespace-pre-wrap font-mono text-[11.5px]">{call.tool?.text}</pre>}
      <p className={cn("text-[11.5px]", result?.error ? "text-danger" : "text-muted")}>
        {!result ? "没有记录回答" : result.error ? `没有回答：${result.text}` : questions.length > 1 ? "已回答" : ""}
      </p>
    </div>
  );
}

function Message({ block }: { block: Block }): ReactNode {
  if (block.type === "work") return <Work steps={block.steps} />;
  if (block.type === "question") return <QuestionCard call={block.call} />;
  const { item } = block;
  if (block.type === "user") {
    return (
      <div className="flex justify-end">
        <div className="max-w-[85%] space-y-1 rounded-2xl bg-pill px-3.5 py-2 text-[13.5px] leading-[1.65]">
          {item.text && <div className="selectable whitespace-pre-wrap break-words">{item.text}</div>}
          {item.images > 0 && <div className="flex items-center gap-1 text-[12px] text-muted"><ImageIcon className="h-3.5 w-3.5" />附带 {item.images} 张图片</div>}
        </div>
      </div>
    );
  }
  if (block.type === "assistant") {
    return <div><Markdown text={item.text} />{item.omitted > 0 && <p className="mt-1 text-[11.5px] text-muted">（另有 {item.omitted.toLocaleString()} 个字符未显示）</p>}</div>;
  }
  return <Note item={item} />;
}

/** A session's conversation: messages, folded work and question cards. Also rendered into saved pages. */
export function Conversation({ items }: { items: TranscriptItem[] }) {
  const blocks = useMemo(() => build(items), [items]);
  return <>{blocks.map((b, i) => <Message key={i} block={b} />)}</>;
}

/** One session as a conversation filling the content area, loaded page by page as it scrolls. */
export function ChatView({ folder, session, onBack, onCopy, onMove, onExport, onExportHtml, onResume }: {
  folder: string; session: ClaudeCodeSession; onBack: () => void; onCopy: () => void; onMove: () => void; onExport: () => void; onExportHtml: () => void;
  /** Continues the session in its container; absent elsewhere. */
  onResume?: () => void;
}) {
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const end = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  // Escape goes back, unless a dialog (copy, move) is open on top.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && !document.querySelector("[data-dialog-layer]")) onBack(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onBack]);
  const load = (offset: number) => {
    setLoading(true); setError("");
    api.readCodeSession(folder, session.id, offset, PAGE)
      .then((t) => { setItems((old) => (offset ? [...old, ...t.items] : t.items)); setTotal(t.total); })
      .catch((e) => setError(errorText(e))).finally(() => setLoading(false));
  };
  useEffect(() => { setItems([]); setTotal(null); load(0); }, [folder, session.id]);
  const more = total !== null && items.length < total;
  useEffect(() => {
    if (!more || loading || error || !end.current) return;
    const seen = new IntersectionObserver((entries) => { if (entries.some((e) => e.isIntersecting)) load(items.length); }, { root: body.current, rootMargin: "600px" });
    seen.observe(end.current);
    return () => seen.disconnect();
  }, [more, loading, error, items.length]);

  return (
    <div className="claude-chat flex h-full flex-col bg-bg text-fg">
      {/* The window's own top bar is hidden here: this bar drags the window and holds its controls. */}
      <div data-tauri-drag-region className="flex h-12 shrink-0 items-center gap-2 border-b-[0.5px] border-line-2 pl-4 pr-3">
        <Button size="sm" variant="ghost" onClick={onBack}><ArrowLeft />返回</Button>
        <div data-tauri-drag-region className="min-w-0 flex-1">
          <h2 className="pointer-events-none truncate text-[14px] font-semibold">{session.title}</h2>
          <p className="pointer-events-none truncate text-[11.5px] text-muted">
            {session.cwd ?? "未记录工作目录"} · {session.model ?? "模型未知"} · {formatTime(session.lastActivityAt)}
          </p>
        </div>
        {onResume && <Button size="sm" variant="outline" title="出口检测通过后，在容器里继续这个会话" onClick={onResume}><Play />在容器中继续</Button>}
        <Button size="iconSm" variant="ghost" aria-label="复制到其他位置" title="复制到其他位置" onClick={onCopy}><Copy /></Button>
        <Button size="iconSm" variant="ghost" aria-label="移动到其他位置" title="移动到其他位置" onClick={onMove}><MoveRight /></Button>
        <Button size="iconSm" variant="ghost" aria-label="导出为网页" title="导出为网页（HTML，可直接用浏览器阅读）" onClick={onExportHtml}><FileText /></Button>
        <Button size="iconSm" variant="ghost" aria-label="导出备份" title="导出备份（zip，可导入到其他位置继续使用）" onClick={onExport}><Download /></Button>
        {!isMac && <WindowControls />}
      </div>
      <div ref={body} className="min-h-0 flex-1 overflow-y-auto px-5 pt-5">
      <div className="mx-auto max-w-[760px] space-y-4 pb-8">
        <Conversation items={items} />
        {error && <p role="alert" className="text-sm text-danger">{error} <button className="underline" onClick={() => load(items.length)}>重试</button></p>}
        {loading && <p className="text-center text-sm text-muted">正在读取…</p>}
        {!loading && !error && total === 0 && <p className="text-center text-sm text-muted">这个会话没有可显示的内容。</p>}
        <div ref={end} />
      </div>
      </div>
    </div>
  );
}
