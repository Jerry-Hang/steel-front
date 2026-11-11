// 由 src/lib.rs 拆出（2026-11-11，纯移动）：触屏输入（Android/iOS 等无鼠标设备）。
//
// 原样保留 `#[cfg(target_os = "android")]` 门控 —— 桌面构建里这个 impl 整块不参与编译，
// 所以 `touch_move`/`touch_fire` 这些字段在桌面上本来就不存在（漏掉 cfg 会报 no field）。
#[cfg(target_os = "android")]
use crate::engine::game::GameState;
#[cfg(target_os = "android")]
use crate::GameApp;

/// 分区（屏幕坐标，y 向下）：
/// - 左半屏        → 方向杆：起点为圆心，偏移超死区即产生 WASD。
/// - 右下角热区    → 开火（按住连发）。
/// - 其余（右半屏）→ 拖动转视角。
#[cfg(target_os = "android")]
impl GameApp {
    fn handle_touch(&mut self, touch: winit::event::Touch) {
        use winit::event::TouchPhase;
        let (sw, sh) = self.screen_size;
        let (x, y) = (touch.location.x as f32, touch.location.y as f32);
        // 开火热区：右下角 22% x 40%
        let in_fire_zone = x > sw * 0.78 && y > sh * 0.60;
        match touch.phase {
            TouchPhase::Started => {
                // 开始菜单/加载中：点一下即开局（手机上只有触控，没有键盘）。
                let st = self.game.state();
                if st == GameState::StartMenu || st == GameState::LoadingMap {
                    self.game.on_any_key();
                    return;
                }
                if in_fire_zone && self.touch_fire.is_none() {
                    self.touch_fire = Some(touch.id);
                    self.fire_requested = true;
                    self.fire_edge = true;
                } else if x < sw * 0.5 && self.touch_move.is_none() {
                    self.touch_move = Some((touch.id, x, y));
                } else if self.touch_look.is_none() {
                    self.touch_look = Some((touch.id, x, y));
                }
            }
            TouchPhase::Moved => {
                if let Some((id, ox, oy)) = self.touch_move {
                    if id == touch.id {
                        // 方向杆半径取屏高的 12%，死区 25%
                        let r = (sh * 0.12).max(24.0);
                        let dead = r * 0.25;
                        let (dx, dy) = (x - ox, y - oy);
                        self.key_state.forward = dy < -dead;
                        self.key_state.backward = dy > dead;
                        self.key_state.left = dx < -dead;
                        self.key_state.right = dx > dead;
                        // 🔴 **必须同步给游戏**（2026-10-15，外部复核指出第 4 条，已确认）：
                        // 第一人称的移动**只**来自 `game.set_movement(..)`，而它**只**被
                        // `sync_game_movement()` 调用（`camera.update(&key_state, ..)`
                        // 只在 `CameraMode::Flight` 下移动相机；第一人称相机位置每帧由
                        // `game.player_eye()` 覆盖）。
                        // 原来这里只改 `GameApp::key_state`、**从不调 `sync_game_movement`**
                        // ⇒ **Android 触屏端左半屏方向杆在第一人称下完全推不动玩家**。
                        self.sync_game_movement();
                    }
                }
                if let Some((id, lx, ly)) = self.touch_look {
                    if id == touch.id {
                        self.camera.look(x - lx, y - ly);
                        self.touch_look = Some((id, x, y));
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.touch_fire == Some(touch.id) {
                    self.touch_fire = None;
                    self.fire_requested = false;
                }
                if let Some((id, _, _)) = self.touch_move {
                    if id == touch.id {
                        self.touch_move = None;
                        self.key_state.reset();
                        // 松手同样要同步 —— 否则最后那个方向会**卡住不放**
                        // （`set_movement` 那边还留着按下时的值）
                        self.sync_game_movement();
                    }
                }
                if let Some((id, _, _)) = self.touch_look {
                    if id == touch.id {
                        self.touch_look = None;
                    }
                }
            }
        }
    }
}
