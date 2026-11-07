import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export const PALETTE = [
  "rgb(66 133 180)",
  "rgb(76 150 96)",
  "rgb(150 96 180)",
  "rgb(190 80 110)",
  "rgb(60 150 150)",
  "rgb(90 110 200)",
  "rgb(200 150 50)",
  "rgb(80 86 100)",
];

export function badgeLetter(name: string): string {
  const c = Array.from(name).find((ch) => /[\p{L}\p{N}]/u.test(ch)) ?? name.charAt(0);
  return (c ?? "?").toUpperCase();
}

export function relativeTime(ms?: number | null): string {
  if (!ms) return "从未使用";
  const diff = Date.now() - ms;
  const min = Math.round(diff / 60_000);
  if (min < 1) return "刚刚";
  if (min < 60) return `${min} 分钟前`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} 小时前`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d} 天前`;
  return new Date(ms).toLocaleDateString("zh-CN");
}
