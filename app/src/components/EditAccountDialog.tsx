import { Check } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type Profile, type ProfileOptions } from "@/lib/api";
import { PALETTE, badgeLetter, cn } from "@/lib/utils";
import { EnvironmentPicker } from "./EnvironmentPicker";
import { exitText } from "./NodeConfigForm";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { Input } from "./ui/input";
import { Switch } from "./ui/switch";
import { useToast } from "./ui/toast";

interface Props {
  profile: Profile | null;
  onClose: () => void;
  onSaved: () => void;
}

export function EditAccountDialog({ profile, onClose, onSaved }: Props) {
  const toast = useToast();
  const [opts, setOpts] = useState<ProfileOptions | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setOpts(null);
    setError(null);
    if (profile) api.options(profile.id).then(setOpts).catch((e) => setError(errorText(e)));
  }, [profile]);

  if (!profile) return null;
  const color = opts && opts.color >= 0 ? opts.color : profile.color;

  const save = async () => {
    if (!opts) return;
    setSaving(true);
    setError(null);
    try {
      await api.saveOptions(profile.id, opts);
      toast(profile.pid ? "已保存。环境配置在重启该账号后完整生效。" : "已保存。");
      onSaved();
      onClose();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title="编辑账号"
      description={profile.isDefault ? "默认账号使用 Claude 原有的数据目录。" : <span className="selectable break-all font-mono text-[11px]">{profile.path}</span>}
      className="max-w-[560px] bg-bg"
      footer={
        <>
          <Button variant="outline" onClick={onClose}>取消</Button>
          <Button onClick={save} disabled={!opts || saving}>{saving ? "保存中…" : "保存"}</Button>
        </>
      }
    >
      {!opts ? (
        <p className="py-8 text-center text-[13px] text-muted">{error ?? "读取中…"}</p>
      ) : (
        <div className="space-y-5">
          <section>
            <div className="group-label">基础</div>
            <div className="list-card">
              <div className="flex items-center gap-3 px-4 py-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[11px] text-[15px] font-semibold text-white" style={{ background: PALETTE[color] }}>
                  {badgeLetter(opts.name || profile.id)}
                </div>
                <Input id="acc-name" aria-label="名称" value={opts.name} maxLength={40} autoFocus onChange={(e) => setOpts({ ...opts, name: e.target.value })} />
              </div>
              <div className="flex flex-wrap items-center gap-2 px-4 py-3">
                <span className="mr-1 text-[12.5px] text-fg-2">颜色</span>
                <button onClick={() => setOpts({ ...opts, color: -1 })} className={cn("h-6 rounded-md px-2 text-[12px]", opts.color < 0 ? "bg-fg text-card" : "bg-pill text-muted hover:text-fg")}>
                  自动
                </button>
                {PALETTE.map((c, i) => (
                  <button
                    key={c}
                    aria-label={`颜色 ${i + 1}`}
                    onClick={() => setOpts({ ...opts, color: i })}
                    className="flex h-6 w-6 items-center justify-center rounded-full"
                    style={{ background: c, boxShadow: opts.color === i ? `0 0 0 2px rgb(var(--card)), 0 0 0 3.5px ${c}` : undefined }}
                  >
                    {opts.color === i && <Check className="h-3 w-3 text-white" />}
                  </button>
                ))}
              </div>
              <div className="flex items-center justify-between gap-4 px-4 py-3">
                <div>
                  <label htmlFor="acc-keep" className="text-[13px] font-medium">保持运行</label>
                  <p className="mt-0.5 text-[11.5px] text-muted">Claude 因自动更新、注销或重启而关闭时自动重新打开；手动退出不会触发。</p>
                </div>
                <Switch id="acc-keep" checked={opts.keepRunning} onCheckedChange={(v) => setOpts({ ...opts, keepRunning: v })} />
              </div>
            </div>
          </section>

          <section>
            <div className="group-label">环境配置</div>
            <EnvironmentPicker value={opts.environment} onChange={(environment) => setOpts({ ...opts, environment })} />
            {opts.lastExit && <p className="mt-1.5 px-1 text-[11.5px] text-muted">上次启动检测：{exitText(opts.lastExit)}</p>}
          </section>

          {error && <p className="selectable whitespace-pre-wrap rounded-xl bg-danger-soft px-4 py-3 text-[12.5px] text-danger">{error}</p>}
        </div>
      )}
    </Dialog>
  );
}
