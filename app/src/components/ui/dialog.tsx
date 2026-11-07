import { X } from "lucide-react";
import { useEffect, type ReactNode } from "react";
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

export function Dialog({ open, onClose, title, description, children, footer, className }: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6 pt-12">
      <div className="absolute inset-0 animate-fade-in bg-black/40 dark:bg-black/60" onClick={onClose} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={cn(
          "relative flex max-h-full w-full max-w-md animate-pop-in flex-col rounded-xl border border-border bg-card shadow-lg",
          className,
        )}
      >
        <div className="flex items-start justify-between gap-4 px-5 pb-2 pt-5">
          <div className="min-w-0">
            <h2 className="text-lg font-semibold">{title}</h2>
            {description && <div className="mt-1 text-sm text-muted-foreground">{description}</div>}
          </div>
          <button
            onClick={onClose}
            aria-label="关闭"
            className="-mr-1 rounded-md p-1 text-muted-foreground hover:bg-gray-100 hover:text-foreground dark:hover:bg-gray-800"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {children && <div className="min-h-0 overflow-y-auto px-5 py-3">{children}</div>}
        {footer && <div className="flex justify-end gap-2 border-t border-border px-5 py-3">{footer}</div>}
      </div>
    </div>
  );
}
