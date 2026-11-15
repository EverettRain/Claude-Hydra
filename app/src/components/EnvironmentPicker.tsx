import { Globe2, Plus, WifiOff } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, errorText, type Environment } from "@/lib/api";
import { environmentSummary } from "@/lib/environment";
import { cn } from "@/lib/utils";
import { EnvironmentDialog } from "./EnvironmentDialog";

interface Props {
  value: string;
  onChange: (id: string) => void;
  disabled?: boolean;
  /** Only environments with a proxy node can be chosen (container accounts). */
  requireNode?: boolean;
  /** Environments this account cannot use, with the reason shown in their place. */
  unusable?: Record<string, string>;
}

function Option({ on, icon: Icon, title, sub, onClick, disabled }: { on: boolean; icon: React.ComponentType<{ className?: string }>; title: string; sub: string; onClick: () => void; disabled?: boolean }) {
  return (
    <button
      role="radio"
      aria-checked={on}
      disabled={disabled}
      onClick={onClick}
      className={cn("flex w-full items-center gap-3 px-4 py-2.5 text-left active:!scale-100 disabled:opacity-50", on ? "bg-pill/70" : "hover:bg-card-2")}
    >
      <span className={cn("flex h-4 w-4 shrink-0 items-center justify-center rounded-full border", on ? "border-fg bg-fg" : "border-faint")}>
        {on && <span className="h-1.5 w-1.5 rounded-full bg-card" />}
      </span>
      <Icon className="h-[15px] w-[15px] shrink-0 text-muted" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-medium">{title}</span>
        <span className="block truncate text-[11.5px] text-muted">{sub}</span>
      </span>
    </button>
  );
}

/** Choose which saved environment an account uses, or create one on the spot. */
export function EnvironmentPicker({ value, onChange, disabled, requireNode, unusable }: Props) {
  const [envs, setEnvs] = useState<Environment[] | null>(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);

  const load = useCallback(async () => {
    try { setEnvs(await api.listEnvironments()); setError(""); }
    catch (e) { setError(errorText(e)); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  return (
    <>
      <div role="radiogroup" aria-label="环境配置" className="list-card">
        {!requireNode && <Option on={!value} icon={WifiOff} title="不使用环境配置" sub="直连网络 · 系统时区 · 默认启动参数" onClick={() => onChange("")} disabled={disabled} />}
        {envs?.map((e) => {
          const why = requireNode && !e.node ? "没有代理节点，容器不能使用" : unusable?.[e.id];
          return <Option key={e.id} on={value === e.id} icon={why ? WifiOff : Globe2} title={e.name} sub={why ?? environmentSummary(e)} onClick={() => onChange(e.id)} disabled={disabled || !!why} />;
        })}
        <button onClick={() => setCreating(true)} disabled={disabled} className="flex w-full items-center gap-2 px-4 py-2.5 text-[12.5px] text-muted hover:bg-card-2 hover:text-fg active:!scale-100">
          <Plus className="h-3.5 w-3.5" />
          新建环境配置…
        </button>
      </div>
      {/* Going without one is a choice that should not pass unnoticed once proxies are set up. */}
      {!value && !requireNode && envs?.some((e) => e.node) && (
        <p className="mt-1.5 px-1 text-[11.5px] text-warn">
          未使用环境配置：这个账号直连网络、使用系统时区，网站看到的是本机的真实 IP。
          {envs.filter((e) => e.node).every((e) => unusable?.[e.id]) && "它用不了带代理的环境；需要经代理使用时，请添加账号。"}
        </p>
      )}
      {envs === null && !error && <p className="mt-1.5 px-1 text-[11.5px] text-muted">正在读取环境配置…</p>}
      {error && <p className="mt-1.5 px-1 text-[11.5px] text-danger">{error}</p>}
      <EnvironmentDialog open={creating} id={null} onClose={() => setCreating(false)} onSaved={async (id) => { setCreating(false); await load(); onChange(id); }} />
    </>
  );
}
