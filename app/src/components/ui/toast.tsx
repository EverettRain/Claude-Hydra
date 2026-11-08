import { AlertCircle, CheckCircle2 } from "lucide-react";
import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

type Kind = "success" | "error";
interface Toast {
  id: number;
  kind: Kind;
  text: string;
}

const Ctx = createContext<(text: string, kind?: Kind) => void>(() => {});

export function useToast() {
  return useContext(Ctx);
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Toast[]>([]);
  const push = useCallback((text: string, kind: Kind = "success") => {
    const id = Date.now() + Math.random();
    setItems((v) => [...v.slice(-3), { id, kind, text }]);
    setTimeout(() => setItems((v) => v.filter((t) => t.id !== id)), kind === "error" ? 6000 : 2600);
  }, []);
  return (
    <Ctx.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed inset-x-0 bottom-5 z-[60] flex flex-col items-center gap-2 px-4">
        {items.map((t) => (
          <div
            key={t.id}
            style={{ background: "var(--pop)" }}
            className="pointer-events-auto flex max-w-md animate-pop-in items-start gap-2 rounded-xl px-3.5 py-2.5 text-[13px] shadow-pop backdrop-blur-md"
          >
            {t.kind === "error" ? (
              <AlertCircle className="mt-px h-4 w-4 shrink-0 text-danger" />
            ) : (
              <CheckCircle2 className={cn("mt-px h-4 w-4 shrink-0 text-ok")} />
            )}
            <span className="selectable whitespace-pre-wrap break-words">{t.text}</span>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}
