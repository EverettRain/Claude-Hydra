import { cn } from "@/lib/utils";

interface SwitchProps {
  checked: boolean;
  onCheckedChange: (value: boolean) => void;
  disabled?: boolean;
  id?: string;
  label?: string;
}

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
        "relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-blue-500 dark:bg-blue-600" : "bg-gray-200 dark:bg-gray-700",
      )}
    >
      <span
        className={cn(
          "pointer-events-none block h-4 w-4 rounded-full bg-white shadow-sm transition-transform duration-150",
          checked ? "translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );
}
