import json, hashlib, sys

p = r'D:\ClaudeHydra\settings.json'
raw = open(p, 'rb').read()
s = json.loads(raw.decode('utf-8'))

print('--- box names (sandbox = Hydra_ + sha256(id)[:10]) ---')
for a in s['accounts']:
    print('  %-12s -> Hydra_%s' % (a, hashlib.sha256(a.encode()).hexdigest()[:10]))

print('--- accounts block (raw) ---')
i = raw.find(b'"accounts"')
print(raw[i:i + 400].decode('utf-8'))

print('--- account fields of interest ---')
for a, acc in s['accounts'].items():
    print('  %-12s environment=%r node=%r proxyEndpoint=%r relayPort=%r' % (
        a, acc.get('environment'), bool(acc.get('node')),
        acc.get('proxyEndpoint'), acc.get('relayPort')))

print('--- non-ascii strings inside env ---')
def walk(o, pre=''):
    if isinstance(o, dict):
        for k, v in o.items():
            walk(v, pre + '/' + k)
    elif isinstance(o, list):
        for n, v in enumerate(o):
            walk(v, pre + '[%d]' % n)
    elif isinstance(o, str) and any(ord(c) > 127 for c in o):
        print('  %s = %r  (codepoints: %s)' % (pre, o, ' '.join('U+%04X' % ord(c) for c in o[:12])))

for eid, env in s['environments'].items():
    walk(env, eid)

print('--- U+FFFD in file? ---', raw.count(b'\xef\xbf\xbd'))
print('--- lone surrogates / control chars? ---')
txt = raw.decode('utf-8')
bad = [(n, repr(txt[n - 20:n + 20])) for n, c in enumerate(txt) if ord(c) < 0x20 and c not in '\r\n\t']
print('  count=%d' % len(bad), bad[:5])
print('--- top-level keys ---', list(s.keys()))
print('--- requireEnvironment ---', s.get('requireEnvironment'))
