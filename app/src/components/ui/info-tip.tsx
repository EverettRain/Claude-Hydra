import { Info } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

const WIDTH = 280;

/**
 * An (i) that shows `text` on hover or click. The note is placed on the page itself, outside the
 * dialog it sits in: a dialog scrolls and animates, which would clip or shift it. The `title`
 * attribute is not used, since the system web view may never show it.
 */
export function InfoTip({ text, label = "说明" }: { text: string; label?: string }) {
  const icon = useRef<HTMLButtonElement>(null);
  const note = useRef<HTMLDivElement>(null);
  // Opened by a click it stays until dismissed; opened by hovering it follows the pointer.
  const [open, setOpen] = useState<"hover" | "click" | null>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !icon.current) return;
    const r = icon.current.getBoundingClientRect();
    const left = Math.max(8, Math.min(r.left - 8, window.innerWidth - WIDTH - 8));
    setPlace({ left, top: r.bottom + 6 });
  }, [open]);

  useEffect(() => {
    if (open !== "click") return;
    const away = (e: MouseEvent) => {
      if (!icon.current?.contains(e.target as Node) && !note.current?.contains(e.target as Node)) setOpen(null);
    };
    const key = (e: KeyboardEvent) => {
      // Closes the note, not the dialog it belongs to.
      if (e.key === "Escape") { e.stopImmediatePropagation(); setOpen(null); }
    };
    const scrolled = () => setOpen(null);
    document.addEventListener("mousedown", away);
    window.addEventListener("keydown", key, true);
    window.addEventListener("scroll", scrolled, true);
    return () => {
      document.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("scroll", scrolled, true);
    };
  }, [open]);

  return (
    <>
      <button
        ref={icon}
        type="button"
        aria-label={label}
        aria-expanded={!!open}
        className="flex h-4 w-4 items-center justify-center rounded text-faint hover:text-muted focus-visible:text-muted active:!scale-100"
        onClick={() => setOpen((o) => (o === "click" ? null : "click"))}
        onMouseEnter={() => setOpen((o) => o ?? "hover")}
        onMouseLeave={() => setOpen((o) => (o === "hover" ? null : o))}
      >
        <Info className="h-3 w-3" />
      </button>
      {open && place && createPortal(
        <div
          ref={note}
          role="tooltip"
          style={{ left: place.left, top: place.top, width: WIDTH }}
          className="selectable fixed z-[60] animate-fade-in rounded-lg bg-card px-3 py-2 text-[12px] leading-[1.6] text-fg-2 shadow-pop"
        >
          {text}
        </div>,
        document.body,
      )}
    </>
  );
}
