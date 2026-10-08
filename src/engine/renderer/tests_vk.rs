// 由 renderer.rs 按主题拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 唯一改动是自扫源码那几处 include_str! 的相对路径（多一层目录）。
// 这些模块仍是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段与文件作用域私有项；
// `use super::*;` 把上一层的名字转发下来，子模块里的 `use super::*;` 因此仍解析到 renderer。
use super::*;

#[cfg(test)]
mod pt_config_tests {
    use super::pt_resident_needed;

    /// 🔴 `RV3D_PT_LIVE=1` 自称"强制开"，但 2026-09-25 之前它**只改 `pt_live_enabled`**，
    /// 而 PT 真正出画还要求 `pt_resident.is_some()`（常驻资源只在 `config.pt_enable==true` 时构建）
    /// ⇒ 只设环境变量时 PT **一帧都跑不出来**，外面只看到"开了但画面没变"。
    /// 那次"PT 验证"因此什么也没验到（详见 `docs/PROGRESS.md` §21.17）。
    ///
    /// 判据：环境变量 `1` 必须**也**触发常驻资源；显式 `0` 不构建（省显存，也符合"强制关"）。
    #[test]
    fn pt_live_env_one_also_builds_the_resident() {
        assert!(pt_resident_needed(false, Some("1")), "强制开必须真的能开");
        assert!(pt_resident_needed(true, None), "配置开着就构建");
        assert!(pt_resident_needed(true, Some("1")));
    }

    #[test]
    fn pt_resident_is_off_when_nothing_asks_for_it() {
        assert!(!pt_resident_needed(false, None));
        assert!(!pt_resident_needed(false, Some("0")));
        assert!(!pt_resident_needed(true, Some("0")), "强制关也要能关掉常驻资源");
    }

    use super::pt_render_extent;

    /// 🔴 `RV3D_PT_SIZE` 的注释写着"单值覆盖（等比）"，但 2026-09-25 之前它**只改宽**、
    /// 高始终取窗口高 ⇒ 实际是 `512x1600` 这种被压扁的图（PT 参照帧/功耗 A/B 因此失去可比性，
    /// 2026-09-25 那次 PT 验证就跑在 `512x1600` 上）。这里把它钉成"等比 + 8 的倍数"。
    #[test]
    fn pt_size_env_scales_proportionally() {
        // 2560x1600 窗口、用户给 512 ⇒ 高按同比例缩到 320（不是 1600）
        assert_eq!(pt_render_extent(2560, 1600, Some(512)), (512, 320));
        // 16:9 窗口：注意窗口高**先**对齐 8（900 → 896），再按 1024/1600 等比 ⇒ 896×0.64 = 573 → 568
        assert_eq!(pt_render_extent(1600, 900, Some(1024)), (1024, 568));
        // 未设 / 非法值 ⇒ 跟随窗口（仍是 8 的倍数）
        assert_eq!(pt_render_extent(2561, 1601, None), (2560, 1600));
        assert_eq!(pt_render_extent(2560, 1600, Some(100)), (2560, 1600)); // <128 非法
        assert_eq!(pt_render_extent(2560, 1600, Some(513)), (2560, 1600)); // 非 8 的倍数
        assert_eq!(pt_render_extent(2560, 1600, Some(8192)), (2560, 1600)); // >4096 非法
        // 窗口退化时不 panic、不返回 0
        assert_eq!(pt_render_extent(0, 0, Some(256)), (256, 256));
    }
}

#[cfg(test)]
mod pt_prop_attrs_tests {
    use super::pt_bake_prop_attrs;

    fn dec_n(w: u32, shift: u32) -> f32 {
        (((w >> shift) & 0xFF) as f32 - 127.0) / 127.0
    }
    fn dec_c(w: u32, shift: u32) -> f32 {
        ((w >> shift) & 0xFF) as f32 / 255.0
    }
    fn v(pos: [f32; 3], col: [f32; 3]) -> [f32; 11] {
        let mut a = [0.0f32; 11];
        a[0..3].copy_from_slice(&pos);
        a[8..11].copy_from_slice(&col);
        a
    }

    #[test]
    fn quantization_roundtrip_stays_within_u8_bounds() {
        // 任意朝向三角：法线往返误差 ≤ 半格（1/127·0.5 容差放宽到 1/127），色 ≤ 1/255
        let verts = [
            v([0.0, 0.0, 0.0], [0.10, 0.50, 0.90]),
            v([1.0, 0.3, 0.0], [0.20, 0.55, 0.85]),
            v([0.2, 1.0, 0.7], [0.30, 0.45, 0.80]),
        ];
        let idx = [0u32, 1, 2];
        let a = pt_bake_prop_attrs(&verts, &idx);
        assert_eq!(a.len(), 2);
        let raw = {
            let e1 = [1.0f32, 0.3, 0.0];
            let e2 = [0.2f32, 1.0, 0.7];
            let n = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            [n[0] / l, n[1] / l, n[2] / l]
        };
        for (k, want) in raw.iter().enumerate() {
            let got = dec_n(a[0], (k as u32) * 8);
            assert!(
                (got - want).abs() <= 1.0 / 127.0 + 1e-6,
                "法线轴 {k} 往返误差越界: got {got} want {want}"
            );
        }
        for (k, want) in [(0u32, 0.20f32), (8, 0.5), (16, 0.85)] {
            let got = dec_c(a[1], k);
            assert!(
                (got - (want * 255.0).round() / 255.0).abs() < 1e-6,
                "色均值量化不是最近格点: shift {k} got {got}"
            );
        }
    }

    #[test]
    fn degenerate_triangle_falls_back_to_up_normal() {
        let verts = [
            v([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
            v([2.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
            v([1.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
        ];
        let a = pt_bake_prop_attrs(&verts, &[0, 1, 2]);
        assert_eq!(dec_n(a[0], 0), 0.0);
        assert_eq!(dec_n(a[0], 8), 1.0);
        assert_eq!(dec_n(a[0], 16), 0.0);
    }

    #[test]
    fn tail_indices_below_one_full_triangle_are_dropped() {
        // 7 个索引 = 2 整角 + 1 尾：尾被丢弃 ⇒ 4 个字；
        // 与 prop_index_count(=7) 的等式把关会因此拒绝道具进 BLAS（宁缺不漏）
        let verts = [
            v([0.0, 0.0, 0.0], [0.4, 0.4, 0.4]),
            v([1.0, 0.0, 0.0], [0.4, 0.4, 0.4]),
            v([0.0, 1.0, 0.0], [0.4, 0.4, 0.4]),
            v([0.0, 0.0, 1.0], [0.4, 0.4, 0.4]),
        ];
        let a = pt_bake_prop_attrs(&verts, &[0, 1, 2, 0, 1, 3, 0]);
        assert_eq!(a.len(), 4);
        assert_ne!(a.len() as u32 / 2 * 3, 7, "等式把关对残缺索引必须不成立");
    }

    #[test]
    fn axis_aligned_faces_get_exact_normals() {
        // 水平面（+Y）与竖直面（+Z）的量化法线必须精确落在 ±1/0 格点上
        let up = [
            v([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            v([1.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            v([0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        ];
        let a = pt_bake_prop_attrs(&up, &[0, 2, 1]);
        assert_eq!(dec_n(a[0], 8), 1.0, "水平面法线必须是 +Y");
        assert_eq!(dec_n(a[0], 0), 0.0);
        assert_eq!(dec_n(a[0], 16), 0.0);
    }
}

#[cfg(test)]
mod vk_failure_path_tests {
    //! Vulkan **失败路径**的守卫（2026-09-22 复查新增）。
    //!
    //! 本仓这一年 GPU 侧的事故，修法全都是 `log` + 降级；而失败路径上有两种写法
    //! 会把"偶发的一次分配/映射失败"升级成更坏的状态：
    //! 1. 把 `expect` / `unwrap` 接在 `map_memory` / `create_buffer` / `allocate_memory`
    //!    这类调用后面 —— 直接 panic（切枪那一枪正好走这条路 = 整个进程没了）；
    //! 2. 用 `if let Ok(..)` 接 —— 失败被吞掉，后面的代码继续用**未初始化的显存**
    //!    （画错或设备消失，且不报错）。
    //!
    //! 这两条**没法用普通单测触发**（本机 `map_memory` 不会失败），所以退一步：
    //! 加源码检查把它们挡住。它是**检查，不是证明** —— 只管这两种写法。
    //! 判据：修之前这两条检查各自报出真实位置（枪模重映射两处 / 士兵索引映射一处），
    //! 修完转绿。
    //!
    //! ⚠️ 正文写在 `mod` 之内（`//!` 而不是 `///`）：外层的文档注释在 `mod` 行**之前**，
    //! 会被自己的扫描算进去 —— 那段文字里就写着这两个模式，自指 ⇒ 永远红。

    /// Vulkan 调用关键字（与真实写法逐字一致）
    ///
    /// 🔴 2026-09-26：`.create_debug_utils_messenger(` **不在这张表里**，于是
    /// `init_instance()` 里那句 `.expect("创建调试报告器失败")` 一直躲过了
    /// `no_expect_or_unwrap_on_vulkan_calls` —— 而那个函数**本来就返回 `Result`**，
    /// 也就是说：一次本可以干净返回的错误被升级成了进程 abort。
    /// **判据漏掉一个名字，规则就等于没有**（与教训 46「没跑成的第三种结局」同形）。
    const CALLS: [&str; 9] = [
        ".map_memory(",
        ".create_buffer(",
        ".allocate_memory(",
        ".bind_buffer_memory(",
        ".create_image(",
        ".create_image_view(",
        ".create_swapchain(",
        ".create_command_pool(",
        ".create_debug_utils_messenger(",
    ];

    /// 只把**代码行**算进扫描：注释里为了解释这个坑，本来就要写出这两个模式，
    /// 不排除的话"解释它的注释"会自己踩线（第一次就是这么红的）。
    fn is_comment(line: &str) -> bool {
        let t = line.trim_start();
        t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
    }

    /// 第 `idx` 行（含）往前 `window` 行内是否出现过 Vulkan 调用；返回相隔行数
    fn vk_call_within(lines: &[&str], idx: usize, window: usize) -> Option<usize> {
        (0..=window).find(|&back| {
            idx.checked_sub(back)
                .and_then(|j| lines.get(j))
                .is_some_and(|l| !is_comment(l) && CALLS.iter().any(|c| l.contains(c)))
        })
    }

    /// 🔴 **命令缓冲必须按「在飞帧槽位」取，不能按 `image_index` 取**（2026-09-25 验证层实测 + 探针）。
    ///
    /// 复现：独显 + mailbox + `RV3D_VALIDATION=1`（`DISABLE_RTSS_LAYER`/`DISABLE_GAMEPP_LAYER` 关掉
    /// 隐式层）跑 survive，两轮各 2 条：
    /// ```text
    /// vkBeginCommandBuffer(): on active VkCommandBuffer 0x…c66d0 before it has completed.
    ///   VUID-vkBeginCommandBuffer-commandBuffer-00049
    /// vkQueueSubmit(): … VkCommandBuffer 0x…c66d0 is already in use …
    ///   VUID-vkQueueSubmit-pCommandBuffers-00071
    /// ```
    /// 一次性探针（`RV3D_SYNC_DIAG=1`，验完即删）在报错那一拍打出**相邻两帧**：
    /// `cf=0 image=1 cb=0x…c66d0` 紧跟 `cf=1 image=1 cb=0x…c66d0` ——
    /// 同一张交换链图像被**连续两帧** acquire（mailbox 下完全合法），
    /// 而我们要等的围栏属于**另一个槽位** ⇒ 上一次提交还没完成就重录了同一条命令缓冲。
    /// 围栏只保证"这个槽位的上一次提交完成了"；它等于"这条命令缓冲的上一次提交完成了"
    /// **只当两者同槽**。所以命令缓冲必须跟着**围栏/槽位**走，不能跟着图像走。
    #[test]
    fn command_buffer_is_indexed_by_frame_slot_not_by_swapchain_image() {
        // ⚠️ 与上面那条同样的自指问题：正文写在 `mod` 之内，且先切掉本测试模块
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let code: Vec<&str> = src.lines().filter(|l| !is_comment(l)).collect();
        // 先证明这条检查真的扫到了东西（否则文件改名/被搬走时会静默恒真）
        assert!(
            code.iter().any(|l| l.contains("fn render(")),
            "检查失效：生产代码里没扫到 render()"
        );
        let bad: Vec<&str> = code
            .iter()
            .copied()
            .filter(|l| l.contains("command_buffers[image_index"))
            .collect();
        assert!(
            bad.is_empty(),
            "命令缓冲按图像下标取 ⇒ 会重录仍在 pending 的命令缓冲（VUID-00049 / -00071）：\n{}",
            bad.join("\n")
        );
        let by_slot = code
            .iter()
            .filter(|l| l.contains("command_buffers[self.current_frame]"))
            .count();
        assert!(
            by_slot >= 1,
            "按 current_frame 取命令缓冲的那一行不见了（实际命中 {} 处）",
            by_slot
        );
        // record 与 submit 必须用**同一条**：取一次存进局部变量，两处都用它
        assert!(
            code.iter().any(|l| l.contains("let cmd_buffer = self.command_buffers[self.current_frame]")),
            "期望 render() 里把命令缓冲取进局部变量 cmd_buffer，再由 record 与 submit 共用"
        );
    }

    /// Vulkan **等待**调用关键字（与真实写法逐字一致）
    const WAIT_CALLS: [&str; 4] = [
        ".wait_for_fences(",
        ".wait_semaphores(",
        ".acquire_next_image(",
        ".device_wait_idle(",
    ];

    /// 🔴 **所有 Vulkan 等待必须有上界**（铁律 B，2026-09-25 真机代价 = "游戏静默卡死"）。
    ///
    /// `u64::MAX` 当超时 = 无限等：GPU 侧再也不会 signal 时，进程**日志停住、无 panic、
    /// 无 VUID、无 `has been lost`**，外面只能看到"游戏死了"。判据就是这一条源码检查
    /// （改回无限等立刻红）。同类判据：`swapchain_waits_are_bounded` 钉住那几个常量有限。
    ///
    /// 现状：2026-09-25 复查时**截图读回那句 `wait_for_fences(..., u64::MAX)` 是漏网的一处**
    /// （第一轮只改了 acquire 与主循环围栏），修完为 0 处。
    #[test]
    fn no_unbounded_wait_on_vulkan_calls() {
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let code: Vec<&str> = src.lines().filter(|l| !is_comment(l)).collect();
        // 先证明这条检查真的扫到了等待调用（否则文件被搬走/改名时会静默恒真）
        let waits = code
            .iter()
            .filter(|l| WAIT_CALLS.iter().any(|w| l.contains(w)))
            .count();
        assert!(waits >= 3, "检查失效：只扫到 {} 处等待调用", waits);
        let bad: Vec<&str> = code
            .iter()
            .copied()
            .filter(|l| {
                l.contains("u64::MAX") && WAIT_CALLS.iter().any(|w| l.contains(w))
            })
            .collect();
        assert!(
            bad.is_empty(),
            "等待必须有限（u64::MAX = 无限等 ⇒ 静默卡死）：\n{}",
            bad.join("\n")
        );
    }

    /// 在源码文本里挑出「非原点、且整个角完全由字面量写死」的 `Offset3D`。
    ///
    /// 原点 `{ x: 0, y: 0, z: 0 }` 合法（每次 blit/copy 都要），所以只报非原点的那种。
    /// 逐行扫会漏掉跨行写法（`vk::Offset3D {` 后面换行），所以整段拼起来再按记号切。
    fn hardcoded_offsets_in(src: &str) -> Vec<String> {
        let mut bad = Vec::new();
        for frag in src.split("vk::Offset3D {").skip(1) {
            let body = frag.split('}').next().unwrap_or("");
            // 只含字段名/数字/分隔符/空白 ⇒ 这一角完全是字面量（`pw as i32` 这类带字母，被排除）。
            // ⚠️ 必须放行换行：跨行写法（`vk::Offset3D {` 之后换行）正是最容易漏的一类，
            // 第一版忘了放行 `\n`，下面那条跨行自检样本当场把它抓了出来。
            let literal_only = !body.is_empty()
                && body.chars().all(|c| {
                    c.is_ascii_digit()
                        || matches!(c, 'x' | 'y' | 'z' | ':' | ',' | ' ' | '-' | '\n' | '\r' | '\t')
                });
            if !literal_only {
                continue;
            }
            let nonzero = body
                .split(|c: char| !c.is_ascii_digit())
                .filter(|t| !t.is_empty())
                .any(|t| t.parse::<i64>().map(|n| n != 0).unwrap_or(false));
            if nonzero {
                bad.push(format!("vk::Offset3D {{{}", body.trim()));
            }
        }
        bad
    }

    /// 判据：**blit / copy 的边界不许是写死的像素数** —— 非原点的 `Offset3D` 必须由
    /// 具名尺寸（`swapchain_extent` / `src_w` / `dst_w` …）算出来。
    ///
    /// 真机代价（2026-09-26）：PT 上屏那次 `cmd_blit_image` 把目标角写死成 2560x1600，
    /// 于是"默认窗口尺寸能跑、别的尺寸越界" —— 而**默认尺寸正是平时验证用的那一个**，
    /// 所以它躲过了此前每一轮验证（这条 bug 是读代码读出来的，不是跑出来的）。
    /// 复现 = `scripts\run_resize_probe.ps1 -PT`：窗口改到 1280x720 后立刻
    /// `VUID-vkCmdBlitImage-dstOffsets-00203`。
    #[test]
    fn blit_regions_never_hardcode_pixel_extents() {
        // 自检：检测器必须能在"写死的"样本上判红、在"具名的"样本上判绿。
        // 少了这一步，"0 处"既可能是真干净，也可能是检测器根本没在工作（教训 27）。
        assert_eq!(
            hardcoded_offsets_in(
                "vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: 2560, y: 1600, z: 1 }"
            )
            .len(),
            1,
            "自检失败：写死像素数的样本没被判红"
        );
        assert!(
            hardcoded_offsets_in(
                "vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: pw as i32, y: ph as i32, z: 1 }"
            )
            .is_empty(),
            "自检失败：具名尺寸的样本被误判"
        );
        // 跨行写法也必须抓到（这正是逐行扫会漏掉的那种）
        assert_eq!(
            hardcoded_offsets_in("vk::Offset3D {\n    x: 2560,\n    y: 1600,\n    z: 1\n}").len(),
            1
        );

        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let code: Vec<&str> = src.lines().filter(|l| !is_comment(l)).collect();
        // 先证明真的扫到了 blit 区域（否则文件改名/被搬走时这条检查会静默恒真）
        assert!(
            code.iter().filter(|l| l.contains("vk::Offset3D {")).count() >= 4,
            "检查失效：生产代码里几乎没扫到 Offset3D"
        );
        assert!(
            code.iter().any(|l| l.contains(".cmd_blit_image(")),
            "检查失效：生产代码里没扫到 cmd_blit_image"
        );
        let bad = hardcoded_offsets_in(&code.join("\n"));
        assert!(
            bad.is_empty(),
            "blit/copy 的边界写死了像素数 ⇒ 换个窗口尺寸就越界（VUID-…-dstOffsets-00203）：\n{}",
            bad.join("\n")
        );
    }

    use super::{
        clamp_swapchain_extent, frame_suppressed, is_device_lost_error, pick_device_extensions,
        next_stall_count, present_stall, prop_buffer_growth_needed, shadow_due, shadow_static_due,
        should_retry_swapchain, swapchain_extent_choice, terrain_coarse_height, terrain_height,
        wait_idle_failure_message, PresentStall, PRESENT_STALL_FALLBACK, PRESENT_STALL_US,
        RECREATE_RETRY_MIN_SECS, TERRAIN_CELLS, TERRAIN_HALF,
    };
    use ash::vk;

    /// 判据：**最细一级地形网格的插值误差必须留在预算内**（把"看着差不多"变成数）。
    ///
    /// 依据（2026-09-26）：帧预算地图显示**地形网格占 8.4% 帧时间**（`RV3D_NO_TERRAIN=1`
    /// 实测 144.7 → 156.8），而它最细那一级是 **2m 网格**（256 格 × 512m，131k 三角形）。
    /// 可**城市中心是平的**（`terrain_height` 半径 140m 内恒为 0），2m 网格在那里纯属浪费；
    /// 丘陵区（140..256m）才需要密度。降到 4m 网格后实测 BASE +4.6%、全关底噪 184.6 → 200.4。
    ///
    /// ⚠️ 但降密度**会改变丘陵的曲面**（网格点之间线性插值），所以这里给的是**数**：
    /// 采样每个格子的内部点（网格点上误差恒为 0，只采格点等于没测），取 |网格插值 − 真值|
    /// 的最大值。**实测三档**（同一把尺子）：`256 格 = 0.007m`、**`128 格 = 0.029m`**、
    /// `64 格 = 0.110m` —— 误差按间距平方走，而丘陵本身 ≤15m ⇒ 4m 网格的 2.9cm 完全不可见。
    /// 阈值取 **0.10m**：既远高于现行值（0.029），又把"再粗一档"（8m ⇒ 0.110）挡在门外。
    #[test]
    fn terrain_finest_grid_interpolation_error_stays_within_budget() {
        let cells = TERRAIN_CELLS; // 现行最细一级（128 格 = 4m）
        let w = cells + 1;
        let cell = 512.0 / cells as f32;
        let mut hs: Vec<f32> = Vec::with_capacity(w * w);
        for iz in 0..w {
            for ix in 0..w {
                hs.push(terrain_height(
                    -TERRAIN_HALF + ix as f32 * cell,
                    -TERRAIN_HALF + iz as f32 * cell,
                ));
            }
        }
        let (mut worst, mut at) = (0.0f32, (0.0f32, 0.0f32));
        for iz in 0..cells {
            for ix in 0..cells {
                // 只采格子内部：网格点上的误差恒为 0（那正是采样点本身）
                for (fx, fz) in [(0.25f32, 0.25f32), (0.5, 0.5), (0.75, 0.25), (0.25, 0.75)] {
                    let x = -TERRAIN_HALF + (ix as f32 + fx) * cell;
                    let z = -TERRAIN_HALF + (iz as f32 + fz) * cell;
                    let mesh = terrain_coarse_height(x, z, &hs, cells);
                    let err = (mesh - terrain_height(x, z)).abs();
                    if err > worst {
                        worst = err;
                        at = (x, z);
                    }
                }
            }
        }
        assert!(
            worst < 0.10,
            "地形最细一级插值误差 {worst:.3}m 超预算（最差点 {at:?}）：网格太粗，丘陵会变形。\
             预算依据见 §21.46：现行 4m 网格实测 0.31m、旧的 2m 网格 0.08m"
        );
    }

    /// 🔴 **判据：缺扩展要降级，不是让 `create_device` 失败。**
    ///
    /// 原代码在 `VK_EXT_mesh_shader` 可用时**无条件**请求 5 个光追扩展，而枚举结果
    /// 只在事后打一行 warn ⇒ "设备缺任一光追扩展"的后果是**游戏起不来**，
    /// 而 PT 本来就是**默认关**的，根本不值得为它挡住启动。
    ///
    /// 三个方向各钉一条（都不是恒真断言）：
    /// 1. 全齐 ⇒ 整组启用（免得把正常路径也改坏）；
    /// 2. **缺任何一个 ⇒ 整组不启用** —— 这是最关键的一条：只启用剩下几个时，
    ///    特性链与后续代码路径都假设它们齐全，**半套是未定义行为，比整组不用更危险**；
    /// 3. `required` 缺了就如实报出来，但**不阻止**其它已支持的扩展启用
    ///    （把"可选"当"必需"正是这次要修的错）。
    #[test]
    fn device_extensions_degrade_instead_of_failing() {
        let rt = ["A", "B", "C", "D", "E"];
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();

        // 1) 全齐 ⇒ swap + 5 个光追全部启用，且不报缺失
        let (en, miss) = pick_device_extensions(&s(&["swap", "A", "B", "C", "D", "E"]), &["swap"], &rt);
        assert_eq!(miss.len(), 0, "全齐时不该报缺失");
        assert_eq!(en.len(), 6, "swap + 光追 5 个都该启用");

        // 2) 缺一个 ⇒ **整组**不启用（不是"启用剩下 4 个"）
        let (en, miss) = pick_device_extensions(&s(&["swap", "A", "B", "C", "D"]), &["swap"], &rt);
        assert_eq!(en, vec!["swap"], "光追组缺一个就必须整组不启用，只留 required");
        assert_eq!(miss, vec!["E"], "缺的那个要如实报出来");

        // 3) required 缺了 ⇒ 报出来，但不影响已支持的那一组
        let (en, miss) = pick_device_extensions(&s(&["A", "B", "C", "D", "E"]), &["swap"], &rt);
        assert!(!en.contains(&"swap"), "不支持的 required 不许出现在启用列表里");
        assert_eq!(miss, vec!["swap"]);
        assert_eq!(en.len(), 5, "光追组不受 required 缺失影响");

        // 4) 一个都没有 ⇒ 两边都空/都报，不 panic
        let (en, miss) = pick_device_extensions(&s(&[]), &["swap"], &rt);
        assert!(en.is_empty());
        assert_eq!(miss.len(), 6, "1 个 required + 5 个光追全报");
    }

    /// 🔴 **判据：`queue_present` 卡顿必须能被分类，且正常帧必须清零计数。**
    ///
    /// 背景：`vkQueuePresentKHR` **签名里没有超时参数**，加不了上界 —— 而 Wayland 下
    /// FIFO（合成器不提供 `wp_fifo_v1`）**就是在 present 里阻塞等 frame callback**，
    /// 窗口不可见时那个回调不会来。acquire 与围栏都有超时，present 此前是唯一没有的，
    /// 后果与 acquire 那次同类：日志停住、无 panic、无 VUID，从外面看就是"游戏死了"。
    /// 既然加不了超时，判据只能退化到**耗时**，所以更要保证它不会被误触发。
    ///
    /// 三条各有指向：
    /// 1. 正常耗时要判 `Ok` —— 否则每帧都在报"卡顿"；
    /// 2. 阈值附近要**跨过阈值取点**（教训 42：阈值型分支的测试必须取跨过阈值的输入，
    ///    `PRESENT_STALL_US - 1` 与 `PRESENT_STALL_US` 必须落在不同分支）；
    /// 3. **第 3 条是关键**：`Ok` 时调用方清零，所以"一次长卡顿 + 一次正常"之后
    ///    计数必须回到 1 而不是 2 —— 漏了清零，几次**偶发**长卡顿会累积成"连续三次"
    ///    从而误降级（把好端端的 mailbox/fifo 换掉）。
    ///    这里用**状态机走一遍**来钉住它，不是只调一次纯函数。
    #[test]
    fn present_stall_classifies_and_clears() {
        // 1) 正常帧：Ok，且必须让计数清零
        assert_eq!(present_stall(43, 1), PresentStall::Ok);
        assert_eq!(present_stall(373, 1), PresentStall::Ok);

        // 2) 跨过阈值取点（教训 42）
        assert_eq!(present_stall(PRESENT_STALL_US - 1, 1), PresentStall::Ok);
        assert_eq!(present_stall(PRESENT_STALL_US, 1), PresentStall::Warn);
        assert_eq!(present_stall(PRESENT_STALL_US, PRESENT_STALL_FALLBACK), PresentStall::Degrade);

        // 3) 状态机走一遍 —— **用的是线上那个 `next_stall_count`**，不是测试里另抄一份。
        //    漏了清零时几次偶发长卡顿会累积成"连续三次"从而误降级，这条就是钉它的。
        let mut n: u32 = 0;
        n = next_stall_count(n, PRESENT_STALL_US);
        assert_eq!(n, 1, "第 1 次卡顿");
        n = next_stall_count(n, 43);
        assert_eq!(n, 0, "中间来一个正常帧 ⇒ 必须清零");
        n = next_stall_count(n, PRESENT_STALL_US);
        assert_eq!(n, 1, "清零后重新数，不能是 2");
        n = next_stall_count(n, PRESENT_STALL_US);
        assert_eq!(n, 2);
        n = next_stall_count(n, PRESENT_STALL_US);
        assert_eq!(n, 3, "连续 3 次才该降级");
        assert_eq!(
            present_stall(PRESENT_STALL_US, n),
            PresentStall::Degrade,
            "计数到 FALLBACK 时必须判降级"
        );

        // 4) 计数的饱和：u32 上限下不许 panic（release 里溢出是回绕，别让它变成"突然不卡了"）
        assert_eq!(next_stall_count(u32::MAX, PRESENT_STALL_US), u32::MAX);
    }

    /// 判据：交换链兜底尺寸必须落在 surface 给的范围内。
    ///
    /// 依据（2026-09-26 静态复查）：旧代码在 `current_extent == u32::MAX` 这一支写死 `1280x720`。
    /// 本机 Win32 surface 恒给具体尺寸 ⇒ **这条分支在默认配置下永远走不到**，写死的值也就永远
    /// 没被验证过（同 §21.23 的 PT blit：默认窗口恰好等于写死的 2560x1600）。
    #[test]
    fn swapchain_fallback_extent_is_clamped() {
        let fallback = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let wide = (
            vk::Extent2D {
                width: 16,
                height: 16,
            },
            vk::Extent2D {
                width: 8192,
                height: 8192,
            },
        );
        // 1) 常规设备：兜底值本来就合法 ⇒ 原样使用
        let e = clamp_swapchain_extent(fallback, wide.0, wide.1);
        assert_eq!((e.width, e.height), (1280, 720));
        // 2) max 更小 ⇒ 夹到 max（旧写死值就是在这条上违规）
        let e = clamp_swapchain_extent(
            fallback,
            wide.0,
            vk::Extent2D {
                width: 1024,
                height: 600,
            },
        );
        assert_eq!((e.width, e.height), (1024, 600), "超过 max 必须夹回去");
        // 3) min 更大 ⇒ 抬到 min
        let e = clamp_swapchain_extent(
            fallback,
            vk::Extent2D {
                width: 1920,
                height: 1080,
            },
            wide.1,
        );
        assert_eq!((e.width, e.height), (1920, 1080), "低于 min 必须抬上来");
        // 4) 退化输入：全 0 也不能产出 0（imageExtent=0 没有兜底路径）
        let zero = vk::Extent2D {
            width: 0,
            height: 0,
        };
        let e = clamp_swapchain_extent(fallback, zero, zero);
        assert_eq!((e.width, e.height), (1, 1), "零尺寸退到 1x1");
        // 5) max < min（驱动乱报）：以 min 为准，且不小于 1
        let e = clamp_swapchain_extent(
            fallback,
            vk::Extent2D {
                width: 800,
                height: 600,
            },
            vk::Extent2D {
                width: 100,
                height: 100,
            },
        );
        assert_eq!((e.width, e.height), (800, 600));
    }

    /// 🔴 **判据（Linux 适配）：`currentExtent` 未定义时必须用窗口尺寸，不是写死的 1280x720。**
    ///
    /// 背景：`VkSurfaceCapabilitiesKHR::currentExtent` 在 **Wayland 下是设计上未定义的**
    /// （Mesa 填 `{UINT32_MAX, UINT32_MAX}`），而 Win32 / X11 下恒等于窗口尺寸
    /// ⇒ 旧代码那条「否则用 1280x720」的分支**在 Windows 上永远走不到**，
    /// 于是它在 Linux 上把整条渲染尺寸链钉死在 720p，且不报错。
    ///
    /// 四条断言各自钉住一个**会写错的方向**（都不是恒真断言）：
    /// 1. `currentExtent` 有定义 ⇒ **原样返回**（Windows 侧行为不许被这次改动碰到）；
    /// 2. 未定义 + 有窗口尺寸 ⇒ **返回窗口尺寸**（这才是修掉的那条）；
    /// 3. 未定义 + 窗口尺寸为 0（Wayland 首个 configure 之前）⇒ 退到 1280x720；
    /// 4. 前两条都必须过 `min/maxImageExtent` 这一关（否则 `imageExtent-01274`）。
    #[test]
    fn swapchain_extent_follows_the_window_when_current_extent_is_undefined() {
        let undef = vk::Extent2D {
            width: u32::MAX,
            height: u32::MAX,
        };
        let wide = (
            vk::Extent2D {
                width: 16,
                height: 16,
            },
            vk::Extent2D {
                width: 8192,
                height: 8192,
            },
        );
        let win = vk::Extent2D {
            width: 2560,
            height: 1600,
        };

        // 1) 有定义 ⇒ 原样返回，**即使它和窗口尺寸不一致**（合成器说了算）
        let e = swapchain_extent_choice(
            vk::Extent2D {
                width: 1920,
                height: 1080,
            },
            win,
            wide.0,
            wide.1,
        );
        assert_eq!((e.width, e.height), (1920, 1080), "有定义时不许被窗口尺寸顶掉");

        // 2) 未定义 ⇒ 窗口尺寸（旧代码在这条返回 1280x720）
        let e = swapchain_extent_choice(undef, win, wide.0, wide.1);
        assert_eq!(
            (e.width, e.height),
            (2560, 1600),
            "Wayland 下交换链必须跟随窗口；返回 1280x720 就是这次修掉的缺陷"
        );

        // 3) 未定义且窗口还没尺寸 ⇒ 才退到兜底
        let e = swapchain_extent_choice(
            undef,
            vk::Extent2D {
                width: 0,
                height: 0,
            },
            wide.0,
            wide.1,
        );
        assert_eq!((e.width, e.height), (1280, 720));

        // 4) 夹取对两条路都生效：窗口尺寸超 max ⇒ 夹回 max
        let e = swapchain_extent_choice(
            undef,
            vk::Extent2D {
                width: 16384,
                height: 16384,
            },
            wide.0,
            vk::Extent2D {
                width: 4096,
                height: 4096,
            },
        );
        assert_eq!((e.width, e.height), (4096, 4096), "窗口尺寸也必须夹进 maxImageExtent");
    }

    /// 判据：阴影图**隔帧重画**的调度（纯函数）。
    ///
    /// 依据：`perf_run -NoShadow` 的 A/B 显示阴影 pass 占 ~32% 帧时间，而画面里每帧真的
    /// 会动的只有 NPC 的箱子（太阳、道具、地形都静止）⇒ 隔帧重画只让影子旧一帧。
    /// 这条测试同时钉住 `every = 1` 必须**逐帧**都画（A/B 对照组的语义）。
    #[test]
    fn shadow_pass_is_scheduled_every_n_frames() {
        for s in 0..5u64 {
            assert!(shadow_due(s, 1, false), "every=1 必须每帧都画（A/B 对照）");
        }
        assert!(shadow_due(0, 2, false));
        assert!(!shadow_due(1, 2, false), "隔帧：奇数帧跳过");
        assert!(shadow_due(2, 2, false));
        assert!(shadow_due(4, 4, false));
        assert!(!shadow_due(5, 4, false));
        assert!(!shadow_due(0, 1, true), "检视模式从不画");
        assert!(
            shadow_due(3, 0, false),
            "非法间隔（0）被夹成 1 ⇒ 仍然每帧画，绝不能变成永不画"
        );
    }

    /// 判据：**静态**阴影图的调度（纯函数）。
    ///
    /// 静态图装的是不动的那批投射者（地形/地面场/marker/道具），所以它只需要偶尔重画；
    /// 关掉拆分（`split = false`）时不单独画 —— 那条路走"单张图、两类一起画"的旧逻辑。
    #[test]
    fn static_shadow_pass_is_scheduled_every_n_frames() {
        // 默认 30 帧一次：只有 0、30、60… 这几帧画
        assert!(shadow_static_due(0, 30, false, true), "首帧必须画（图里还什么都没有）");
        assert!(!shadow_static_due(1, 30, false, true));
        assert!(shadow_static_due(30, 30, false, true));
        assert!(!shadow_static_due(31, 30, false, true));
        // 非法间隔被夹成 1 ⇒ 退化成每帧画，绝不能变成"永不画"
        assert!(shadow_static_due(7, 0, false, true));
        // 关掉拆分 / 检视模式：都不单独画静态图
        assert!(!shadow_static_due(0, 30, false, false), "拆分关掉时不画静态图");
        assert!(!shadow_static_due(0, 30, true, true), "检视模式不画");
    }

    /// 判据：道具缓冲**只在要得更多时**才重建（`need > capacity`，不是 `need != capacity`）。
    ///
    /// 写成 `!=` 的代价枪模那边实测过：destroy 正在被 GPU 使用的 buffer ⇒ `VK_ERROR_DEVICE_LOST`。
    #[test]
    fn prop_buffer_growth_is_strictly_by_need() {
        // 道具变少（换小地图）：绝不重建
        assert!(!prop_buffer_growth_needed(10_000, 65_536, 20_000, 65_536, true));
        // 刚好相等：不重建
        assert!(!prop_buffer_growth_needed(65_536, 65_536, 65_536, 65_536, true));
        // 要得更多（顶点 / 索引任一超过容量）：才重建
        assert!(prop_buffer_growth_needed(65_537, 65_536, 20_000, 65_536, true));
        assert!(prop_buffer_growth_needed(10_000, 65_536, 65_537, 65_536, true));
        // 句柄还没建（首帧 / 上次创建失败）：必须建
        assert!(prop_buffer_growth_needed(1, 65_536, 1, 65_536, false));
    }

    /// 判据：**先建新的，成功了再拆旧的** —— 上传缓冲的销毁不许出现在创建之前。
    ///
    /// 真机代价（2026-09-26 复查，`set_props`）：旧写法是「先 unmap/destroy/free 旧的 →
    /// 再 create 新的」，而 create 失败时只 `log + return` ⇒ 句柄字段里留着
    /// **已销毁却非 null** 的 VkBuffer：① 阴影 pass 只判 `!= null` 就绑它；
    /// ② 下一次扩容 / 退出清理对同一句柄**二次 destroy_buffer**。
    /// 枪模路径 2026-09-22 已改成"先建后毁"，道具这条路当时漏了 —— 所以这条检查
    /// 对**两条路**都必须成立（两条都在这里扫）。
    ///
    /// ⚠️ 只认"跟这一对上传缓冲有关"的销毁：`set_props` 开头还会销毁 PT 属性表
    /// （`prop_attr_buf`，那处本来就正确置空），把它算进来会误报（第一版就是这么红的）。
    #[test]
    fn upload_buffers_are_created_before_the_old_ones_are_destroyed() {
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let lines: Vec<&str> = src.lines().collect();
        // 函数名 → 该函数里"这一对上传缓冲"的字段名（用于把无关的销毁排除掉）
        // ⚠️ 只匹配 `fn NAME(`：拆模块时这些方法统一加宽成 `pub(crate) fn`（Rust 的方法私有性
        // 不同于字段私有性，见 docs/refactor-plan.md），写死 `pub fn ` 会扫不到而误红。
        let targets: [(&str, [&str; 4]); 2] = [
            (
                "fn set_props(",
                [
                    "prop_vertex_buffer",
                    "prop_index_buffer",
                    "prop_vertex_memory",
                    "prop_index_memory",
                ],
            ),
            (
                "fn set_first_person_gun_mesh(",
                [
                    "gun_vertex_buffer",
                    "gun_index_buffer",
                    "gun_vertex_buffer_memory",
                    "gun_index_buffer_memory",
                ],
            ),
        ];
        for (fname, names) in targets {
            let start = lines
                .iter()
                .position(|l| l.contains(fname))
                .unwrap_or_else(|| panic!("检查失效：源码里找不到 {fname}"));
            // 函数体 = 到下一个同级 `fn` 为止（impl 内的方法都是 4 空格缩进；含 `pub(crate) fn`）
            let end = lines[start + 1..]
                .iter()
                .position(|l| {
                    l.starts_with("    fn ")
                        || l.starts_with("    pub fn ")
                        || l.starts_with("    pub(crate) fn ")
                })
                .map(|i| start + 1 + i)
                .unwrap_or(lines.len());
            let body: Vec<&str> = lines[start..end]
                .iter()
                .copied()
                .filter(|l| !is_comment(l))
                .collect();
            let first_create = body
                .iter()
                .position(|l| l.contains("create_host_buffer("))
                .unwrap_or_else(|| panic!("检查失效：{fname} 里没扫到 create_host_buffer"));
            // "这一对缓冲"的销毁：销毁语句本身提到字段名，或紧邻上文（旧写法把字段塞在一个
            // 元组数组里循环销毁）提到字段名
            let first_destroy = body.iter().enumerate().position(|(i, l)| {
                if !(l.contains("destroy_buffer(") || l.contains("free_memory(")) {
                    return false;
                }
                let from = i.saturating_sub(6);
                body[from..=i]
                    .iter()
                    .any(|w| names.iter().any(|n| w.contains(n)))
            });
            let first_destroy = first_destroy
                .unwrap_or_else(|| panic!("检查失效：{fname} 里没扫到销毁这一对上传缓冲的语句"));
            assert!(
                first_create < first_destroy,
                "{fname}：销毁旧缓冲出现在创建新缓冲之前 —— 创建失败就会留下已销毁但非 null 的句柄\n\
                 创建在第 {} 行、销毁在第 {} 行（函数内相对行号，不含注释）",
                first_create + 1,
                first_destroy + 1
            );
        }
    }


    /// 判据：**设备丢失之后不许再重试重建**。
    ///
    /// 真机代价（2026-09-26，`scripts\run_resize_probe.ps1 -PT`）：PT blit 的越界目标范围
    /// 把设备打掉之后，`main.rs` 的尺寸自检每帧重试重建交换链 —— 12 秒 1961 轮，
    /// 1961 条 WARN + 3930 条 ERROR（`重建交换链失败：等待设备空闲失败: … has been lost`），
    /// 进程还活着、窗口还在，但一帧都不再更新。修复 = 设备丢失置粘性标志 ⇒ 这一处返回 false。
    #[test]
    fn device_lost_stops_rebuilding() {
        // 设备丢失：无论距上次多久（就算已经过了很久），都不值得再试
        assert!(
            !should_retry_swapchain(true, true, RECREATE_RETRY_MIN_SECS * 10.0),
            "设备丢失后重试重建 = 每帧刷错误日志（不可恢复）"
        );
        assert!(!should_retry_swapchain(true, true, 0.0));
        // 不是设备丢失、交换链也没坏 ⇒ 值得试
        assert!(should_retry_swapchain(false, false, 0.0));
        // 刚失败过 ⇒ 限流（避免 160Hz 刷屏）
        assert!(!should_retry_swapchain(false, true, 0.0));
        assert!(!should_retry_swapchain(false, true, RECREATE_RETRY_MIN_SECS * 0.5));
        // 过了一秒 ⇒ 允许再试（重建成功即自动恢复）
        assert!(should_retry_swapchain(false, true, RECREATE_RETRY_MIN_SECS));
        assert!(should_retry_swapchain(false, true, RECREATE_RETRY_MIN_SECS + 0.01));
    }

    /// 判据：`VK_ERROR_DEVICE_LOST` 必须能被认出来（引擎各层只做字符串前缀拼接，
    /// 到这里只剩 ash 的 Display 文本）。样本取自真机日志那一行。
    #[test]
    fn device_lost_error_text_is_recognised() {
        assert!(is_device_lost_error(
            "重建交换链失败：等待设备空闲失败: The logical device has been lost. See <https://registry.khronos.org/vulkan/specs/1.3-extensions/html/vkspec.html#devsandqueues-lost-device>"
        ));
        assert!(is_device_lost_error("等待围栏失败: ERROR_DEVICE_LOST"));
        // 不许误判：普通的交换链失败（设备还活着）必须仍然是"可重试"
        assert!(!is_device_lost_error("重建交换链失败：交换链创建失败: ERROR_OUT_OF_DATE_KHR"));
        assert!(!is_device_lost_error("等待围栏超时（5s 内这一帧没完成）"));
    }

    /// 判据：降级状态必须真的挡住提交（`gpu_stalled` / `swapchain_broken`）。
    #[test]
    fn degraded_states_suppress_the_frame() {
        assert!(!frame_suppressed(false, false));
        assert!(frame_suppressed(true, false), "GPU 卡死必须停止提交");
        assert!(
            frame_suppressed(false, true),
            "交换链重建中途失败后不许再拿可能已销毁的句柄提交"
        );
        assert!(frame_suppressed(true, true));
    }

    /// 判据：`renderer.rs` 里对 Vulkan 调用的结果不许 `.expect()` / `.unwrap()`。
    /// 现状（2026-09-22 修完后）为 0 处；修前会红在两处枪模 `map_memory`（5353 / 5365 行）。
    #[test]
    fn no_expect_or_unwrap_on_vulkan_calls() {
        // ⚠️ 只扫**生产代码**：`include_str!` 会把本测试模块自身也读进来，
        // 而它正文里就写着 `.expect(` 这几个字（自指 ⇒ 这条检查永远红）。
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let lines: Vec<&str> = src.lines().collect();
        // 先证明这条检查真的扫到了东西（否则文件被搬走/改名时会静默恒真）
        assert!(
            lines.iter().any(|l| l.contains(".map_memory(")),
            "检查失效：源码里一个 map_memory 都没扫到"
        );
        let mut bad = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            if !(line.contains(".expect(") || line.contains(".unwrap()")) {
                continue;
            }
            if let Some(back) = vk_call_within(&lines, i, 8) {
                bad.push(format!(
                    "第 {} 行（Vulkan 调用在 {} 行之前）: {}",
                    i + 1,
                    i + 1 - back,
                    line.trim()
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "Vulkan 失败路径必须 log + 降级，不许 panic：\n{}",
            bad.join("\n")
        );
    }

    /// `device_wait_idle` 的失败必须**点名错误**且只报一次（判据 = `wait_idle_failure_message`）。
    ///
    /// 点名是硬要求：`device lost` 与 `out of host memory` 的处置完全相反
    /// （前者不可恢复、后者可以少要点资源重试），两者在日志里长得一样就等于什么都没说。
    #[test]
    fn wait_idle_failure_is_named_and_reported_once() {
        let lost = wait_idle_failure_message(vk::Result::ERROR_DEVICE_LOST, false)
            .expect("第一次失败必须留痕（静默正是这次复查要消灭的形状）");
        assert!(lost.contains("ERROR_DEVICE_LOST"), "消息必须点名错误：{lost}");
        // 与既有的"什么算设备丢失"判定联动：日志里出现它，外面就能认出"不可恢复"
        assert!(
            is_device_lost_error(&lost),
            "device lost 必须能被 is_device_lost_error 认出：{lost}"
        );
        let oom = wait_idle_failure_message(vk::Result::ERROR_OUT_OF_HOST_MEMORY, false)
            .expect("第一次失败必须留痕");
        assert!(oom.contains("ERROR_OUT_OF_HOST_MEMORY"), "消息必须点名错误：{oom}");
        assert!(!is_device_lost_error(&oom), "OOM 不该被误判成设备丢失");
        assert_ne!(lost, oom, "两种错误不能长一样");
        assert!(
            wait_idle_failure_message(vk::Result::ERROR_DEVICE_LOST, true).is_none(),
            "第二次必须被闩住 —— 这几条路径在换枪/重载/PT 重建时反复跑"
        );
    }

    /// 🔴 **`device_wait_idle` 的结果不许被 `let _ =` 丢掉**（2026-09-26 复查实测 6 处）。
    ///
    /// 那 6 处全是"等空闲 → 销毁/重建在飞资源"的关键路径
    /// （`set_first_person_gun_mesh` / `set_props` ×2 / `set_shadow_props` /
    /// `pt_set_scene_markers` / `Drop`），而**等待失败与等待成功在日志里长得一模一样**：
    /// 后面那句 `destroy_buffer` 到底安不安全，排查的人手里没有任何证据。
    /// 它最可能返回的错误正好是 `VK_ERROR_DEVICE_LOST`（铁律 B：不可恢复）。
    ///
    /// 判据 = 本测试（改回 `let _ =` 立刻红）+ `wait_idle_failure_is_named_and_reported_once`。
    #[test]
    fn device_wait_idle_errors_are_never_silently_dropped() {
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let code: Vec<&str> = src.lines().filter(|l| !is_comment(l)).collect();
        // 先证明这条检查真的扫到了那条调用（否则文件被搬走/改名时会静默恒真）
        let calls = code.iter().filter(|l| l.contains(".device_wait_idle(")).count();
        assert!(calls >= 1, "检查失效：生产代码里没扫到 device_wait_idle");
        let bad: Vec<&str> = code
            .iter()
            .copied()
            .filter(|l| l.contains("let _ =") && l.contains(".device_wait_idle("))
            .collect();
        assert!(
            bad.is_empty(),
            "device_wait_idle 的结果被丢掉 ⇒ 等待失败与等待成功在日志里无法区分，\
             而失败时紧跟的 destroy/create 并不安全：\n{}",
            bad.join("\n")
        );
        // 正对照：留痕入口必须在 —— 它才是把"失败"变成证据的那一处
        assert!(
            code.iter().any(|l| l.contains("fn wait_idle_checked")),
            "统一的留痕入口 wait_idle_checked 不见了"
        );
    }

    /// 判据：Vulkan 调用的失败不许被 `if let Ok(..)` **静默吞掉**。
    /// 修前会红在士兵网格的索引映射（`if let Ok(ip) = ...map_memory(...)`）——
    /// 那一处失败时索引一个都没写，函数却继续把 `soldier_index_count` 设成
    /// `indices.len()`，draw call 读未初始化显存。
    #[test]
    fn no_if_let_ok_swallowing_vulkan_calls() {
        // 扫**整个 renderer 生产子树**（拆模块后只扫 renderer.rs 会漏掉搬走的代码）；
        // 支持模块见 src/engine/renderer/tests_support.rs（fail-closed）。
        let full = super::tests_support::renderer_production_sources();
        let src = full.as_str();
        let lines: Vec<&str> = src.lines().collect();
        assert!(
            lines.iter().any(|l| l.contains(".map_memory(")),
            "检查失效：源码里一个 map_memory 都没扫到"
        );
        let mut bad = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if is_comment(line) {
                continue;
            }
            if !line.contains("if let Ok(") {
                continue;
            }
            if let Some(back) = vk_call_within(&lines, i, 8) {
                bad.push(format!(
                    "第 {} 行（Vulkan 调用在 {} 行之前）: {}",
                    i + 1,
                    i + 1 - back,
                    line.trim()
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "Vulkan 调用的失败不许用 if let Ok 吞掉（要 log + 降级）：\n{}",
            bad.join("\n")
        );

        // 同一类静默通路的另一半（2026-09-26 补）：`<vulkan 调用>(...).ok();`
        // —— `.ok()` 把 `Err` 变成 `None`，**编译器就不再管了**（`Result` 的 #[must_use] 失效），
        // 是 `let _ =` / `if let Ok` 之外的第三条静默通路。
        // ⚠️ 只查**同一行**：`.ok()` 也大量出现在非 Vulkan 调用上（`env::var(..).ok()`、
        // `Mutex::lock().ok()`），跨行匹配会把它们全算进来 —— **会误判的检查等于没有检查**（教训 26）。
        let calls: Vec<&str> = CALLS.iter().chain(WAIT_CALLS.iter()).copied().collect();
        let bad_ok: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !is_comment(l))
            .filter(|l| l.contains(".ok()") && calls.iter().any(|c| l.contains(c)))
            .collect();
        assert!(
            bad_ok.is_empty(),
            "Vulkan 调用的失败不许用 `.ok()` 抹平（要 log + 降级）：\n{}",
            bad_ok.join("\n")
        );
    }
}

/// `queue_present` 结果分类的判据（2026-09-22 复查新增）。
///
/// 背景：呈现路径此前只认两种"需要重建交换链"的结果，**其余 `Err` 落空** ——
/// 于是 `ERROR_SURFACE_LOST_KHR` / `ERROR_DEVICE_LOST` 被当成呈现成功。
/// 这个分类是纯函数，所以这条真的能用单测钉住（不像上面的源码守卫）。
#[cfg(test)]
mod present_result_tests {
    use super::{classify_present, PresentOutcome};
    use super::{classify_acquire_err, AcquireOutcome};
    use super::{frame_action, FrameAction};
    use super::{ACQUIRE_STALL_FALLBACK, ACQUIRE_STALL_MAX, ACQUIRE_TIMEOUT_NS};
    use super::FENCE_WAIT_TIMEOUT_NS;
    use super::{fence_stall_due, FENCE_STALL_MAX};
    use ash::vk;

    #[test]
    fn only_ok_false_counts_as_presented() {
        assert_eq!(classify_present(Ok(false)), PresentOutcome::Presented);
    }

    #[test]
    fn suboptimal_and_out_of_date_ask_for_a_new_swapchain() {
        assert_eq!(classify_present(Ok(true)), PresentOutcome::RecreateSwapchain);
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_OUT_OF_DATE_KHR)),
            PresentOutcome::RecreateSwapchain
        );
    }

    /// 这两条就是"改了 2026-09-22 之前会红"的判据：旧写法把它们当成功了。
    #[test]
    fn surface_lost_and_device_lost_are_failures() {
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_SURFACE_LOST_KHR)),
            PresentOutcome::Failed
        );
        assert_eq!(
            classify_present(Err(vk::Result::ERROR_DEVICE_LOST)),
            PresentOutcome::Failed
        );
    }

    /// 🔴 **等待必须有上界**（2026-09-25 真机教训）：独显 + `defense_line` + IMMEDIATE 那次
    /// "游戏死了"的真身是主循环用 `u64::MAX` 无限等 acquire ⇒ 日志停住、无 panic、无 VUID。
    /// 这条断言把"不许再出现无限等待"钉死（改回 `u64::MAX` 立刻红）。
    #[test]
    fn swapchain_waits_are_bounded() {
        assert!(
            ACQUIRE_TIMEOUT_NS > 0 && ACQUIRE_TIMEOUT_NS <= 2_000_000_000,
            "acquire 超时必须有限且 ≤2s，实际 {}",
            ACQUIRE_TIMEOUT_NS
        );
        assert!(
            FENCE_WAIT_TIMEOUT_NS > 0 && FENCE_WAIT_TIMEOUT_NS <= 10_000_000_000,
            "围栏超时必须有限且 ≤10s，实际 {}",
            FENCE_WAIT_TIMEOUT_NS
        );
        assert!(
            ACQUIRE_STALL_FALLBACK >= 1 && ACQUIRE_STALL_FALLBACK < ACQUIRE_STALL_MAX,
            "降级阈值必须在放弃阈值之前（{} < {}）",
            ACQUIRE_STALL_FALLBACK,
            ACQUIRE_STALL_MAX
        );
    }

    /// 围栏超时的升级判据：前两次只报错（可能只是某一帧特别久），第三次才判定卡死。
    #[test]
    fn fence_stall_escalates_after_three_timeouts() {
        assert!(!fence_stall_due(0));
        assert!(!fence_stall_due(1));
        assert!(!fence_stall_due(FENCE_STALL_MAX - 1));
        assert!(fence_stall_due(FENCE_STALL_MAX));
        assert!(fence_stall_due(FENCE_STALL_MAX + 5));
    }

    /// 🔴 **成功 acquire 之后不许提前 return**（2026-09-25 深夜复查）。
    ///
    /// acquire 成功 = `image_available_semaphores[current_frame]` 已被 signal，而这张图像只有
    /// 走完 present 才会被交还；那个信号量是渲染器生命周期对象，**重建交换链不会重建它**
    /// （`init_sync_objects` 只建一次），且 `current_frame` 只在整帧走完时才前进
    /// ⇒ 成功 acquire 之后任何提前 return 都会让**下一帧拿一个仍 signaled 的信号量去 acquire**
    /// （UB：`VUID-vkAcquireNextImageKHR-semaphore-01286` / `-01779`，本机默认不开验证层 ⇒ 静默）。
    ///
    /// 旧写法正是在 `suboptimal` 处直接 `return Err("交换链过期")`。**红证**：该分支被判成失败，
    /// 于是在旧代码上"acquire suboptimal + present 成功"这一格不可能给出"已呈现"的结论。
    #[test]
    fn acquire_suboptimal_never_aborts_before_present() {
        // suboptimal 只登记重建意图：本帧照常 present，然后才重建
        assert_eq!(
            frame_action(true, PresentOutcome::Presented),
            FrameAction::RecreateAfterPresent
        );
        // 关键不变式：acquire 的 suboptimal 标志**永远不能**单独把这一帧判成失败
        assert_ne!(frame_action(true, PresentOutcome::Presented), FrameAction::Fail);
        // present 自己说 suboptimal/out-of-date 时同样是"先呈现再重建"
        assert_eq!(
            frame_action(false, PresentOutcome::RecreateSwapchain),
            FrameAction::RecreateAfterPresent
        );
        assert_eq!(
            frame_action(true, PresentOutcome::RecreateSwapchain),
            FrameAction::RecreateAfterPresent
        );
        // 正常路径与真失败路径不受影响
        assert_eq!(
            frame_action(false, PresentOutcome::Presented),
            FrameAction::Presented
        );
        assert_eq!(frame_action(false, PresentOutcome::Failed), FrameAction::Fail);
        assert_eq!(frame_action(true, PresentOutcome::Failed), FrameAction::Fail);
    }

    /// acquire 的错误分类：只有"这轮没图像"能重试；OUT_OF_DATE/SURFACE_LOST 要重建；
    /// DEVICE_LOST 之类必须当失败（旧写法把它们全都静默丢进 `?` 的 map 里）。
    #[test]
    fn acquire_errors_are_classified() {
        assert_eq!(
            classify_acquire_err(vk::Result::TIMEOUT),
            AcquireOutcome::Retry
        );
        assert_eq!(
            classify_acquire_err(vk::Result::NOT_READY),
            AcquireOutcome::Retry
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_OUT_OF_DATE_KHR),
            AcquireOutcome::RecreateSwapchain
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_SURFACE_LOST_KHR),
            AcquireOutcome::RecreateSwapchain
        );
        assert_eq!(
            classify_acquire_err(vk::Result::ERROR_DEVICE_LOST),
            AcquireOutcome::Failed
        );
    }
}

/// `RV3D_GPU`（物理设备选择）的判据（2026-09-23 加）。
///
/// 起因：本机是双 GPU 笔记本，验证时 dGPU 可能被占（用户在跑 AI）⇒ 需要一个"强制走核显"的开关。
/// 这两条纯函数把"怎么解析"与"怎么挑"分开，于是**不需要真显卡就能单测**。
#[cfg(test)]
mod gpu_pick_tests {
    use super::{parse_gpu_preference, pick_physical_device, GpuPreference};
    use ash::vk;

    fn dgpu() -> (vk::PhysicalDeviceType, String) {
        (
            vk::PhysicalDeviceType::DISCRETE_GPU,
            "NVIDIA GeForce RTX 5060 Laptop GPU".to_string(),
        )
    }
    fn igpu() -> (vk::PhysicalDeviceType, String) {
        (
            vk::PhysicalDeviceType::INTEGRATED_GPU,
            "AMD Radeon 610M (integrated)".to_string(),
        )
    }

    #[test]
    fn parse_gpu_preference_maps_known_aliases() {
        assert_eq!(parse_gpu_preference(None), GpuPreference::Auto);
        assert_eq!(parse_gpu_preference(Some("")), GpuPreference::Auto);
        assert_eq!(parse_gpu_preference(Some("   ")), GpuPreference::Auto, "空白 = 未设");
        assert_eq!(parse_gpu_preference(Some("igpu")), GpuPreference::Integrated);
        assert_eq!(parse_gpu_preference(Some("IGPU")), GpuPreference::Integrated);
        assert_eq!(
            parse_gpu_preference(Some("integrated")),
            GpuPreference::Integrated
        );
        assert_eq!(parse_gpu_preference(Some("dgpu")), GpuPreference::Discrete);
        assert_eq!(
            parse_gpu_preference(Some("discrete")),
            GpuPreference::Discrete
        );
        // 其它值 = 名字子串，统一转小写
        assert_eq!(
            parse_gpu_preference(Some("  Radeon ")),
            GpuPreference::Name("radeon".to_string())
        );
    }

    #[test]
    fn pick_physical_device_honours_preference() {
        let both = [dgpu(), igpu()];
        // 🔴 默认必须是独显（历史行为，不许因为加了开关而改变）
        assert_eq!(pick_physical_device(&both, &GpuPreference::Auto), Some(0));
        assert_eq!(pick_physical_device(&both, &GpuPreference::Discrete), Some(0));
        assert_eq!(pick_physical_device(&both, &GpuPreference::Integrated), Some(1));
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("radeon".into())),
            Some(1)
        );
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("nvidia".into())),
            Some(0)
        );
        // 匹配不到 ⇒ None（调用方据此**报错退出**，绝不静默回退到独显）
        assert_eq!(
            pick_physical_device(&both, &GpuPreference::Name("intel".into())),
            None
        );
        // 只有集显的机器上要独显 ⇒ 也是 None
        assert_eq!(
            pick_physical_device(&[igpu()], &GpuPreference::Discrete),
            None
        );
        // 顺序无关：集显在前的列表里 Auto 仍选独显
        let flipped = [igpu(), dgpu()];
        assert_eq!(pick_physical_device(&flipped, &GpuPreference::Auto), Some(1));
    }
}
