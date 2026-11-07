import type React from "react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

export interface MenuEntry {
  label: string;
  icon?: React.ComponentType<{ className?: string }>;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
}

/** A small dropdown anchored to its trigger. Closes on outside click, Escape or a choice. */
export function Menu({ trigger, items }: { trigger: (toggle: () => void) => ReactNode; items: (MenuEntry | "sep")[] }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => ref.current && !ref.current.contains(e.target as Node) && setOpen(false);
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      {trigger(() => setOpen((v) => !v))}
      {open && (
        <div className="absolute right-0 top-full z-40 mt-1 min-w-[168px] animate-pop-in rounded-lg border border-border bg-card p-1 shadow-lg">
          {items.map((item, i) =>
            item === "sep" ? (
              <div key={i} className="my-1 h-px bg-border" />
            ) : (
              <button
                key={item.label}
                disabled={item.disabled}
                onClick={() => {
                  setOpen(false);
                  item.onSelect();
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm disabled:opacity-40",
                  item.danger
                    ? "text-red-500 hover:bg-red-500/10"
                    : "text-foreground hover:bg-gray-100 dark:hover:bg-gray-800",
                )}
              >
                {item.icon && <item.icon className="h-4 w-4 shrink-0 opacity-70" />}
                {item.label}
              </button>
            ),
          )}
        </div>
      )}
    </div>
  );
}
