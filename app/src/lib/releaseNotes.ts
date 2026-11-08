/** A release body parsed for display. Text only: nothing in a release body becomes HTML. */
export type NoteBlock =
  | { type: "heading"; text: string }
  | { type: "item"; depth: 0 | 1; text: string }
  | { type: "text"; text: string };

/** Sections that describe installation rather than changes. */
const SKIP = /^(install|installation|安装|下载)$/i;

export function parseNotes(body: string): NoteBlock[] {
  const blocks: NoteBlock[] = [];
  let skipping = false;
  for (const raw of body.replace(/<!--[\s\S]*?-->/g, "").split("\n")) {
    if (!raw.trim()) continue;
    const heading = /^#{1,6}\s+(.+?)\s*#*$/.exec(raw.trim());
    if (heading) {
      skipping = SKIP.test(heading[1]);
      if (!skipping) blocks.push({ type: "heading", text: heading[1] });
      continue;
    }
    if (skipping) continue;
    const item = /^(\s*)[-*+]\s+(.+)$/.exec(raw);
    if (item) blocks.push({ type: "item", depth: item[1].length >= 2 ? 1 : 0, text: item[2].trim() });
    else blocks.push({ type: "text", text: raw.trim() });
  }
  return blocks;
}

/** Inline runs: **bold**, `code`, and plain text. */
export function inlineRuns(text: string): { kind: "bold" | "code" | "plain"; text: string }[] {
  return text.split(/(\*\*[^*]+\*\*|`[^`]+`)/).filter(Boolean).map((part) =>
    part.startsWith("**") && part.endsWith("**") && part.length > 4 ? { kind: "bold", text: part.slice(2, -2) }
      : part.startsWith("`") && part.endsWith("`") && part.length > 2 ? { kind: "code", text: part.slice(1, -1) }
        : { kind: "plain", text: part });
}

/** `a` is newer than `b` (stable `major.minor.patch`; anything else compares as not newer). */
export function isNewer(a: string, b: string): boolean {
  const p = (v: string) => /^v?(\d+)\.(\d+)\.(\d+)$/.exec(v.trim())?.slice(1).map(Number);
  const [x, y] = [p(a), p(b)];
  if (!x || !y) return false;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}
