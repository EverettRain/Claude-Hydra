import json, shutil, io, sys
p = r"D:\ClaudeHydra\settings.json"
raw = open(p, "rb").read()
txt = raw.decode("utf-8")
data = json.loads(txt)
old = txt
anchor = '"accounts": {\n    "Account 2": {\n'
assert anchor in txt, "anchor missing"
if '"environment"' in txt.split('"environments"')[0]:
    print("already bound")
else:
    txt = txt.replace(anchor, anchor + '      "environment": "env-1a1cef07923",\n', 1)
    open(p, "wb").write(txt.encode("utf-8"))
    print("patched")
d2 = json.loads(open(p, "rb").read().decode("utf-8"))
print("accounts:", json.dumps(d2["accounts"], ensure_ascii=False))
print("env keys:", list(d2["environments"].keys()))
