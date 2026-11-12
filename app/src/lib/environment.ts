import type { Environment, EnvironmentInput } from "./api";

export const LOCALES: [string, string][] = [
  ["", "跟随系统"], ["en-US", "英语（美国）"], ["en-GB", "英语（英国）"],
  ["zh-CN", "简体中文"], ["zh-TW", "繁體中文"], ["ja-JP", "日语"], ["ko-KR", "韩语"],
];

export const emptyEnvironment = (): EnvironmentInput => ({ name: "", node: null, timezone: "proxy", locale: "", args: "", env: "", lastCheck: null, exitRule: "", allowCn: false, isolation: "" });

export const EXIT_RULES: [string, string][] = [
  ["", "不限（只拦截中国大陆出口）"], ["country", "同国家"], ["region", "同省/州"], ["city", "同城市"], ["static", "固定 IP"],
];

/** One-line description: proxy · timezone · language · extras. */
/** `detectedTimezone`: the confirmed exit zone, shown instead of "follows proxy" once known. */
export function environmentSummary(e: EnvironmentInput & { detectedTimezone?: string | null }): string {
  const parts: string[] = [];
  if (e.node) {
    const host = e.node.host.includes(":") ? `[${e.node.host}]` : e.node.host;
    parts.push(`${e.node.kind.toUpperCase()} ${host}:${e.node.port}`);
  } else parts.push("直连");
  parts.push(e.timezone === "system" ? "系统时区" : !e.timezone || e.timezone === "proxy" ? (e.node ? (e.detectedTimezone ? `时区 ${e.detectedTimezone}` : "时区跟随代理") : "系统时区") : e.timezone);
  const lang = LOCALES.find(([v]) => v === e.locale);
  if (e.locale && lang) parts.push(lang[1]);
  const extras = [e.args.trim() && "启动参数", e.env.trim() && "环境变量"].filter(Boolean);
  if (extras.length) parts.push(extras.join("、"));
  if (e.exitRule) parts.push(`出口：${EXIT_RULES.find(([v]) => v === e.exitRule)?.[1] ?? e.exitRule}`);
  if (e.isolation === "sandboxie") parts.push("沙箱隔离");
  return parts.join(" · ");
}

export function toInput(e: Environment): EnvironmentInput {
  const { name, node, timezone, locale, args, env, lastCheck, exitRule, allowCn, isolation } = e;
  return { name, node, timezone, locale, args, env, lastCheck: lastCheck ?? null, exitRule: exitRule ?? "", allowCn: !!allowCn, isolation: isolation ?? "" };
}
