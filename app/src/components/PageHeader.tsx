import type React from "react";

interface PageHeaderProps {
  title: string;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
}

export function PageHeader({ title, subtitle, actions }: PageHeaderProps) {
  return (
    <header className="sticky top-0 z-10 flex min-h-[52px] items-center justify-between gap-4 bg-background px-6 pb-3">
      <div className="min-w-0">
        <h1 className="truncate text-3xl font-semibold tracking-normal text-foreground">{title}</h1>
        {subtitle && <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}
