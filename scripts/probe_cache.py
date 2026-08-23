
# -*- coding: utf-8 -*-
import ctypes, struct
k32 = ctypes.windll.kernel32
REL_CACHE = 2
def query(rel):
    len_ = ctypes.c_ulong(0)
    k32.GetLogicalProcessorInformationEx(rel, None, ctypes.byref(len_))
    n = len_.value
    buf = ctypes.create_string_buffer(n)
    ok = k32.GetLogicalProcessorInformationEx(rel, buf, ctypes.byref(len_))
    return buf.raw[:len_.value] if ok else None
raw = query(REL_CACHE)
print('entries', len(raw) // 48)
for i in range(len(raw) // 48):
    relv = struct.unpack_from('<I', raw, i * 48)[0]
    if relv != REL_CACHE:
        continue
    base = i * 48 + 8
    level = raw[base]
    size = struct.unpack_from('<I', raw, base + 4)[0]
    mask = struct.unpack_from('<Q', raw, base + 32)[0]
    group = struct.unpack_from('<H', raw, base + 40)[0]
    print(i, 'level', level, 'size', size, 'group', group, 'mask', hex(mask), 'bits', bin(mask).count('1'))
