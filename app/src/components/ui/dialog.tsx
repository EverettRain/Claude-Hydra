import { X } from "lucide-react";
import { useEffect, useId, useRef, type ReactNode } from "react";
import { cn } from "@/lib/utils";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  className?: string;
}

/** Open dialogs, topmost last; Escape only closes the topmost. */
const stack: string[] = [];

export function Dialog({ open, onClose, title, description, children, footer, className }: DialogProps) {
  const key = useId();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    stack.push(key);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && stack[stack.length - 1] === key) {
        e.stopImmediatePropagation();
        close.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      const i = stack.indexOf(key);
      if (i >= 0) stack.splice(i, 1);
    };
  }, [open, key]);

  if (!open) return null;
  return (
    <div data-dialog-layer className="fixed inset-0 z-50 flex items-center justify-center p-6 pt-14">
      <div className="absolute inset-0 animate-fade-in bg-black/20 backdrop-blur-[2px] dark:bg-black/45" onClick={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={cn("relative flex max-h-full w-full max-w-md animate-pop-in flex-col rounded-2xl bg-card shadow-pop", className)}
      >
        <div className="flex items-start justify-between gap-4 px-5 pb-1 pt-4">
          <div className="min-w-0">
            <h2 className="text-[15px] font-semibold tracking-[-0.01em]">{title}</h2>
            {description && <div className="mt-1 text-[12.5px] leading-5 text-muted">{description}</div>}
          </div>
          <button onClick={onClose} aria-label="关闭" className="-mr-1.5 flex h-[26px] w-[26px] items-center justify-center rounded-[7px] text-muted hover:bg-pill hover:text-fg">
            <X className="h-4 w-4" />
          </button>
        </div>
        {children && <div className="min-h-0 overflow-y-auto px-5 py-3">{children}</div>}
        {footer && <div className="flex justify-end gap-2 px-5 pb-4 pt-2">{footer}</div>}
      </div>
    </div>
  );
}
