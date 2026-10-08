// 由 src/engine/renderer/tests_vk.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `tests_vk` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
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
