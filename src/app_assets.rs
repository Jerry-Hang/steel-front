// 由 src/lib.rs 拆出（2026-11-11，纯移动 + 仅加 pub(crate)）：角色/世界资产的装载入口。
//
// 目前放**士兵 GLB 的加载与缓存**（`load_soldier_glb` 只做查缓存，未命中才走 `_uncached`）。
// 与 `app_gun` 里的枪模加载对称：两者都是"外部资产 → 引擎网格"的同一条流水线。
use crate::GameApp;

impl GameApp {
    pub(crate) fn load_soldier_glb() -> Option<(Vec<[f32; 11]>, Vec<u32>)> {
        // 画质 M2：士兵 GLB 若没带 NORMAL（与道具同样的合并流程），加载侧补平滑法线。

        // 🔴🔴 2026-09-13：**必须缓存**。调用点在每帧的渲染准备段里，而这个是
        // "读盘 + 解析 GLB" —— 实测 12 秒内被调用 **1277 次**（每次 45KB 读盘 + 全量解析）。
        // 士兵网格只在启动时用一次，缓存后每帧只是取一个 `&`。
        // `OnceLock` 是 std 的，不引入依赖（本项目硬约束：不新增第三方依赖）。
        static CACHE: std::sync::OnceLock<Option<(Vec<[f32; 11]>, Vec<u32>)>> =
            std::sync::OnceLock::new();
        if let Some(cached) = CACHE.get() {
            // 只有 `Some` 需要克隆一次给调用方；`None` 直接返回。
            // 克隆发生在每帧，但 1082 个顶点 × 44 字节 ≈ 47KB —— 比读盘+解析便宜两个量级。
            return cached.clone();
        }
        let loaded = Self::load_soldier_glb_uncached();
        let _ = CACHE.set(loaded.clone());
        loaded
    }

    pub(crate) fn load_soldier_glb_uncached() -> Option<(Vec<[f32; 11]>, Vec<u32>)> {
        const PATH: &str = "assets/soldier/soldier.glb";
        // 🔴 **走资产抽象层**（2026-10-15，外部复核指出第 3 条，已确认）：
        // 原来这里是 `std::fs::read` ⇒ 在 Android（`AAssetManager`）与
        // `RV3D_ASSETS_FROM_MEMORY=1`（`MemSource`）下**必然失败**；
        // 而旁边同族的 `load_gun_glb` 在第 5 批已经改过来了
        // —— **同一件事两处实现、只改了一处**（本仓最贵的缺陷形态）。
        let bytes = match crate::engine::asset_source::global().read(PATH) {
            Ok(b) => b,
            Err(e) => {
                log::info!("soldier: 未发现 {PATH}（{e}），NPC 继续用 18 段箱体");
                return None;
            }
        };
        match crate::engine::assets::parse_glb(&bytes) {
            Ok(mesh) => {
                if mesh.verts.is_empty() || mesh.indices.is_empty() {
                    log::warn!("soldier: {PATH} 为空网格，NPC 继续用 18 段箱体");
                    return None;
                }
                let mut verts = mesh.verts;
                crate::engine::props::ensure_smooth_normals(&mut verts, &mesh.indices);
                log::info!(
                    "soldier: 载入 {PATH}（{} 顶点 / {} 索引，原始尺度与朝向，未归一化）",
                    verts.len(),
                    mesh.indices.len()
                );
                Some((verts, mesh.indices))
            }
            Err(e) => {
                log::warn!("soldier: {PATH} 解析失败（{e}），NPC 继续用 18 段箱体");
                None
            }
        }
    }
}
