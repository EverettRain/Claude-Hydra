import type { NodeConfig, NodeKind } from "./api";

export const emptyNode = (): NodeConfig => ({
  kind: "socks5", host: "", port: 0, username: "", password: "", ipVersion: "ipv4", geoChannel: "ipinfo", refreshUrl: "",
});

const KINDS: Record<string, NodeKind> = { http: "http", https: "https", socks5: "socks5", socks5h: "socks5", socks: "socks5" };

/**
 * Splits pasted node text into fields. Accepts
 * `type://host:port:user:pass`, `type://user:pass@host:port`, `host:port:user:pass`,
 * `user:pass@host:port` and `host:port` (IPv6 hosts in brackets).
 * Returns null when the text is just a hostname or bare IPv6 address.
 */
export function parseNodeText(text: string): Partial<NodeConfig> | null {
  let rest = text.trim();
  if (!rest || /\s/.test(rest)) return null;
  const out: Partial<NodeConfig> = {};
  const scheme = /^([a-z0-9]+):\/\//i.exec(rest);
  if (scheme) {
    const kind = KINDS[scheme[1].toLowerCase()];
    if (!kind) return null;
    out.kind = kind;
    rest = rest.slice(scheme[0].length).replace(/\/$/, "");
  }
  const at = rest.lastIndexOf("@");
  if (at > 0) {
    const cred = rest.slice(0, at);
    rest = rest.slice(at + 1);
    const i = cred.indexOf(":");
    out.username = decodeURIComponent(i < 0 ? cred : cred.slice(0, i));
    out.password = i < 0 ? "" : decodeURIComponent(cred.slice(i + 1));
  }
  let host: string;
  let tail: string[];
  if (rest.startsWith("[")) {
    const end = rest.indexOf("]");
    if (end < 0) return null;
    host = rest.slice(1, end);
    tail = rest.slice(end + 1).replace(/^:/, "").split(":");
  } else {
    if (rest.includes("::")) return null; // bare IPv6 address, not a node string
    [host, ...tail] = rest.split(":");
  }
  if (!host || !tail[0] || !/^\d{1,5}$/.test(tail[0])) return null;
  const port = Number(tail[0]);
  if (port < 1 || port > 65535) return null;
  out.host = host;
  out.port = port;
  if (tail.length > 1 && at < 0) {
    out.username = tail[1];
    out.password = tail.slice(2).join(":");
  }
  return out;
}

export function formatNode(n: NodeConfig): string {
  const host = n.host.includes(":") ? `[${n.host}]` : n.host;
  return `${n.kind}://${host}:${n.port}${n.username ? `:${n.username}:${n.password}` : ""}`;
}

export const COMMON_TIMEZONES = [
  "Asia/Shanghai", "Asia/Hong_Kong", "Asia/Taipei", "Asia/Tokyo", "Asia/Seoul", "Asia/Singapore", "Asia/Kolkata", "Asia/Dubai",
  "Europe/London", "Europe/Paris", "Europe/Berlin", "Europe/Amsterdam", "Europe/Moscow",
  "America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles", "America/Toronto", "America/Sao_Paulo",
  "Australia/Sydney", "Pacific/Auckland", "UTC",
];
