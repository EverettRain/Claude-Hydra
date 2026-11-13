import type React from "react";
import { cn } from "@/lib/utils";

/** Small pill next to an account name. */
export function Tag({ children, tone, title }: { children: React.ReactNode; tone?: "warn" | "env" | "danger"; title?: string }) {
  return (
    <span
      title={title ?? (tone === "env" ? "环境配置" : undefined)}
      className={cn(
        "inline-flex h-[18px] max-w-[160px] shrink-0 items-center truncate rounded-full px-1.5 text-[10.5px] font-medium",
        tone === "danger" ? "bg-danger-soft text-danger" : tone === "warn" ? "bg-warn-soft text-warn" : tone === "env" ? "bg-brand-soft text-brand" : "bg-pill text-fg-2",
      )}
    >
      {children}
    </span>
  );
}
