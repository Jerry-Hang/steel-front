// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

/// `RV3D_NO_GROUND_TEX=1` 时关掉地面微细节层（`light_data.flags.w` 保持 0）。
/// A/B 诊断门，与 `RV3D_NO_SHADOW` 同一套惯例。**读一次缓存住**：本函数在每帧构建
/// 光照 UBO 的路径上，不该每帧 `getenv`。
pub(crate) fn no_ground_detail_tex() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RV3D_NO_GROUND_TEX").as_deref() == Ok("1"))
}
/// 纯函数：相机到地形中心地面距离 → 基础 LOD 级别
/// （默认 Medium 画质；非测试构建下仅被 #[cfg(test)] 单元测试调用）
#[allow(dead_code)]
pub(crate) fn terrain_lod_for_distance(dist: f32) -> TerrainLod {
    terrain_lod_for_distance_with_params(dist, quality_params(QualityPreset::DEFAULT))
}
/// 纯函数：按画质参数计算相机到地形中心地面距离 → 基础 LOD 级别
pub(crate) fn terrain_lod_for_distance_with_params(dist: f32, params: QualityParams) -> TerrainLod {
    if dist < params.terrain_lod_high_end {
        TerrainLod::High
    } else if dist < params.terrain_lod_med_end {
        TerrainLod::Medium
    } else {
        TerrainLod::Low
    }
}
/// 纯函数（默认 Medium 画质）：距离 → (要绘制的网格级别, morph 进度 t∈[0,1])。
/// （非测试构建下仅被 #[cfg(test)] 单元测试调用）
#[allow(dead_code)]
pub(crate) fn terrain_lod_blend(dist: f32) -> (TerrainLod, f32) {
    terrain_lod_blend_with_params(dist, quality_params(QualityPreset::DEFAULT))
}
/// 纯函数：按画质参数计算距离 → (要绘制的网格级别, morph 进度 t∈[0,1])。
/// t 为该级网格顶点高度向下一级（更粗）曲面三角形插值的进度：
/// t=0 完全细曲面，t=1 完全等于下一级曲面（几何重合，切换无 popping）。
pub(crate) fn terrain_lod_blend_with_params(dist: f32, params: QualityParams) -> (TerrainLod, f32) {
    if dist < params.terrain_lod_high_end {
        let t = ((dist - params.terrain_lod_high_morph_start)
            / (params.terrain_lod_high_end - params.terrain_lod_high_morph_start))
        .clamp(0.0, 1.0);
        (TerrainLod::High, smooth_t(t))
    } else if dist < params.terrain_lod_med_end {
        let t = ((dist - params.terrain_lod_med_morph_start)
            / (params.terrain_lod_med_end - params.terrain_lod_med_morph_start))
        .clamp(0.0, 1.0);
        (TerrainLod::Medium, smooth_t(t))
    } else {
        (TerrainLod::Low, 1.0)
    }
}
/// 纯函数：画质预设 → 参数表
pub(crate) fn quality_params(preset: QualityPreset) -> QualityParams {
    match preset {
        QualityPreset::Low => QUALITY_PARAMS[0],
        QualityPreset::Medium => QUALITY_PARAMS[1],
        QualityPreset::High => QUALITY_PARAMS[2],
    }
}

// ============================================================
// PNG 截图（swapchain 图像读回，纯逻辑部分）
// ============================================================
/// `(本次呈现耗时, 含本次在内的连续卡顿次数)` → 处置。判据
/// `present_stall_classifies_and_clears`。
pub(crate) fn present_stall(present_us: u64, consecutive: u32) -> PresentStall {
    if present_us < PRESENT_STALL_US {
        PresentStall::Ok
    } else if consecutive < PRESENT_STALL_FALLBACK {
        PresentStall::Warn
    } else {
        PresentStall::Degrade
    }
}
/// 连续卡顿计数的推进（纯函数）。**正常帧必须清零** ——
/// 漏了清零，几次**偶发**长卡顿会累积成"连续三次"从而误降级。
///
/// 🔴 单独抽出来的理由是**让测试真的钉住调用方**：状态机若写在测试里的局部闭包里，
/// 真实调用方忘了清零时测试照样绿（那只是把意图抄了一遍，不是判据）。
/// 抽成纯函数后，线上与测试跑的是同一份逻辑。
pub(crate) fn next_stall_count(current: u32, present_us: u64) -> u32 {
    match present_stall(present_us, current.saturating_add(1)) {
        PresentStall::Ok => 0,
        _ => current.saturating_add(1),
    }
}
/// 是否已经该判定 GPU 侧卡死（纯函数，可单测）
pub(crate) fn fence_stall_due(timeouts: u32) -> bool {
    timeouts >= FENCE_STALL_MAX
}
/// 纯函数：swapchain 像素格式 → 字节序策略。
/// 支持 B8G8R8A8 / R8G8B8A8 的 UNORM/SRGB 四种格式；SRGB 仅影响解码语义，
/// 存储字节序与 UNORM 相同，写 PNG 时保持原始编码；未知格式返回 Err。
pub(crate) fn pixel_order_for_format(format: vk::Format) -> Result<PixelOrder, String> {
    match format {
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => Ok(PixelOrder::Bgra),
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => Ok(PixelOrder::Rgba),
        _ => Err(format!("不支持的交换链像素格式: {:?}", format)),
    }
}
/// 纯函数：把 staging buffer 中的像素字节流（swapchain 格式字节序）转换为 RGBA8。
/// src/dst 长度必须相等且为 4 的倍数（每像素 4 字节）；未知格式返回 Err。
pub(crate) fn convert_pixels_to_rgba(format: vk::Format, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    if src.len() != dst.len() || src.len() % 4 != 0 {
        return Err("像素缓冲区长度非法".to_string());
    }
    match pixel_order_for_format(format)? {
        PixelOrder::Bgra => {
            for (chunk, out) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                out[0] = chunk[2];
                out[1] = chunk[1];
                out[2] = chunk[0];
                out[3] = chunk[3];
            }
        }
        PixelOrder::Rgba => dst.copy_from_slice(src),
    }
    Ok(())
}
/// 解析 `RV3D_GPU`：`igpu`/`integrated` → 集显；`dgpu`/`discrete` → 独显；
/// 其它非空值 → **按设备名子串匹配**（大小写不敏感，例如 `RV3D_GPU=radeon`）；空/未设 → `Auto`。
pub(crate) fn parse_gpu_preference(raw: Option<&str>) -> GpuPreference {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return GpuPreference::Auto;
    };
    match s.to_ascii_lowercase().as_str() {
        "igpu" | "integrated" => GpuPreference::Integrated,
        "dgpu" | "discrete" => GpuPreference::Discrete,
        other => GpuPreference::Name(other.to_string()),
    }
}
/// 设备类型排序权重（`Auto` 用：独显 2 > 集显 1 > 其它 0）
pub(crate) fn gpu_type_rank(t: vk::PhysicalDeviceType) -> u8 {
    match t {
        vk::PhysicalDeviceType::DISCRETE_GPU => 2,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
        _ => 0,
    }
}
/// 从候选里挑一个设备（**纯函数，可单测**）：返回下标；匹配不到返回 `None`
/// （调用方据此**报错退出**，不静默回退 —— 见调用点注释）。
pub(crate) fn pick_physical_device(
    candidates: &[(vk::PhysicalDeviceType, String)],
    pref: &GpuPreference,
) -> Option<usize> {
    match pref {
        GpuPreference::Discrete => candidates
            .iter()
            .position(|(t, _)| *t == vk::PhysicalDeviceType::DISCRETE_GPU),
        GpuPreference::Integrated => candidates
            .iter()
            .position(|(t, _)| *t == vk::PhysicalDeviceType::INTEGRATED_GPU),
        GpuPreference::Name(want) => candidates
            .iter()
            .position(|(_, n)| n.to_ascii_lowercase().contains(want.as_str())),
        // 与历史行为一致：`max_by_key` 取"最大的那个"，并列时取**最后一个**
        GpuPreference::Auto => candidates
            .iter()
            .enumerate()
            .max_by_key(|(_, (t, _))| gpu_type_rank(*t))
            .map(|(i, _)| i),
    }
}
/// 纯函数：`queue_present` 的返回值 → 处置方式。判据见 `PresentOutcome`。
pub(crate) fn classify_present(result: Result<bool, vk::Result>) -> PresentOutcome {
    match result {
        Ok(false) => PresentOutcome::Presented,
        Ok(true) => PresentOutcome::RecreateSwapchain,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => PresentOutcome::RecreateSwapchain,
        Err(_) => PresentOutcome::Failed,
    }
}
pub(crate) fn classify_acquire_err(e: vk::Result) -> AcquireOutcome {
    match e {
        vk::Result::TIMEOUT | vk::Result::NOT_READY => AcquireOutcome::Retry,
        vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR => {
            AcquireOutcome::RecreateSwapchain
        }
        _ => AcquireOutcome::Failed,
    }
}
/// 纯函数：`(acquire 是否 SUBOPTIMAL, present 的结果)` → 本帧处置。
///
/// 🔴 2026-09-25 深夜复查补：**成功 acquire 之后、present 之前，不许再 return**。
///
/// `acquire_next_image` 成功 ⇒ `image_available_semaphores[current_frame]` 已被 signal，
/// 而这张图像只有走完 present 才会被交还；那个信号量是**渲染器生命周期对象**
/// （`init_sync_objects` 只建一次，`recreate_swapchain` 不会重建它），而 `current_frame`
/// 也只在整帧走完时才前进 ⇒ 成功 acquire 之后提前 return，会让**下一帧拿一个仍 signaled
/// 的二值信号量去 acquire** —— 未定义行为（`VUID-vkAcquireNextImageKHR-semaphore-01286`：
/// semaphore 必须 unsignaled；相关条 `-01779`：不得有未完成的 signal/wait）。本机默认不开
/// 验证层（`RV3D_VALIDATION=1` 才开）⇒ 这类问题**完全静默**，本仓历史上最贵的一类 bug 同形。
///
/// 旧写法把"acquire 说 suboptimal"与"present 说 suboptimal"混成了同一件事：前者直接
/// `return Err("交换链过期")` **丢掉了已经拿到手的图像**。现在两件事分开 ——
/// acquire 的 suboptimal 只是一个**登记**（本帧照常 record/submit/present），
/// 重建统一发生在 present 之后（与 present 自己返回 SUBOPTIMAL / OUT_OF_DATE 合流）。
///
/// 签名本身承载这条不变式：**present 结果必须作为参数传进来**，
/// 也就是"想返回 `RecreateAfterPresent` 就必须先 present 过"。
pub(crate) fn frame_action(acquire_suboptimal: bool, present: PresentOutcome) -> FrameAction {
    match present {
        PresentOutcome::Failed => FrameAction::Fail,
        PresentOutcome::RecreateSwapchain => FrameAction::RecreateAfterPresent,
        PresentOutcome::Presented if acquire_suboptimal => FrameAction::RecreateAfterPresent,
        PresentOutcome::Presented => FrameAction::Presented,
    }
}
/// 这一帧要不要**跳过渲染**（纯函数，可单测）。
///
/// 两种降级都必须跳过：① `gpu_stalled`（围栏连续超时 ⇒ GPU 侧已卡死，再提交也只是白等
/// 5 秒）；② `swapchain_broken`（**重建交换链中途失败** ⇒ `swapchain` 等句柄可能已被
/// `destroy_swapchain` 销毁，再 acquire/提交就是拿空句柄调 Vulkan）。
/// 两者都保持进程与输入响应；后者在下一次重建**成功**时自动清除（尺寸变化 / 5 秒
/// 尺寸自检都会重试）。
pub(crate) fn frame_suppressed(gpu_stalled: bool, swapchain_broken: bool) -> bool {
    gpu_stalled || swapchain_broken
}
/// 这一帧要不要重画阴影图（纯函数，可单测）。见 `Renderer::shadow_every`：隔帧重画，
/// `every = 1` 就是每帧（A/B 对照）；`void_mode`（检视模式）下从不画。
pub(crate) fn shadow_due(frame_seq: u64, every: u32, void_mode: bool) -> bool {
    !void_mode && frame_seq % every.max(1) as u64 == 0
}
/// **静态**阴影图这一帧要不要重画（纯函数，可单测）。
///
/// 静态图只装世界不动的那批投射者（地形/地面场/marker/道具），所以它只需要偶尔重画：
/// `every` 帧一次（默认 30 ≈ 半秒，安全网），`split = false`（A/B 关掉拆分）时不单独画
/// —— 那条路走"单张图、每帧两类投射者一起画"的旧逻辑。`void_mode`（检视模式）下从不画。
pub(crate) fn shadow_static_due(frame_seq: u64, every: u64, void_mode: bool, split: bool) -> bool {
    split && !void_mode && frame_seq % every.max(1) == 0
}
/// surface 未给出 `current_extent`（= `u32::MAX`）时的**兜底**尺寸：必须夹进
/// `[min_image_extent, max_image_extent]`（纯函数，可单测）。
///
/// Vulkan 规定 `current_extent` 未定义时 `imageExtent` 必须落在 surface 给的范围里
/// （`VUID-VkSwapchainCreateInfoKHR-imageExtent-01274`）。旧代码在这一支直接写死 `1280x720` ——
/// 只要某台设备的 `maxImageExtent` 比它小，创建交换链当场违规；而**本机 Win32 surface 恒给出
/// 具体尺寸，所以这条分支在默认配置上永远走不到**（与 §21.23 的 PT blit 写死 2560x1600 是同一类：
/// 默认配置恰好等于那个写死的值，于是缺陷永远被躲过去）。
///
/// 退化输入也要挡住：驱动若报 0 或 `max < min`，宁可退到 `min`（且不小于 1），
/// 也不要提交一个 `imageExtent = 0` 的交换链 —— 那没有兜底路径。
pub(crate) fn clamp_swapchain_extent(
    fallback: vk::Extent2D,
    min: vk::Extent2D,
    max: vk::Extent2D,
) -> vk::Extent2D {
    let lo_w = min.width.max(1);
    let lo_h = min.height.max(1);
    let hi_w = max.width.max(lo_w);
    let hi_h = max.height.max(lo_h);
    vk::Extent2D {
        width: fallback.width.clamp(lo_w, hi_w),
        height: fallback.height.clamp(lo_h, hi_h),
    }
}
/// 决定交换链到底用哪个尺寸 —— **窗口尺寸优先于那个写死的 1280x720**。
///
/// 🔴 **2026-09-28 修（Linux 适配，Wayland 阻断级）**：`currentExtent` 在
/// **Wayland 下是设计上就未定义的**（Mesa 明确填 `{UINT32_MAX, UINT32_MAX}`，见
/// `wsi_common_wayland.c::wsi_wl_surface_get_capabilities`），而在 Win32 / X11
/// 下它**恒等于窗口尺寸** ⇒ 这条兜底分支在 Windows 上**永远走不到**，于是
/// 「兜底 = 写死 1280x720」这个值一路漂到 Linux 才暴露，后果是三重错位：
/// 交换链恒为 1280x720、投影用窗口尺寸、HUD 排版也用窗口尺寸 ⇒ 画面比例错乱、
/// HUD 按窗口排版却画进 720p 视口；而 `Resized` 的重建**走的还是同一段代码**
/// ⇒ **永远不跟随窗口**。
///
/// 这与 §21.23 的「PT blit 写死 2560x1600」是同一类缺陷：默认配置恰好等于那个
/// 写死的值，于是缺陷永远被躲过去。
///
/// 语义（判据 `swapchain_extent_follows_the_window_when_current_extent_is_undefined`）：
/// - `currentExtent.width != u32::MAX`（Win32 / X11）⇒ **原样返回**，
///   与修复前逐字节一致，Windows 侧行为不变；
/// - 未定义（Wayland）⇒ 用**调用方给的窗口物理尺寸**；
/// - 窗口尺寸不可用（0，Wayland 首个 configure 之前）⇒ 才退到 1280x720 兜底。
///
/// 后两条都要夹进 `min/maxImageExtent`（`VUID-VkSwapchainCreateInfoKHR-imageExtent-01274`）。
pub(crate) fn swapchain_extent_choice(
    current: vk::Extent2D,
    window: vk::Extent2D,
    min: vk::Extent2D,
    max: vk::Extent2D,
) -> vk::Extent2D {
    if current.width != u32::MAX {
        return current;
    }
    let fallback = if window.width > 0 && window.height > 0 {
        window
    } else {
        vk::Extent2D {
            width: 1280,
            height: 720,
        }
    };
    clamp_swapchain_extent(fallback, min, max)
}
/// 设备扩展选择：**缺扩展要降级，不要让 `create_device` 直接失败**。
///
/// 🔴 **2026-09-29 修**。原代码在 `VK_EXT_mesh_shader` 可用时**无条件**追加 5 个光追扩展，
/// 而 `enumerate_device_extension_properties` 的结果只在**事后**用来打一行 warn ——
/// 于是"设备缺任一光追扩展"的后果不是降级，而是 **`create_device` 失败 = 游戏起不来**，
/// 而 PT 本来就是**默认关**的，根本不值得为它挡住启动。
/// 同一段还有第二处：`PhysicalDeviceRayQueryFeaturesKHR` 等**特性结构无条件挂进 pNext**，
/// 即使对应扩展没启用 —— 那本身就是无效用法。
///
/// 语义（判据 `device_extensions_degrade_instead_of_failing`）：
/// - `required`：逐个按实际支持情况启用；缺的**如实报出来**（是否致命由调用方决定，
///   本函数不替它决定 —— 这里只负责"别把可选的东西当必需"）；
/// - `all_or_nothing`：**全有或全无**的一组。
///   🔴 光追那 5 个必须同进同退：只启用一半时，特性链与后续代码路径都假设它们齐全，
///   半套就是未定义行为 —— 比"整组不用"危险得多。
///   ⇒ 缺任何一个就**整组不启用**，并把缺的那些报出来。
pub(crate) fn pick_device_extensions<'a>(
    available: &[String],
    required: &[&'a str],
    all_or_nothing: &[&'a str],
) -> (Vec<&'a str>, Vec<&'a str>) {
    let has = |n: &str| available.iter().any(|a| a == n);
    let mut enabled: Vec<&'a str> = required.iter().copied().filter(|n| has(n)).collect();
    let mut missing: Vec<&'a str> = required.iter().copied().filter(|n| !has(n)).collect();
    let absent: Vec<&'a str> = all_or_nothing.iter().copied().filter(|n| !has(n)).collect();
    if absent.is_empty() {
        enabled.extend(all_or_nothing.iter().copied());
    } else {
        // 整组不启用：把缺的那些报出去（**已存在的那几个也算"没启用"**，
        // 因为调用方要的是"这一组能不能用"，不是"哪几个名字恰好存在"）
        missing.extend(absent);
    }
    (enabled, missing)
}
/// 现在值不值得再试一次交换链重建（纯逻辑，可单测；"距上次尝试多久"由调用方折成秒）。
///
/// - 设备丢失 ⇒ **永远不值得**：本引擎没有重建设备的路径，重试只会失败。实测（2026-09-26）
///   设备被打掉之后，`main.rs` 的尺寸自检**每帧**重试重建，12 秒跑 1961 轮、刷 5900 行
///   错误日志，而进程看着还活着 —— "看起来在跑、其实一帧都画不出来"正是本仓最反对的静默。
/// - 上一次刚失败（< `RECREATE_RETRY_MIN_SECS`）⇒ 先不试（限流，别刷屏）；
/// - 其余 ⇒ 值得（重建成功即自动恢复 `swapchain_broken`）。
pub(crate) fn should_retry_swapchain(device_lost: bool, swapchain_broken: bool, secs_since_attempt: f32) -> bool {
    if device_lost {
        return false;
    }
    !(swapchain_broken && secs_since_attempt < RECREATE_RETRY_MIN_SECS)
}
/// 这条错误串是不是"设备丢了"（纯函数，可单测）。
///
/// 引擎各层的错误类型都是 `String`（只做前缀拼接），`VK_ERROR_DEVICE_LOST` 到这一层
/// 只剩 ash 的 Display 文本 `The logical device has been lost.` ⇒ 只能按子串判。
/// 判据独立成函数是为了**只有一处**定义"什么算不可恢复"。
pub(crate) fn is_device_lost_error(msg: &str) -> bool {
    msg.contains("device has been lost") || msg.contains("DEVICE_LOST")
}
/// `device_wait_idle` 失败时该打的日志（纯函数，可单测）：`None` = 这次不打。
///
/// 🔴 2026-09-26 复查发现全仓 6 处 `let _ = self.device.device_wait_idle();`
/// （`set_first_person_gun_mesh` / `set_props` ×2 / `set_shadow_props` /
/// `pt_set_scene_markers` / `Drop`）—— 全都是"**等空闲 → 销毁/重建在飞资源**"的关键路径，
/// 而**等待失败与等待成功在日志里长得一模一样**：后面那句 `destroy_buffer` 到底安不安全，
/// 排查的人手里没有任何证据。而这条命令最可能返回的错误正好是 `VK_ERROR_DEVICE_LOST`
/// ——本仓最贵的一类故障（铁律 B「设备丢失 = 不可恢复」）。
///
/// 消息**必须点名错误**：否则 `device lost` 与 `out of host memory` 在日志里分不清，
/// 而这两者的处置完全相反（前者不可恢复、后者可以少要点资源重试）。
/// 用闩而不是每次都记：这几条路径在换枪 / 重载 / 进出 PT 时反复跑。
///
/// 判据 = `wait_idle_failure_is_named_and_reported_once` +
/// `device_wait_idle_errors_are_never_silently_dropped`。
pub(crate) fn wait_idle_failure_message(err: vk::Result, already_warned: bool) -> Option<String> {
    if already_warned {
        return None;
    }
    Some(format!(
        "gpu: device_wait_idle 失败（{err:?}）—— 紧接着的 destroy/create 都发生在**尚未确认空闲**的\
         设备上；若为 device lost 则不可恢复，见铁律 B"
    ))
}
/// 道具缓冲要不要（重新）创建 —— 纯逻辑，可单测。
///
/// 判据是 `need > capacity`，**不是** `need != capacity`：写成 `!=` 时"道具变少"也会重建，
/// 而重建等于 destroy 正在被在飞帧与 PT BLAS 引用的缓冲。枪模那条路为此付过代价
/// （2026-09-15 实测按一下 2 = 整台设备消失，见 `set_first_person_gun_mesh` 的注释）。
pub(crate) fn prop_buffer_growth_needed(
    need_v: u32,
    cap_v: u32,
    need_i: u32,
    cap_i: u32,
    mapped_ok: bool,
) -> bool {
    !mapped_ok || need_v > cap_v || need_i > cap_i
}
/// 启动时要不要构建 PT 常驻资源（纯函数，可单测）。
///
/// 🔴 2026-09-25 修：`RV3D_PT_LIVE=1` 自称"强制开"，但它**只**改 `pt_live_enabled`，
/// 而 PT 真正出画还要求 `pt_resident.is_some()`（判据见 `render()` 里那句
/// `if self.pt_live_enabled && self.pt_resident.is_some()`），常驻资源却只在
/// `config.pt_enable == true` 时构建 ⇒ **只设环境变量时 PT 一帧都跑不出来**，
/// 外面只看到"开关写着开了、画面没变"。那次"PT 验证"因此什么也没验到（§21.17）。
///
/// 现在三态一致：`1` 强制开（含常驻资源）、`0` 强制关（连资源都不建，省显存）、
/// 未设时跟随配置。
pub(crate) fn pt_resident_needed(configured: bool, live_env: Option<&str>) -> bool {
    match live_env {
        Some("0") => false,
        Some("1") => true,
        _ => configured,
    }
}
/// PT 实时渲染分辨率（纯函数，可单测）：`(窗口宽, 窗口高, RV3D_PT_SIZE)` → `(w, h)`。
///
/// 三态：`RV3D_PT_SIZE` 合法（128..=4096 且 8 的倍数）时**等比**缩放到该宽度；
/// 未设或非法时跟随窗口；两者都对齐到 8 的倍数（`%8==0` 是 PT 图像的硬要求），
/// 且窗口退化为 0 时也不会返回 0（驱动不接受 0 尺寸）。
///
/// 🔴 2026-09-25 修：注释一直写着"单值覆盖（**等比**）"，但实现只改了宽、高取窗口高，
/// 于是 `RV3D_PT_SIZE=512` 在 2560x1600 的窗口上得到 **512x1600** 的压扁图 ——
/// PT 参照帧与功耗 A/B 都因此失去可比性（判据 = 本函数上方那条单测）。
pub(crate) fn pt_render_extent(win_w: u32, win_h: u32, size_env: Option<u32>) -> (u32, u32) {
    let snap = |v: u32| (v & !7).max(8);
    let (w, h) = (snap(win_w.max(64)), snap(win_h.max(64)));
    match size_env {
        Some(s) if (128..=4096).contains(&s) && s % 8 == 0 => {
            // 等比：高 = 窗口高 × (s / 窗口宽)，再对齐 8
            let scaled = (h as f32 * (s as f32 / w as f32)).round() as u32;
            (s, snap(scaled.max(8)))
        }
        _ => (w, h),
    }
}
/// 某顶点 (x,z) 在下一级（更粗）网格曲面上的高度：
/// 先定位所在粗网格 cell，再用与地形索引一致的三角形剖分做重心插值。
/// 粗网格点与细网格点重合处返回值与该点粗网格高度完全一致。
pub(crate) fn terrain_coarse_height(x: f32, z: f32, coarse: &[f32], coarse_cells: usize) -> f32 {
    let cell = 512.0 / coarse_cells as f32;
    let uf = (x + TERRAIN_HALF) / cell;
    let vf = (z + TERRAIN_HALF) / cell;
    let cw = coarse_cells + 1;
    let cx = (uf.floor().max(0.0) as usize).min(coarse_cells - 1);
    let cz = (vf.floor().max(0.0) as usize).min(coarse_cells - 1);
    let u = uf - cx as f32;
    let v = vf - cz as f32;
    let h00 = coarse[cz * cw + cx];
    let h10 = coarse[cz * cw + cx + 1];
    let h01 = coarse[(cz + 1) * cw + cx];
    let h11 = coarse[(cz + 1) * cw + cx + 1];
    if u >= v {
        // 三角形 (v0,v1,v2)：右下三角
        (1.0 - u) * h00 + (u - v) * h10 + v * h11
    } else {
        // 三角形 (v0,v3,v2)：左上三角
        (1.0 - v) * h00 + (v - u) * h01 + u * h11
    }
}
/// 纯函数：worker 数 → 并行剔除段数（调用线程参与首段，故 +1），并以
/// [`CULL_MAX_SEGMENTS`] 为上限 —— 段数只影响并行度，不影响结果（前缀和按段相加，
/// 段边界怎么切都不改变可见集合与近/远分档）。**不碰线程调度策略**：池的拓扑、
/// 亲和、降频都在 `cpu.rs`，这里只决定"切几段"。
pub(crate) fn cull_segment_count(workers: usize) -> usize {
    (workers + 1).min(CULL_MAX_SEGMENTS)
}
/// 道具分桶边长（米）。全城约 ±175m ⇒ 约 9×9 格。
/// 🔴 2026-09-12 第 44 轮：由 40m 改为 20m。**原注释的理由已被实测推翻** ——
/// 「桶再小则 draw call 数上升（每桶一次 cmd_draw_indexed 与其绑定开销）」
/// 这条经 `RV3D_ONE_PROP_DRAW` 对照否定：把 28 个可见桶合并成 **1 个** draw
/// 反而**更慢**（126.7 → 82.6 fps，wait_fence 4164 → 9006µs），
/// 因为它没剔除、多画了 2.7 倍顶点。**draw call 数不是瓶颈。**
/// 而第 43 轮同时证明成本与**绘制的顶点数近似成正比**（2.69× 顶点 → 2.14× 时间）。
/// ⇒ 该优化的是"每帧画多少顶点"，切细桶正是为此；draw call 上升是安全的代价。
// 🔴 2026-09-12 第 104 轮：20.0 → **10.0**。
//
// 第 103 轮的同场 A/B 定案：**AI 侧 CPU 优化不再涨帧**（省 196µs `ai_us`，fps 纹丝不动）——
// 因为同期 `wait_fence_us` 3000~4000 = **CPU 在等 GPU**。⇒ 第②条只剩 GPU 侧，而**道具是最大分项
// （3.20ms / 34%）**，成本随**画的顶点数**走（第 44 轮实测：2.69x 顶点 → 2.14x 时间）。
//
// 分桶更细 ⇒ 每帧可见桶覆盖的顶点更少 ⇒ 直接减 GPU 顶点吞吐。**绘制调用数不是瓶颈**
// （第 44 轮实测：单桶全画反而更慢，82.6 fps）⇒ 细分安全。
// 第 44 轮 40→20 的已验证收益：三角形 −14%、顶点跨度 −15%、`wait_fence_us` 4164→2、fps 126.7→131.7。
// 🔴 2026-09-12：20.0 → **10.0**（第 104 轮，**实测 +1.35% fps**：中位 133.1→134.9、最差帧 125.6→131.4）。
//
// ⚠️ **5.0 试过并已按预先声明的判据退回**（第 105 轮）：中位 fps 一模一样（134.9），
// **而最差帧从 131.4 退到 125.5**。同场取证：桶 164/546 可见、提交三角形 199190、
// 顶点区间 474508（占顶点总数 1563020 的 **30%**）。
// ⇒ **10.0 是这条杠杆的拐点**：再细分只会增加绘制调用与剔除开销，不再减少顶点吞吐。
// **不要再往下调，除非先证明瓶颈已从"顶点吞吐"变成别的。**
/// 障碍种类 → 基础材质色（片元 marker 路径直出 tint × fade，无贴图混合）
pub(crate) fn obstacle_base_color(kind: ObstacleKind) -> [f32; 3] {
    match kind {
        ObstacleKind::Wall => [0.66, 0.38, 0.30], // 砖墙红
        ObstacleKind::Block => [0.46, 0.50, 0.56], // 金属掩体（钢灰蓝）
        ObstacleKind::Barrier => [0.60, 0.45, 0.28], // 木板路障
        ObstacleKind::Tree => [0.48, 0.36, 0.22], // 树干棕
        ObstacleKind::Building => [0.58, 0.58, 0.61], // 混凝土
        ObstacleKind::Ruin => [0.44, 0.39, 0.33], // 残骸土棕
    }
}
/// 障碍 marker 材质 tint：种类基础色 × 确定性逐障碍微变（1/8m 量化格点哈希）。
/// 同一障碍每帧/每关颜色恒定；三通道各自独立抖动形成材质颗粒感。
pub(crate) fn obstacle_material_tint(kind: ObstacleKind, x: f32, z: f32) -> [f32; 4] {
    let base = obstacle_base_color(kind);
    let qx = (x * 8.0).round() as i32;
    let qz = (z * 8.0).round() as i32;
    let unit = |ix: i32, iz: i32| (terrain_hash(ix, iz) & 0xFFFF) as f32 / 65535.0;
    let jr = 0.94 + 0.12 * unit(qx + 1, qz);
    let jg = 0.94 + 0.12 * unit(qx, qz + 1);
    let jb = 0.94 + 0.12 * unit(qx, qz);
    [
        (base[0] * jr).clamp(0.0, 1.0),
        (base[1] * jg).clamp(0.0, 1.0),
        (base[2] * jb).clamp(0.0, 1.0),
        1.0,
    ]
}
/// Hermite 平滑插值（LOD morph 过渡系数 / 地形抬升 / 值噪声插值共用）
pub(crate) fn smooth_t(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
/// 确定性整数哈希（纯 u32 算术，跨平台逐位一致；值噪声格点采样用）
pub(crate) fn terrain_hash(ix: i32, iz: i32) -> u32 {
    let mut h = (ix as u32).wrapping_mul(0x1B873593) ^ (iz as u32).wrapping_mul(0xCC9E2D51);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846CA68B);
    h ^= h >> 16;
    h
}
/// 格点伪随机高度：[-1, 1)
pub(crate) fn terrain_lattice_height(ix: i32, iz: i32) -> f32 {
    (terrain_hash(ix, iz) & 0xFFFF) as f32 / 32768.0 - 1.0
}
/// 双线性 smoothstep 值噪声（确定性、低频平缓、C1 连续）
pub(crate) fn terrain_value_noise(x: f32, z: f32, cell: f32) -> f32 {
    let fx = x / cell;
    let fz = z / cell;
    let ix = fx.floor() as i32;
    let iz = fz.floor() as i32;
    let tx = smooth_t(fx - ix as f32);
    let tz = smooth_t(fz - iz as f32);
    let h00 = terrain_lattice_height(ix, iz);
    let h10 = terrain_lattice_height(ix + 1, iz);
    let h01 = terrain_lattice_height(ix, iz + 1);
    let h11 = terrain_lattice_height(ix + 1, iz + 1);
    let a = h00 + (h10 - h00) * tx;
    let b = h01 + (h11 - h01) * tx;
    a + (b - a) * tz
}
/// 地形高度：半径 ≤ TERRAIN_FLAT_RADIUS（中央 60×60 安全区、障碍环带 58–130m、
/// 两军接火区都落在此圆内）恒 y=0；之外按距离 smoothstep 抬升的确定性值噪声丘陵，
/// 幅值 ≤ TERRAIN_HILL_AMPLITUDE、坡度平缓（LOD morph 无突兀）。
pub(crate) fn terrain_height(x: f32, z: f32) -> f32 {
    let flat_r2 = TERRAIN_FLAT_RADIUS * TERRAIN_FLAT_RADIUS;
    let r2 = x * x + z * z;
    if r2 <= flat_r2 {
        return 0.0;
    }
    let t = ((r2.sqrt() - TERRAIN_FLAT_RADIUS) / TERRAIN_HILL_RAMP).clamp(0.0, 1.0);
    smooth_t(t) * TERRAIN_HILL_AMPLITUDE * terrain_value_noise(x, z, TERRAIN_HILL_CELL)
}
/// 供 CPU 侧（NPC/实例）查询地形高度，与 GPU 地形完全同源
pub(crate) fn terrain_height_at(x: f32, z: f32) -> f32 {
    terrain_height(x, z)
}

// ============================================================
// 渲染器
// ============================================================
pub(crate) fn load_spirv(path: &str) -> Result<Vec<u32>, String> {
    // 2026-10-09：改走资产抽象层（Android 侧将换 AAssetManager，见 engine/asset_source.rs）。
    let bytes = crate::engine::asset_source::global()
        .read(path)
        .map_err(|e| format!("打开着色器文件失败 '{}': {}", path, e))?;
    let mut cur = std::io::Cursor::new(bytes);
    util::read_spv(&mut cur).map_err(|e| format!("读取 SPIR-V 文件失败 '{}': {}", path, e))
}
/// POD → &[u8]（push constants 上传，零外部依赖）
#[inline]
pub(crate) fn bytemuck_bytes<T: Sized>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}
/// PtBox.material → 反照率（与 WorldMarker 障碍调色板同源，PT 才可当烘焙参照）
pub(crate) fn pt_albedo_of(b: &crate::engine::ray_tracer::PtBox) -> [f32; 3] {
    let k = match b.material {
        1 => ObstacleKind::Building,
        2 => ObstacleKind::Block,
        3 => ObstacleKind::Tree,
        _ => return [0.34, 0.32, 0.29],
    };
    obstacle_base_color(k)
}
/// PT 道具逐三角属性表烘焙（纯函数，判据见 `pt_prop_attrs_tests`）：
/// 每三角 2×u32 = 量化面法线（(v·127+127) 每轴 u8）+ 平均顶点色（u8×3）。
/// 法线由世界坐标顶点直接算（道具是闭合壳，着色器再翻到迎向来射侧，绕序无关）；
/// 退化三角形回退 [0,1,0]（NaN 钳黑教训同源）。不足一整三角的尾索引被丢弃，
/// 由 build_pt_as 的 `prop_attr_tris*3 == prop_index_count` 等式把关兜底。
pub(crate) fn pt_bake_prop_attrs(verts: &[[f32; 11]], indices: &[u32]) -> Vec<u32> {
    let ntri = indices.len() / 3;
    let mut attrs: Vec<u32> = Vec::with_capacity(ntri * 2);
    let qn = |v: f32| -> u32 { (v.clamp(-1.0, 1.0) * 127.0 + 127.0).round() as u32 & 0xFF };
    let qc = |v: f32| -> u32 { (v.clamp(0.0, 1.0) * 255.0).round() as u32 & 0xFF };
    for tri in indices.chunks_exact(3) {
        let vp = |k: usize, c: usize| verts[tri[k] as usize][c];
        let e1 = [vp(1, 0) - vp(0, 0), vp(1, 1) - vp(0, 1), vp(1, 2) - vp(0, 2)];
        let e2 = [vp(2, 0) - vp(0, 0), vp(2, 1) - vp(0, 1), vp(2, 2) - vp(0, 2)];
        let mut n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let ln = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if ln > 1e-7 {
            n = [n[0] / ln, n[1] / ln, n[2] / ln];
        } else {
            n = [0.0, 1.0, 0.0];
        }
        let w0 = qn(n[0]) | (qn(n[1]) << 8) | (qn(n[2]) << 16);
        let cr = (vp(0, 8) + vp(1, 8) + vp(2, 8)) / 3.0;
        let cg = (vp(0, 9) + vp(1, 9) + vp(2, 9)) / 3.0;
        let cb = (vp(0, 10) + vp(1, 10) + vp(2, 10)) / 3.0;
        let w1 = qc(cr) | (qc(cg) << 8) | (qc(cb) << 16);
        attrs.push(w0);
        attrs.push(w1);
    }
    attrs
}
/// 场景指纹（坐标量化到 1m）：只有盒集合真的变了才重建 BLAS，避免逐帧重建
pub(crate) fn pt_scene_sig(boxes: &[crate::engine::ray_tracer::PtBox]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    {
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(boxes.len() as u64);
        for b in boxes {
            for c in b.center.iter().chain(b.half.iter()) {
                mix(*c as i32 as u32 as u64);
            }
            mix(b.material as u64);
        }
    }
    h
}
