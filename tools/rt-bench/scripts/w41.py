# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = """            let dt = t0.elapsed().as_secs_f64();
            let ops = items as f64 * 4096.0 * 2.0 * iters as f64; // FMA=2 ops
            let gops = ops / dt / 1e9;
            if gops < best { best = gops; }
            if gops > worst { worst = gops; }
        }"""
new = """            let dt = t0.elapsed().as_secs_f64();
            let ops = items as f64 * 4096.0 * 2.0 * iters as f64; // FMA=2 ops
            let gops = ops / dt / 1e9;
            if gops < best { best = gops; }
            if gops > worst { worst = gops; }
            // 输出校验：验证计算真实发生（防折叠被驱离）
            if let Ok(p) = dev.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()) {
                let arr = std::slice::from_raw_parts(p as *const u32, (size / 4) as usize);
                let mut sum = 0u64;
                for v in arr.iter().take(1024) { sum = sum.wrapping_add(*v as u64); }
                dev.unmap_memory(mem);
                if sum == 0 { best = 0.0; log_stop("FP 输出全零（计算未发生）"); }
            }
        }"""
if old in s:
    s = s.replace(old, new, 1)
    print('fp verify added')
# log_stop helper + worst 变量使用（防未用警告）
s = s.replace("use std::time::Instant;", "use std::time::Instant;\n\nstatic mut WARN: String = String::new();\nfn log_stop(msg: &str) { unsafe { WARN = msg.to_string(); } }\nfn take_warn() -> String { unsafe { std::mem::take(&mut WARN) } }")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('verify wired')
