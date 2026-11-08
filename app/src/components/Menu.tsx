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
        <div
          style={{ background: "var(--pop)" }}
          className="absolute right-0 top-full z-40 mt-1.5 min-w-[176px] origin-top-right animate-pop-in rounded-[10px] p-1 shadow-pop backdrop-blur-md"
        >
          {items.map((item, i) =>
            item === "sep" ? (
              <div key={i} className="mx-2 my-1 h-px bg-line-2" />
            ) : (
              <button
                key={item.label}
                disabled={item.disabled}
                onClick={() => {
                  setOpen(false);
                  item.onSelect();
                }}
                className={cn(
                  "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] disabled:opacity-35",
                  item.danger ? "text-danger hover:bg-danger-soft" : "text-fg hover:bg-pill",
                )}
              >
                {item.icon && <item.icon className="h-[15px] w-[15px] shrink-0 opacity-60" />}
                {item.label}
              </button>
            ),
          )}
        </div>
      )}
    </div>
  );
}
