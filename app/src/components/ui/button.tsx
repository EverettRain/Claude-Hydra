import * as React from "react";
import { cn } from "@/lib/utils";

const variants = {
  /** Graphite primary: the one action that matters on a surface. */
  default: "bg-fg text-card hover:bg-fg/85",
  /** Indigo, for actions that change state outside the app (rare). */
  brand: "bg-brand text-brand-fg hover:bg-brand/90",
  destructive: "bg-danger text-white hover:bg-danger/90 dark:text-bg",
  /** Soft pill, the default secondary button. */
  outline: "bg-pill text-fg hover:bg-pill-hover",
  ghost: "text-muted hover:bg-pill hover:text-fg",
  success: "bg-ok text-white hover:bg-ok/90",
} as const;

const sizes = {
  default: "h-8 px-3.5 text-[13px]",
  sm: "h-7 px-2.5 text-[12.5px]",
  icon: "h-8 w-8",
  iconSm: "h-[26px] w-[26px] rounded-[7px]",
} as const;

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: keyof typeof variants;
  size?: keyof typeof sizes;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "default", type = "button", ...props }, ref) => (
    <button
      ref={ref}
      type={type}
      className={cn(
        "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg font-medium disabled:pointer-events-none disabled:opacity-40 [&_svg]:h-[15px] [&_svg]:w-[15px]",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
