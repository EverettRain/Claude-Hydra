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
      <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-80 flex-col gap-2">
        {items.map((t) => (
          <div
            key={t.id}
            className={cn(
              "pointer-events-auto flex animate-pop-in items-start gap-2 rounded-lg border bg-card px-3 py-2.5 text-sm shadow-md",
              t.kind === "error" ? "border-red-500/40" : "border-border",
            )}
          >
            {t.kind === "error" ? (
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-red-500" />
            ) : (
              <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" />
            )}
            <span className="selectable whitespace-pre-wrap break-words">{t.text}</span>
          </div>
        ))}
      </div>
    </Ctx.Provider>
  );
}
