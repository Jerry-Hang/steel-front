# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# ① 字段：gun_glb → gun_glbs 缓存
s = s.replace("    /// 导入的 GLB 枪模（assets/guns/*.glb → 烘焙顶点；无则回退程序化枪模）\n    gun_glb: Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)>,",
"    /// 导入的 GLB 枪模缓存（按武器 key：assets/guns/{key}.glb → 顶点；无则该武器回退程序化枪模）\n    gun_glbs: std::collections::HashMap<String, Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)>>,")
# ② new 初始化
s = s.replace("            gun_glb: Self::load_gun_glb(),", "            gun_glbs: std::collections::HashMap::new(),")
# ③ load 函数改造：带 key 参数 + 路径 = assets/guns/{key}.glb（回退 ak12 惯例：key 不存在时用全局 ak12.glb）
s = s.replace("""    /// 导入枪模：优先 assets/guns/ak12_baked.glb（Blender 顶点色烘焙版），
    /// 其次 ak12.glb（原始 Sketchfab）→ 烘焙顶点（同程序化光照）
    fn load_gun_glb() -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        // 2026-08-28 终局：使用原始模型材质本色（baseColorFactor 0.057/0.076 中性黑）
        let path = "assets/guns/ak12.glb";""",
"""    /// 导入枪模（按武器 key 自动寻找 assets/guns/{key}.glb；不存在回退 ak12.glb）
    /// 2026-08-28 终局：使用原始模型材质本色（baseColorFactor 直出 × 忠实现光）
    fn load_gun_glb(key: &str) -> Option<(Vec<crate::engine::meshgen::GVertex>, Vec<u32>)> {
        let path = if std::path::Path::new(&format!("assets/guns/{key}.glb")).exists() {
            std::borrow::Cow::Owned(format!("assets/guns/{key}.glb"))
        } else if std::path::Path::new("assets/guns/ak12.glb").exists() {
            std::borrow::Cow::Borrowed("assets/guns/ak12.glb")
        } else {
            std::borrow::Cow::Borrowed("assets/guns/ak12.glb")
        };""")
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('keyed load part1')
