import type { Environment, EnvironmentInput } from "./api";

export const LOCALES: [string, string][] = [
  ["", "跟随系统"], ["en-US", "英语（美国）"], ["en-GB", "英语（英国）"],
  ["zh-CN", "简体中文"], ["zh-TW", "繁體中文"], ["ja-JP", "日语"], ["ko-KR", "韩语"],
];

export const emptyEnvironment = (): EnvironmentInput => ({ name: "", node: null, timezone: "proxy", locale: "", args: "", env: "", lastCheck: null });

/** One-line description: proxy · timezone · language · extras. */
export function environmentSummary(e: EnvironmentInput): string {
  const parts: string[] = [];
  if (e.node) {
    const host = e.node.host.includes(":") ? `[${e.node.host}]` : e.node.host;
    parts.push(`${e.node.kind.toUpperCase()} ${host}:${e.node.port}`);
  } else parts.push("直连");
  parts.push(e.timezone === "system" ? "系统时区" : !e.timezone || e.timezone === "proxy" ? (e.node ? "时区跟随代理" : "系统时区") : e.timezone);
  const lang = LOCALES.find(([v]) => v === e.locale);
  if (e.locale && lang) parts.push(lang[1]);
  const extras = [e.args.trim() && "启动参数", e.env.trim() && "环境变量"].filter(Boolean);
  if (extras.length) parts.push(extras.join("、"));
  return parts.join(" · ");
}

export function toInput(e: Environment): EnvironmentInput {
  const { name, node, timezone, locale, args, env, lastCheck } = e;
  return { name, node, timezone, locale, args, env, lastCheck: lastCheck ?? null };
}
