// Release notes for a tag, from the commits since the previous tag.
//
//   node app/scripts/release-notes.mjs v1.0.1 [previous-tag]
//
// Each commit subject becomes one item, grouped as 新功能 / 改进 / 问题修复; the "- " lines of
// its body become sub-items (a body without them is kept as one sub-item). Version bumps,
// trailers (Co-Authored-By, …) and commits that leave the app untouched (README, docs, CI,
// these scripts) are left out. The markdown is what the in-app update dialog
// renders, and what tauri-action writes to the release and to latest.json.
import { execFileSync } from "node:child_process";

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const tag = process.argv[2];
if (!tag) throw new Error("usage: release-notes.mjs <tag> [previous-tag]");

let prev = process.argv[3];
if (!prev) {
  try { prev = git("describe", "--tags", "--abbrev=0", "--match", "v*", `${tag}^`); } catch { prev = ""; }
}

const SEP = "\u001e";
// Each record: subject, body, then the files the commit changed (from --name-only).
const log = git("log", "--no-merges", "--name-only", `--format=${SEP}%s%x1f%b%x1f`, prev ? `${prev}..${tag}` : tag);
const outsideApp = (file) => /^(docs\/|\.github\/|app\/scripts\/)/.test(file) || /\.md$/i.test(file);
const groups = { 新功能: [], 改进: [], 问题修复: [] };

for (const raw of log.split(SEP)) {
  const [subjectRaw = "", body = "", changed = ""] = raw.trim().split("\u001f");
  const files = changed.split("\n").map((f) => f.trim()).filter(Boolean);
  if (files.length && files.every(outsideApp)) continue;
  const subject = subjectRaw.replace(/^(feat|fix|chore|refactor|perf|docs)\s*[:：]\s*/i, "").trim();
  if (!subject || /^(发布|release|bump)\s*v?\d+\.\d+\.\d+/i.test(subject)) continue;
  const lines = body.split("\n").map((l) => l.trim()).filter((l) => l && !/^[\w-]+:\s/.test(l));
  const bullets = lines.filter((l) => /^[-*]\s+/.test(l)).map((l) => l.replace(/^[-*]\s+/, ""));
  const subs = bullets.length ? bullets : lines.length ? [lines.join(" ")] : [];
  const kind = /^(新增|支持|内置|加入|增加|feat|add)/i.test(subjectRaw) ? "新功能"
    : /(修复|fix)/i.test(subject) ? "问题修复" : "改进";
  groups[kind].push([`- ${subject}`, ...subs.map((s) => `  - ${s}`)].join("\n"));
}

const out = Object.entries(groups).filter(([, items]) => items.length)
  .map(([title, items]) => `### ${title}\n\n${items.join("\n")}`).join("\n\n");
process.stdout.write(`${out || "### 改进\n\n- 常规维护与稳定性改进"}\n`);
