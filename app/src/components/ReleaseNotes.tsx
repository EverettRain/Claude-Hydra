import { ExternalLink } from "lucide-react";
import { api, type ReleaseNote } from "@/lib/api";
import { inlineRuns, parseNotes, type NoteBlock } from "@/lib/releaseNotes";
import { cn } from "@/lib/utils";

/** Accent per change group; unknown headings fall back to neutral. */
const TONE: Record<string, string> = {
  新功能: "bg-brand", Features: "bg-brand",
  改进: "bg-ok", Improvements: "bg-ok",
  问题修复: "bg-warn", "Bug Fixes": "bg-warn", 修复: "bg-warn",
};

function Inline({ text }: { text: string }) {
  return <>{inlineRuns(text).map((r, i) =>
    r.kind === "bold" ? <b key={i} className="font-semibold text-fg">{r.text}</b>
      : r.kind === "code" ? <code key={i} className="rounded bg-pill px-1 font-mono text-[11.5px]">{r.text}</code>
        : <span key={i}>{r.text}</span>)}</>;
}

function Blocks({ blocks }: { blocks: NoteBlock[] }) {
  return <div className="space-y-1">
    {blocks.map((b, i) => b.type === "heading"
      ? <div key={i} className={cn("flex items-center gap-1.5 pb-0.5 text-[11.5px] font-semibold uppercase tracking-[0.04em] text-fg-2", i > 0 && "pt-3")}>
          <span className={cn("h-1.5 w-1.5 rounded-full", TONE[b.text] ?? "bg-faint")} />{b.text}
        </div>
      : b.type === "item"
        ? <div key={i} className={cn("flex gap-2 leading-[19px]", b.depth ? "pl-[18px] text-[12px] text-muted" : "pt-1 text-[12.5px] text-fg")}>
            <span className={cn("mt-[8px] shrink-0 rounded-full", b.depth ? "h-[3px] w-[3px] bg-faint" : "h-1 w-1 bg-fg-2")} />
            <span className="min-w-0"><Inline text={b.text} /></span>
          </div>
        : <p key={i} className="text-[12.5px] leading-[19px] text-fg-2"><Inline text={b.text} /></p>)}
  </div>;
}

const day = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString("zh-CN", { year: "numeric", month: "long", day: "numeric" });
};

/** Release notes, newest first; a version header only when several releases are shown. */
export function ReleaseNotes({ releases }: { releases: ReleaseNote[] }) {
  return <div className="selectable space-y-5">
    {releases.map((r) => {
      const blocks = parseNotes(r.body);
      return <section key={r.version}>
        {releases.length > 1 && <div className="mb-2 flex items-baseline gap-2 border-b-[0.5px] border-line pb-1.5">
          <span className="text-[13px] font-semibold">v{r.version}</span>
          <span className="text-[11.5px] text-muted">{day(r.date)}</span>
        </div>}
        {blocks.length ? <Blocks blocks={blocks} />
          : <p className="text-[12.5px] text-muted">此版本没有提供更新说明。
              <button className="ml-1 inline-flex items-center gap-0.5 text-brand hover:underline" onClick={() => void api.openReleasePage(r.version)}>
                在 GitHub 查看<ExternalLink className="h-3 w-3" /></button>
            </p>}
      </section>;
    })}
  </div>;
}
