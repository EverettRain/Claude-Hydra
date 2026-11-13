import struct, sys

def u16(b, o): return struct.unpack_from('<H', b, o)[0]
def u32(b, o): return struct.unpack_from('<I', b, o)[0]

def imports(path):
    data = open(path, 'rb').read()
    e_lfanew = u32(data, 0x3C)
    assert data[e_lfanew:e_lfanew+4] == b'PE\0\0'
    coff = e_lfanew + 4
    nsec = u16(data, coff + 2)
    optsize = u16(data, coff + 16)
    opt = coff + 20
    magic = u16(data, opt)
    plus = magic == 0x20b
    ddoff = opt + (112 if plus else 96)
    secs = []
    secoff = opt + optsize
    for i in range(nsec):
        base = secoff + i * 40
        name = data[base:base+8].rstrip(b'\0').decode('latin1')
        vsize = u32(data, base + 8)
        vaddr = u32(data, base + 12)
        rawsize = u32(data, base + 16)
        rawptr = u32(data, base + 20)
        secs.append((name, vaddr, vsize, rawptr, rawsize))
    def r2o(rva):
        for name, vaddr, vsize, rawptr, rawsize in secs:
            size = max(vsize, rawsize)
            if vaddr <= rva < vaddr + size:
                return rawptr + (rva - vaddr)
        return None
    def cstr(rva):
        off = r2o(rva)
        if off is None: return '?'
        end = data.index(b'\0', off)
        return data[off:end].decode('latin1')
    imp_rva = u32(data, ddoff + 8)
    if not imp_rva: return []
    off = r2o(imp_rva)
    out = []
    while True:
        dll = cstr(u32(data, off + 12))
        thunk = u32(data, off)
        if not dll and not thunk: break
        if thunk:
            t = r2o(thunk)
            while True:
                v = struct.unpack_from('<Q' if plus else '<I', data, t)[0]
                if not v: break
                if not (v >> (63 if plus else 31)):
                    out.append((dll, cstr((v & 0x7fffffff) + 2)))
                t += 8 if plus else 4
        off += 20
    return out

for path in sys.argv[1:]:
    print('==', path)
    for dll, fn in imports(path):
        print('   ', dll, '->', fn)
