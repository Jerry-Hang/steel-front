//! 桌面二进制入口（薄壳）。
//!
//! 引擎主体在 `src/lib.rs`（库目标）——这样 Android 侧才能把同一个 crate 编成
//! `cdylib`（`.so`）并导出 `android_main`。桌面走这个 bin。
fn main() {
    steel_front::run_steel_front_desktop();
}