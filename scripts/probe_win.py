
# -*- coding: utf-8 -*-
import ctypes, struct
k32 = ctypes.windll.kernel32

REL_CORE = 0
REL_CACHE = 1
REL_NUMA = 3
REL_GROUP = 4

def query(rel):
    len_ = ctypes.c_ulong(0)
    k32.GetLogicalProcessorInformationEx(rel, None, ctypes.byref(len_))
    n = len_.value
    buf = ctypes.create_string_buffer(n)
    ok = k32.GetLogicalProcessorInformationEx(rel, buf, ctypes.byref(len_))
    if not ok:
        return None
    return buf.raw[:len_.value]

for rel, name in [(REL_CORE, 'cores'), (REL_CACHE, 'cache')]:
    raw = query(rel)
    if raw is None:
        print(name, 'FAIL')
        continue
    # 按不同步长解析，统计 relationship==rel 的条目数
    for stride in [48, 56, 72, 80]:
        cnt = 0
        for i in range(len(raw) // stride):
            relv = struct.unpack_from('<I', raw, i * stride)[0]
            if relv == rel:
                cnt += 1
        print(name, 'stride', stride, '-> entries', cnt, 'buffer', len(raw))
    print()
