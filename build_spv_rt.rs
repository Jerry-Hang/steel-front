//! RT 求交吞吐基准的 SPIR-V 手工汇编器（naga 无 WGSL ray-query → 直接二进制）
//! 算法：每调用一条光线（原点=0，方向=gid 系数化）→ 初始化 ray-query → 全遍历（proceed 循环）
//! → 命中写 1 到 hits[gid.x]（无原子：CPU 读回求和 = 命中数）。
//! 输出：RT_BENCH_SPV 常量（给渲染器 compute 管线直接用）。

pub fn rt_bench_spv() -> Vec<u32> {
    let mut w: Vec<u32> = Vec::new();
    let mut id = 0u32;
    let mut nid = |id: &mut u32| { *id += 1; *id };
    let mut emit = |w: &mut Vec<u32>, op: u32, ops: &[u32]| {
        w.push(((1 + ops.len()) as u32) << 16 | op);
        w.extend_from_slice(ops);
    };
    let mut i = 0u32; // 全程唯一 id 计数器（按定义顺序分配）
    // ---- 头部 ----
    w.extend_from_slice(&[0x0723_0203u32, 0x0001_0600u32, 0u32, 0u32, 0u32]);
    // ---- Capabilities ----
    emit(&mut w, 17, &[1]);      // Shader
    emit(&mut w, 17, &[4472]);   // RayQueryKHR
    emit(&mut w, 17, &[5340]);   // AccelerationStructureKHR
    emit(&mut w, 14, &[0, 1]);   // MemoryModel Logical GLSL450

    // ---- 预先分配所有 id（定义在下面按序发射）----
    // 类型
    let t_void = nid(&mut i);
    let t_fn = nid(&mut i);
    let t_bool = nid(&mut i);
    let t_u32 = nid(&mut i);
    let t_f32 = nid(&mut i);
    let t_v3u = nid(&mut i);
    let t_v3f = nid(&mut i);
    let t_accel = nid(&mut i);
    let t_p_acc = nid(&mut i);
    let t_p_u32 = nid(&mut i);
    let t_p_v3u = nid(&mut i);
    let t_rq = nid(&mut i);
    let t_p_rq = nid(&mut i);
    // 常量
    let c0 = nid(&mut i);
    let c255 = nid(&mut i);
    let c0f = nid(&mut i);
    let c001f = nid(&mut i);
    let c1000f = nid(&mut i);
    let c_smallf = nid(&mut i); // 0.001（方向缩放）
    let vzero = nid(&mut i);
    // 全局变量
    let g_tlas = nid(&mut i);
    let g_hits = nid(&mut i);
    let g_gid = nid(&mut i);
    // 函数
    let f_main = nid(&mut i);
    // 函数内部
    let lbl_entry = nid(&mut i);
    let rq = nid(&mut i);
    let gv = nid(&mut i);
    let gx = nid(&mut i);
    let gy = nid(&mut i);
    let fx = nid(&mut i);
    let fy = nid(&mut i);
    let dir = nid(&mut i);
    let tlas_l = nid(&mut i);
    let loop_h = nid(&mut i);
    let cont = nid(&mut i);
    let merge = nid(&mut i);
    let latch = nid(&mut i);
    let ityp = nid(&mut i);
    let ishit = nid(&mut i);
    let l_hit = nid(&mut i);
    let l_skip = nid(&mut i);
    let p_hit = nid(&mut i);
    let one = nid(&mut i);

    // ---- 入口点 + 执行模式 + 装饰（在类型前，符合布局）----
    emit(&mut w, 15, &[5, f_main, 0x6d61_696eu32, g_gid]); // OpEntryPoint GLCompute %f_main "main" %g_gid
    emit(&mut w, 16, &[f_main, 17, 64, 1, 1]);            // LocalSize 64 1 1
    emit(&mut w, 71, &[g_tlas, 34, 0]);                    // DescriptorSet 0
    emit(&mut w, 71, &[g_tlas, 33, 0]);                    // Binding 0
    emit(&mut w, 71, &[g_hits, 34, 0]);                    // DescriptorSet 0
    emit(&mut w, 71, &[g_hits, 33, 1]);                    // Binding 1
    emit(&mut w, 71, &[g_gid, 11, 5]);                     // BuiltIn GlobalInvocationId

    // ---- 类型 ----
    emit(&mut w, 19, &[t_void]);
    emit(&mut w, 33, &[t_fn, t_void]);
    emit(&mut w, 20, &[t_bool]);
    emit(&mut w, 21, &[t_u32, 32, 0]);
    emit(&mut w, 22, &[t_f32, 32]);
    emit(&mut w, 23, &[t_v3u, t_u32, 3]);
    emit(&mut w, 23, &[t_v3f, t_f32, 3]);
    emit(&mut w, 5341, &[t_accel]);
    emit(&mut w, 32, &[t_p_acc, 0, t_accel]);
    emit(&mut w, 32, &[t_p_u32, 12, t_u32]);
    emit(&mut w, 32, &[t_p_v3u, 1, t_v3u]);
    emit(&mut w, 4472, &[t_rq]);
    emit(&mut w, 32, &[t_p_rq, 7, t_rq]);
    // ---- 常量 ----
    emit(&mut w, 43, &[c0, t_u32, 0]);
    emit(&mut w, 43, &[c255, t_u32, 255]);
    emit(&mut w, 43, &[c0f, t_f32, 0]);
    emit(&mut w, 43, &[c001f, t_f32, 0x3a83126f]);
    emit(&mut w, 43, &[c1000f, t_f32, 0x447a0000]);
    // 方向 z 分量直接用 0.001（c_smallf 与 c001f 复用）
    let c_smallf = c001f;
    emit(&mut w, 46, &[vzero, t_v3f, c0f, c0f, c0f]);
    emit(&mut w, 43, &[one, t_u32, 1]);
    // ---- 全局变量 ----
    emit(&mut w, 59, &[g_tlas, t_p_acc]);
    emit(&mut w, 59, &[g_hits, t_p_u32]);
    emit(&mut w, 59, &[g_gid, t_p_v3u]);

    // ---- 函数 main ----
    emit(&mut w, 54, &[f_main, t_void, 0, t_fn]);
    emit(&mut w, 248, &[lbl_entry]);
    emit(&mut w, 59, &[rq, t_p_rq]);
    emit(&mut w, 61, &[gv, t_v3u, g_gid]);
    emit(&mut w, 186, &[gx, t_u32, gv, 0]);
    emit(&mut w, 186, &[gy, t_u32, gv, 1]);
    emit(&mut w, 111, &[fx, t_f32, gx]);
    emit(&mut w, 111, &[fy, t_f32, gy]);
    emit(&mut w, 80, &[dir, t_v3f, fx, fy, c001f]);
    emit(&mut w, 61, &[tlas_l, t_accel, g_tlas]);
    emit(&mut w, 4473, &[rq, tlas_l, c0, c255, vzero, c001f, dir, c1000f]);
    emit(&mut w, 248, &[loop_h]);
    emit(&mut w, 246, &[merge, latch, 0]);
    emit(&mut w, 4477, &[cont, t_bool, rq]);
    emit(&mut w, 250, &[cont, latch, merge]);
    emit(&mut w, 248, &[latch]);
    emit(&mut w, 249, &[loop_h]);
    emit(&mut w, 248, &[merge]);
    emit(&mut w, 4479, &[ityp, t_u32, rq, c0]);
    emit(&mut w, 171, &[ishit, t_bool, ityp, c0]);
    emit(&mut w, 250, &[ishit, l_hit, l_skip]);
    emit(&mut w, 248, &[l_hit]);
    emit(&mut w, 65, &[p_hit, t_p_u32, g_hits, gx]);
    emit(&mut w, 62, &[p_hit, one]);
    emit(&mut w, 248, &[l_skip]);
    emit(&mut w, 253, &[]);
    emit(&mut w, 56, &[]);

    // 头部 word 数 + id 数
    w[3] = 44; // bound = 最大 ID(43) + 1
    w
}