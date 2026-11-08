import { useEffect, useState } from "react";
import { api, errorText, type EnvironmentInput } from "@/lib/api";
import { LOCALES, emptyEnvironment } from "@/lib/environment";
import { NodeConfigForm } from "./NodeConfigForm";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { Input, Select, Textarea } from "./ui/input";

interface Props {
  open: boolean;
  /** Existing environment id, or null to create a new one. */
  id: string | null;
  initial?: EnvironmentInput;
  onClose: () => void;
  onSaved: (id: string) => void;
}

function Line({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[88px_1fr] items-center gap-3 px-4 py-2">
      <span className="text-[12.5px] text-fg-2">{label}</span>
      <div className="flex min-w-0 items-center gap-2">{children}</div>
    </div>
  );
}

/** Create or edit a reusable environment: network, timezone, language and advanced launch options. */
export function EnvironmentDialog({ open, id, initial, onClose, onSaved }: Props) {
  const [env, setEnv] = useState<EnvironmentInput>(emptyEnvironment);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    setEnv(initial ? structuredClone(initial) : emptyEnvironment());
    setError("");
  }, [open, initial]);

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      onSaved(await api.saveEnvironment(id, env));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={id ? "编辑环境配置" : "新建环境配置"}
      description="保存后可在任意账号中选择使用；修改会作用于所有使用它的账号。"
      className="max-w-[600px] bg-bg"
      footer={
        <>
          <Button variant="outline" onClick={onClose}>取消</Button>
          <Button onClick={save} disabled={saving || !env.name.trim()}>{saving ? "保存中…" : "保存"}</Button>
        </>
      }
    >
      <div className="space-y-5">
        <section>
          <div className="group-label">基础</div>
          <div className="list-card">
            <Line label="名称">
              <Input aria-label="环境名称" value={env.name} maxLength={40} autoFocus placeholder="例如：东京住宅 IP" onChange={(e) => setEnv({ ...env, name: e.target.value })} />
            </Line>
            <Line label="窗口语言">
              <Select aria-label="窗口语言" value={env.locale} onChange={(e) => setEnv({ ...env, locale: e.target.value })}>
                {LOCALES.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
              </Select>
            </Line>
          </div>
        </section>

        <section>
          <div className="group-label">网络与时区</div>
          <NodeConfigForm idPrefix="env-node" node={env.node} onNodeChange={(node) => setEnv({ ...env, node })}
            timezone={env.timezone} onTimezoneChange={(timezone) => setEnv({ ...env, timezone })} />
        </section>

        <section>
          <div className="group-label">高级</div>
          <div className="list-card">
            <div className="px-4 py-3">
              <div className="mb-1.5 text-[12.5px] font-medium text-fg-2">额外启动参数</div>
              <Input aria-label="额外启动参数" className="font-mono text-xs" placeholder="--disable-gpu" value={env.args} onChange={(e) => setEnv({ ...env, args: e.target.value })} />
            </div>
            <div className="px-4 py-3">
              <div className="mb-1.5 text-[12.5px] font-medium text-fg-2">环境变量</div>
              <Textarea aria-label="环境变量" rows={3} placeholder={"每行一个：名称=值\n设置代理节点后，代理相关变量由 Hydra 管理"} value={env.env} onChange={(e) => setEnv({ ...env, env: e.target.value })} />
            </div>
          </div>
        </section>

        {error && <p className="selectable whitespace-pre-wrap rounded-xl bg-danger-soft px-4 py-3 text-[12.5px] text-danger">{error}</p>}
      </div>
    </Dialog>
  );
}
