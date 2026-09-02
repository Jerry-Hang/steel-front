# -*- coding: utf-8 -*-
import io, re

def patch(path, adds, dels):
    s = io.open(path, encoding='utf-8').read()
    for sym in dels:
        s = re.sub(r'\n\s*#\[allow\(dead_code\)\]\s*\n','\n', s)
    changed = 0
    for sym, note in adds:
        if s.find('#[allow(dead_code)] // ' + sym) >= 0:
            continue
        # 匹配声明前一行
        pat = re.compile(r'(\n(?:\\s*#\[[^\\]]*\\]\s*\n)?)(\\s*(?:pub\\s+)?(?:fn|const|static|struct|enum|type|trait|impl) ' + re.escape(sym) + r'\\b)')
        m = pat.search(s)
        if m:
            s = s[:m.start()] + '\n' + m.group(1).split('\\n')[0] + '\n#[allow(dead_code)] // ' + note + '\n' + m.group(2) + s[m.end():]
            changed += 1
    io.open(path, 'w', encoding='utf-8', newline='').write(s)
    return changed

# assets.rs：GDI 全套（备用管线）+ OBJ 解析器（备用）→ allow；79 尾分号；359 static 改
r1 = patch('src/engine/assets.rs',
    [('GpStatus','规划特性保留：GDI+ 资产管线（备用）'),('Startup','同上：GDI+ 备用'),('TOKEN','同上：GDI+ 备用'),('load_rgba','同上：GDI+ 备用'),('parse_obj','规划特性保留：OBJ 解析器（备用导入）'),('GdiplusStartup','GDI+ 备用'),('GdipLoadImageFromFile','GDI+ 备用'),('GdipGetImageWidth','GDI+ 备用'),('GdipGetImageHeight','GDI+ 备用'),('GdipBitmapLockBits','GDI+ 备用'),('GdipBitmapUnlockBits','GDI+ 备用'),('GdipDisposeImage','GDI+ 备用'),('GdiplusShutdown','GDI+ 备用')],
    [])
print('assets patched', r1)
