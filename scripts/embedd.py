# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
# 找到 PT_FRAME_SPV 写入段后加 DENOISE
anchor = 'output.push_str(&format!("pub const PT_FRAME_SPV: &[u32] = &{:?};\\n\\n", wv));'
if anchor in s:
    add = anchor + '''
    // 降噪后处理
    {
        let dn_path = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("rt").join("denoise.spv");
        if let Ok(dnb) = std::fs::read(dn_path) {
            let wv: Vec<u32> = dnb.chunks_exact(4).map(|c| u32::from_le_bytes([c[0],c[1],c[2],c[3]])).collect();
            output.push_str("pub const DENOISE_SPV: &[u32] = ");
            output.push_str(&format!("&{:?};\\n\\n", wv));
        }
    }'''
    s = s.replace(anchor, add, 1)
    io.open('build.rs', 'w', encoding='utf-8', newline='\n').write(s)
    print('DENOISE_SPV embedded')
else:
    print('anchor miss')
