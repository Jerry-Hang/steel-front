# -*- coding: utf-8 -*-
import io, re
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# 1) 闭包整体替换（找 "let mem_alloc = |buf" 到 "        };" 的匹配块——手工按行）
lines = s.split('\n')
out = []
i = 0
while i < len(lines):
    ln = lines[i]
    if 'let mem_alloc = |buf: vk::Buffer|' in ln:
        # 跳过到闭包结束 "        };"
        while i < len(lines) and '        };' not in lines[i]:
            i += 1
        # 现在 i 在闭包结束行
        # 输出辅助开头 + 结束
        out.append('        let mem_alloc = |buf: vk::Buffer| -> Result<(vk::DeviceMemory, u64), String> {')
        out.append('            mem_alloc_ex(dev, instance, phys, buf, true, true)')
        out.append('        };')
        i += 1
        continue
    # 找其它内联分配块： "let (X, X) = {" ... "};" 中含 create_buffer B8 accel else —— 用正则粗替换整块为 helper 调用
    m = re.match(r'^(\s*)let \((x{1,2}buf|x{1,2}mem|hbo|hbuf|hmem|vmem|imem|tmem|smem|asmem|inst_buf, inst_mem|asbuf, asmem|tbuf, tmem|sbuf, smem|hbuf, hmem)[^=]*\) = \{', ln)
    if m and i + 1 < len(lines):
        # 查看后续 5 行是否为内联块（create_buffer + allocate_memory + bind）
        chunk = '\n'.join(lines[i:i+8])
        if 'create_buffer' in chunk and 'allocate_memory' in chunk:
            # 判断用途（by name）
            name = m.group(1)
            # 跳过直到 "        };"
            while i < len(lines) and '        };' not in lines[i]:
                i += 1
            i += 1
            # 输出通用调用（为简化：直接不对齐，保留原块但补 AI flag —— 由后处理加）
            out.append('        /*INLINE_ALLOC*/')
            continue
    out.append(ln)
    i += 1
s = '\n'.join(out)
# 简化：第二遍把 INLINE_ALLOC 标记断掉？——直接放弃复杂化，改用最稳：就在现有内联块上加 fl flag
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('step1 done (闭包已换)')
