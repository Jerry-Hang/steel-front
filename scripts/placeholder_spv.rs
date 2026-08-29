//! RT 求交吞吐基准的 SPIR-V 手工汇编（naga 不支持 WGSL ray-query → 直接产二进制）
//! 每调用 1 条光线：初始化 ray-query → 全遍历（proceed 循环）→ 三角命中则 atomic 计数。

/// 手工发射 SPIR-V 指令流
pub fn rt_bench_spv() -> Vec<u32> {
    // ---- 头部（版本 1.6, generator 0, 4 个 word 指令计数占位）----
    let mut w: Vec<u32> = vec![0x0723_0203u32, 0x0001_0600u32, 0, 0, 0];
    let mut mid = 1u32;
    let mut nid = || { mid += 1; mid - 1 };
    let mut i32_op = |op: u32, ops: &[u32]| {
        let wc = 1 + ops.len();
        w.push((wc as u32) << 16 | op);
        w.extend_from_slice(ops);
    };
    // 捕获使用的前置声明
    
    // OpCapability
    i32_op(17, &[1]);          // Shader
    i32_op(17, &[4472]);       // RayQueryKHR
    // OpMemoryModel Logical GLSL450
    i32_op(14, &[0, 1]);
    // ---- 类型 ----
    let t_void = nid(); i32_op(19, &[t_void]);                       // OpTypeVoid
    let t_fn = nid(); i32_op(33, &[t_fn, t_void]);                   // OpTypeFunction void
    let t_bool = nid(); i32_op(20, &[t_bool]);                       // OpTypeBool
    let t_u32 = nid(); i32_op(21, &[t_u32, 32, 0]);                  // OpTypeInt 32 0
    let t_f32 = nid(); i32_op(22, &[t_f32, 32]);                     // OpTypeFloat 32
    let t_v3u = nid(); i32_op(23, &[t_v3u, t_u32, 3]);               // OpTypeVector u32 3
    let t_v3f = nid(); i32_op(23, &[t_v3f, t_f32, 3]);               // OpTypeVector f32 3
    let t_accel = nid(); i32_op(5341, &[t_accel]);                   // OpTypeAccelerationStructureKHR
    let t_p_acc = nid(); i32_op(32, &[t_p_acc, 0, t_accel]);         // UniformConstant* accel
    let t_p_u32 = nid(); i32_op(32, &[t_p_u32, 12, t_u32]);          // StorageBuffer* u32
    let t_p_f_v3u = nid(); i32_op(32, &[t_p_f_v3u, 7, t_v3u]);       // Function* v3u32
    let t_rq = nid(); i32_op(4472, &[t_rq]);                         // OpTypeRayQueryKHR
    let t_p_f_rq = nid(); i32_op(32, &[t_p_f_rq, 7, t_rq]);          // Function* RayQuery
    // ---- 常量 ----
    let c0 = nid(); i32_op(43, &[c0, t_u32, 0]);                     // u32 0
    let c255 = nid(); i32_op(43, &[c255, t_u32, 255]);               // mask 0xFF
    let c0f = nid(); i32_op(43, &[c0f, t_f32, 0]);                   // f32 0
    let ctmin = nid(); i32_op(43, &[ctmin, t_f32, 0x3a83126f]);      // 0.001
    let ctmax = nid(); i32_op(43, &[ctmax, t_f32, 0x447a0000]);      // 1000.0
    let vzero = nid(); i32_op(46, &[vzero, t_v3f, c0f, c0f, c0f]);   // vec3 0
    // ---- 全局变量 ----
    let g_tlas = nid(); i32_op(59, &[g_tlas, t_p_acc]);              // OpVariable
    let g_count = nid(); i32_op(59, &[g_count, t_p_u32]);            // OpVariable
    let g_gid = nid(); i32_op(59, &[g_gid, t_p_f_v3u]);              // OpVariable (Function)
    // ---- 入口点 ----
    let f_main = nid(); i32_op(15, &[f_main]);                        // OpEntryPoint 之后补
    // 重新组织：入口点写在函数前; 用占位再修正
    w[0] = 0x0723_0203;
    // 手动补齐入口点与执行模式（因 id 已分配，直接在函数前插会破坏顺序——改用后置重排）
    w
}