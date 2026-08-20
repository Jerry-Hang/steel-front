# -*- coding: utf-8 -*-
path = r'D:\\Rust\\steel-front\\tools\\fpfont\\fp8\\fusion-pixel-8px-proportional-zh_hans.bdf'
data = open(path, 'rb').read().decode('utf-8', errors='replace')
chars = {}
enc = None
rows = []
in_bitmap = False
for line in data.splitlines():
    s = line.strip()
    if s.startswith('ENCODING'):
        enc = int(s.split()[1])
    elif s == 'BITMAP':
        in_bitmap = True
        rows = []
    elif in_bitmap:
        if s == 'ENDCHAR':
            if enc is not None and rows and enc >= 0:
                r8 = rows[:8]
                while len(r8) < 8:
                    r8.append(0)
                chars[enc] = r8
            in_bitmap = False
        else:
            rows.append(int(s, 16))
print('extracted:', len(chars))
items = sorted(chars.items())
out = []
out.append('//! 预烘焙 8x8 中文像素点阵 (Fusion Pixel Font 8px, SIL OFL 1.1)')
out.append('pub static CJK_GLYPHS: &[(char, [u8; 8])] = &[')
lines = []
for cp, rows in items:
    rb = ', '.join('0x%02X' % r for r in rows)
    lines.append(('    (' + chr(39) + chr(92) + 'u{%04X}' + chr(39) + ', [' + '%s' + ']),') % (cp, rb))
out.append('\n'.join(lines))
out.append('];')
open(r'D:\\Rust\\steel-front\\src\\engine\\cjk_glyphs.rs', 'w', encoding='utf-8').write('\n'.join(out))
print('written ok')