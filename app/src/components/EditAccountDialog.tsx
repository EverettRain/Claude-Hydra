import { Check } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type Profile, type ProfileOptions } from "@/lib/api";
import { PALETTE, badgeLetter, cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { Input, Label, Textarea } from "./ui/input";
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
      toast(profile.pid ? "已保存。启动参数和环境变量在重启该账号后生效。" : "已保存。");
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
      description={profile.isDefault ? "默认账号使用 Claude 原有的数据目录。" : `数据目录：${profile.path}`}
      className="max-w-lg"
      footer={
        <>
          <Button variant="outline" onClick={onClose}>
            取消
          </Button>
          <Button onClick={save} disabled={!opts || saving}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </>
      }
    >
      {!opts ? (
        <p className="py-6 text-center text-sm text-muted-foreground">{error ?? "读取中…"}</p>
      ) : (
        <div className="space-y-5">
          <div className="flex items-center gap-4">
            <div
              className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl text-lg font-semibold text-white"
              style={{ background: PALETTE[color] }}
            >
              {badgeLetter(opts.name || profile.id)}
            </div>
            <div className="flex-1 space-y-1.5">
              <Label htmlFor="acc-name">名称</Label>
              <Input
                id="acc-name"
                value={opts.name}
                maxLength={40}
                autoFocus
                onChange={(e) => setOpts({ ...opts, name: e.target.value })}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>颜色</Label>
            <div className="flex flex-wrap items-center gap-2">
              <button
                onClick={() => setOpts({ ...opts, color: -1 })}
                className={cn(
                  "h-8 rounded-lg border px-2.5 text-xs",
                  opts.color < 0 ? "border-blue-500 text-blue-500" : "border-border text-muted-foreground hover:text-foreground",
                )}
              >
                自动
              </button>
              {PALETTE.map((c, i) => (
                <button
                  key={c}
                  aria-label={`颜色 ${i + 1}`}
                  onClick={() => setOpts({ ...opts, color: i })}
                  className="flex h-8 w-8 items-center justify-center rounded-lg ring-offset-2 ring-offset-card"
                  style={{ background: c, boxShadow: opts.color === i ? `0 0 0 2px hsl(var(--card)), 0 0 0 4px ${c}` : undefined }}
                >
                  {opts.color === i && <Check className="h-4 w-4 text-white" />}
                </button>
              ))}
            </div>
          </div>

          <div className="flex items-start justify-between gap-4 rounded-lg border border-border p-3">
            <div>
              <Label htmlFor="acc-keep">保持运行</Label>
              <p className="mt-0.5 text-xs text-muted-foreground">
                Claude 因自动更新、注销或重启而关闭时自动重新打开；手动退出不会触发。
              </p>
            </div>
            <Switch id="acc-keep" checked={opts.keepRunning} onCheckedChange={(v) => setOpts({ ...opts, keepRunning: v })} />
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="acc-args">额外启动参数</Label>
            <Input
              id="acc-args"
              className="font-mono text-xs"
              placeholder="--proxy-server=http://127.0.0.1:7890"
              value={opts.args}
              disabled={profile.isDefault}
              onChange={(e) => setOpts({ ...opts, args: e.target.value })}
            />
            {profile.isDefault && <p className="text-xs text-muted-foreground">商店版的默认账号通过系统启动，不接受额外参数。</p>}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="acc-env">环境变量</Label>
            <Textarea
              id="acc-env"
              rows={4}
              placeholder={"每行一个：名称=值\nHTTPS_PROXY=http://127.0.0.1:7890"}
              value={opts.env}
              onChange={(e) => setOpts({ ...opts, env: e.target.value })}
            />
          </div>

          {error && <p className="selectable whitespace-pre-wrap text-sm text-red-500">{error}</p>}
        </div>
      )}
    </Dialog>
  );
}
