# -*- coding: utf-8 -*-
import io
s = io.open('src/main.rs', encoding='utf-8').read()
# Cow → String
s = s.replace("""        let path = if std::path::Path::new(&format!("assets/guns/{key}.glb")).exists() {
            std::borrow::Cow::Owned(format!("assets/guns/{key}.glb"))
        } else if std::path::Path::new("assets/guns/ak12.glb").exists() {
            std::borrow::Cow::Borrowed("assets/guns/ak12.glb")
        } else {
            std::borrow::Cow::Borrowed("assets/guns/ak12.glb")
        };""",
"""        let path = if std::path::Path::new(&format!("assets/guns/{key}.glb")).exists() {
            format!("assets/guns/{key}.glb")
        } else {
            "assets/guns/ak12.glb".to_string()
        };
        let path: &str = &path;""")
s = s.replace("let path: &str = &path;\n        let bytes", "let path: &str = &path;\n        let bytes")
# 函数体内后续用 path 变量（之前 &str 推断）—— 简单替换调用处的 fn load_gun_glb() → 带 key
s = s.replace("gun_glb: Self::load_gun_glb()", "gun_glbs: std::collections::HashMap::new()")
# first_person_gun_mesh 的 gun_glb 使用：换成当前武器 key 的缓存
s = s.replace("""    fn first_person_gun_mesh(&mut self) -> (Vec<crate::engine::meshgen::GVertex>, Vec<u32>) {
        // 导入枪模优先（检视与第一人称共用：检视时居中，第一人称保持导入姿态）
        if let Some((verts, indices)) = self.gun_glb.clone() {""",
"""    fn first_person_gun_mesh(&mut self) -> (Vec<crate::engine::meshgen::GVertex>, Vec<u32>) {
        // 导入枪模优先（按当前武器 key 缓存；检视与第一人称共用）
        let gkey = self.game.active_weapon_key().to_string();
        let entry = self.gun_glbs.entry(gkey).or_insert_with(|| Self::load_gun_glb(&gkey));
        if let Some((verts, indices)) = entry.clone() {""")
# 后面若还有 self.gun_glb 引用——检查
io.open('src/main.rs', 'w', encoding='utf-8', newline='').write(s)
print('part2 ok')
