import { useEffect, useRef, useState, type RefObject } from "react";
import { cn } from "@/lib/utils";

const MIN_THUMB = 32;
const IDLE_MS = 1200;

/**
 * A scrollbar for the scroll container `target` (the system's are hidden app-wide): a thin thumb
 * along the right edge of the container's positioned parent. It shows while the content scrolls
 * or the pointer is over it, follows the content as it grows (pages load as one scrolls), can be
 * dragged, and a click on its track moves one screen toward that point.
 */
export function Scrollbar({ target }: { target: RefObject<HTMLElement | null> }) {
  const track = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<{ size: number; offset: number } | null>(null);
  const [active, setActive] = useState(false);
  const [hover, setHover] = useState(false);
  const [dragging, setDragging] = useState(false);
  const idle = useRef<number>();
  const drag = useRef<{ y: number; top: number } | null>(null);

  useEffect(() => {
    const el = target.current;
    if (!el) return;
    const measure = () => {
      const height = track.current?.clientHeight ?? el.clientHeight;
      if (el.scrollHeight <= el.clientHeight + 1 || !height) { setThumb(null); return; }
      const size = Math.max(MIN_THUMB, (el.clientHeight / el.scrollHeight) * height);
      const offset = (el.scrollTop / (el.scrollHeight - el.clientHeight)) * (height - size);
      setThumb({ size, offset });
    };
    const scrolled = () => {
      measure();
      setActive(true);
      window.clearTimeout(idle.current);
      idle.current = window.setTimeout(() => setActive(false), IDLE_MS);
    };
    // The container's own size and its content's height (pages are appended while reading).
    const resized = new ResizeObserver(measure);
    resized.observe(el);
    for (const child of Array.from(el.children)) resized.observe(child);
    const added = new MutationObserver(() => {
      for (const child of Array.from(el.children)) resized.observe(child);
      measure();
    });
    added.observe(el, { childList: true });
    el.addEventListener("scroll", scrolled, { passive: true });
    measure();
    return () => {
      el.removeEventListener("scroll", scrolled);
      resized.disconnect();
      added.disconnect();
      window.clearTimeout(idle.current);
    };
  }, [target]);

  if (!thumb) return null;
  const el = target.current;
  const shown = active || hover || dragging;

  const startDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { y: e.clientY, top: el.scrollTop };
    setDragging(true);
  };
  const moveDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    const from = drag.current;
    const height = track.current?.clientHeight ?? 0;
    if (!from || !el || height <= thumb.size) return;
    const perPixel = (el.scrollHeight - el.clientHeight) / (height - thumb.size);
    el.scrollTop = from.top + (e.clientY - from.y) * perPixel;
  };
  const endDrag = () => { drag.current = null; setDragging(false); };
  const page = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!el || e.target !== e.currentTarget) return;
    const y = e.clientY - e.currentTarget.getBoundingClientRect().top;
    el.scrollBy({ top: (y < thumb.offset ? -1 : 1) * el.clientHeight * 0.9, behavior: "smooth" });
  };

  return (
    <div
      ref={track}
      aria-hidden
      className="group absolute bottom-1 right-0.5 top-1 z-10 w-3"
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onPointerDown={page}
    >
      <div
        style={{ height: thumb.size, transform: `translateY(${thumb.offset}px)` }}
        className={cn(
          "absolute right-0.5 w-[5px] rounded-full bg-fg/25 transition-[opacity,width,background-color] duration-200",
          "group-hover:w-[7px] group-hover:bg-fg/40",
          dragging && "w-[7px] bg-fg/50",
          shown ? "opacity-100" : "opacity-0",
        )}
        onPointerDown={startDrag}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      />
    </div>
  );
}
