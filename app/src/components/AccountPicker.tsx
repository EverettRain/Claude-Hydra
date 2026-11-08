import { Check } from "lucide-react";
import type { Profile } from "@/lib/api";
import { PALETTE, badgeLetter, cn } from "@/lib/utils";

interface Props {
  profiles: Profile[] | null;
  value: string;
  onChange: (id: string) => void;
  /** Account that can't be chosen here (e.g. the other side of a copy). */
  exclude?: string;
  disabled?: boolean;
  label: string;
}

/** Pick one account from a row of pills: avatar, name, running dot. */
export function AccountPicker({ profiles, value, onChange, exclude, disabled, label }: Props) {
  if (!profiles) return <span className="text-[12.5px] text-muted">读取中…</span>;
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1.5">
      {profiles.map((p) => {
        const on = p.id === value;
        const blocked = p.id === exclude;
        return (
          <button
            key={p.id}
            role="radio"
            aria-checked={on}
            disabled={disabled || blocked}
            aria-label={p.pid ? `${p.name}（运行中）` : p.name}
            title={blocked ? "已选为另一侧账号" : p.pid ? "运行中" : undefined}
            onClick={() => onChange(on ? "" : p.id)}
            className={cn(
              "inline-flex h-8 items-center gap-2 rounded-full py-1 pl-1 pr-3 text-[12.5px] disabled:cursor-not-allowed disabled:opacity-35",
              on ? "bg-card font-medium text-fg dark:bg-white/[.1] shadow-[0_0_0_1.5px_rgb(var(--fg)),0_1px_2px_rgba(0,0,0,.08)]" : "bg-pill text-fg-2 hover:bg-pill-hover hover:text-fg",
            )}
          >
            <span className="relative flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold text-white" style={{ background: PALETTE[p.color % PALETTE.length] }}>
              {on ? <Check className="h-3.5 w-3.5" /> : badgeLetter(p.name)}
              {p.pid && <span className="absolute -bottom-px -right-px h-2 w-2 rounded-full border-[1.5px] border-card bg-ok" />}
            </span>
            {p.name}
          </button>
        );
      })}
    </div>
  );
}
