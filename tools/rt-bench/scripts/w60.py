# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let mut s = format!(\"RT 鍏夌嚎杩借釜绠楀姏娴嬭瘯鏃ュ織\\nGPU: {}\\n鏃堕棿: {}\\n\\n閰嶇疆: {} 灏勭嚎 x {} 杩唬 x {} 杞甛n宄板€? {:.1} Mrays/s\\n涓綅: {:.1} Mrays/s\\n鎬诲懡涓? {}\\n璇勫垎: {}\\n\", gpu, now, rays, iters, rounds, r.best_mrays, r.median_mrays, r.total_hits, (r.best_mrays * 100.0) as u64);", "let mut s = format!(\"RT Ray-tracing Benchmark Log\\nGPU: {}\\nTime: {}\\n\\nConfig: {} rays x {} iters x {} rounds\\nPeak: {:.1} Mrays/s\\nMedian: {:.1} Mrays/s\\nTotalHits: {}\\nScore: {}\\n\", gpu, now, rays, iters, rounds, r.best_mrays, r.median_mrays, r.total_hits, (r.best_mrays * 100.0) as u64);")
s = s.replace("s.push_str(&format!(\"绗瑊}杞? {:.1} Mrays/s\\n\", i + 1, v));", "s.push_str(&format!(\"Round{}: {:.1} Mrays/s\\n\", i + 1, v));")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main log fixed')
p2 = 'src/rt_impl.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("        Ok(best)\n    }\n}", "        Ok(RtResult { best_mrays: best, median_mrays: best, total_hits: 0, rounds_mrays: vec![best] })\n    }\n}")
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('rt tail fixed')
