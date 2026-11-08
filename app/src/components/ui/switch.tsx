import { cn } from "@/lib/utils";

interface SwitchProps {
  checked: boolean;
  onCheckedChange: (value: boolean) => void;
  disabled?: boolean;
  id?: string;
  label?: string;
}

/** Graphite toggle: dark track when on, quiet grey when off. */
export function Switch({ checked, onCheckedChange, disabled, id, label }: SwitchProps) {
  return (
    <button
      id={id}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "relative inline-flex h-[22px] w-[38px] shrink-0 cursor-pointer items-center rounded-full p-[2px] disabled:cursor-not-allowed disabled:opacity-40",
        checked ? "bg-fg" : "bg-line dark:bg-pill-hover",
      )}
    >
      <span
        className={cn(
          "pointer-events-none block h-[18px] w-[18px] rounded-full bg-white shadow-[0_1px_2px_rgba(0,0,0,.25)] transition-transform duration-200 ease-soft",
          checked ? "translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );
}
