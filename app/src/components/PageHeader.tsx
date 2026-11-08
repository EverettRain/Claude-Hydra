import type React from "react";

interface PageHeaderProps {
  /** Kept for accessibility; the active tab already shows the page name. */
  title: string;
  subtitle?: React.ReactNode;
  actions?: React.ReactNode;
}

/** One quiet intro line under the tab bar, with page actions on the right. */
export function PageHeader({ title, subtitle, actions }: PageHeaderProps) {
  return (
    <header className="flex min-h-[36px] items-center justify-between gap-4 px-5 pb-3">
      <h1 className="sr-only">{title}</h1>
      <p className="min-w-0 text-[13px] leading-5 text-fg-2">{subtitle}</p>
      {actions && <div className="flex shrink-0 items-center gap-1.5">{actions}</div>}
    </header>
  );
}

/** Content column shared by every page. */
export function Page({ children }: { children: React.ReactNode }) {
  return <div className="view-in w-full max-w-[860px] pb-8">{children}</div>;
}

/** A labelled group: small uppercase label above a grouped list card. */
export function Group({ label, action, children, className }: { label?: string; action?: React.ReactNode; children: React.ReactNode; className?: string }) {
  return (
    <section className={className}>
      {(label || action) && (
        <div className="flex items-end justify-between">
          {label ? <div className="group-label">{label}</div> : <span />}
          {action && <div className="pb-1">{action}</div>}
        </div>
      )}
      <div className="list-card">{children}</div>
    </section>
  );
}

/** A settings-style row inside a list card: label and hint on the left, control on the right. */
export function Row({ label, hint, children }: { label: React.ReactNode; hint?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 px-4 py-3">
      <div className="min-w-0">
        <div className="text-[13px] font-medium">{label}</div>
        {hint && <div className="mt-0.5 text-[12px] leading-[18px] text-muted">{hint}</div>}
      </div>
      {children !== undefined && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}
