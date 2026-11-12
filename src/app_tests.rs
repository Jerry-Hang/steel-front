// 由 src/lib.rs 拆出（2026-11-13）：`lib.rs` 的全部单元测试。
//
// `super::*` 的语义**不变**：本模块仍是 crate root 的子模块，所以测试里原有的
// `use super::*;`、对私有字段/私有函数的访问都照旧有效（只是换了个文件）。
//

/// 按**字符**数截取窗口（源码里有中文，按字节切会踩非 char 边界 panic）。
fn src_window(src: &str, start: usize, max_chars: usize) -> &str {
    let s = &src[start..];
    let end = s.char_indices().nth(max_chars).map(|(i, _)| i).unwrap_or(s.len());
    &s[..end]
}

/// 🔴 **释放光标必须恢复系统箭头**（2026-10-20 第 14 批复核第 3 条）：
/// Windows 捕获路径用的是 1×1 全透明自定义光标，释放只 `set_cursor_visible(true)`
/// ⇒ 鼠标能动但**完全隐形**。所有释放点必须共用同一个 helper。
#[test]
fn cursor_release_restores_default_icon_and_is_shared() {
    // 🔴 Windows 检出是 CRLF（本机 `core.autocrlf=true`）⇒ 先归一化行尾，
    // 否则下面搜 `"\n    }\n"` 永远搜不到（实测 2026-10-26）。
    // 🔴 2026-11-10：`sync_cursor` 已搬到 `src/app_window.rs`（lib.rs 拆分），
    // 而这条判据原本只扫 `lib.rs` ⇒ 搬家后它会把"代码动过"误报成"释放点丢了"。
    // 判据要跟着代码走：把两处拼起来扫，并且**两条都必须非空**（防止 include 失效
    // 导致"扫到 0 个"却仍然通过 —— 那是恒真断言）。
    // 🔴 2026-11-13：`window_event` 的实现体搬到 `app_events.rs` 后，释放光标的调用点也有
    // 一部分跟着过去了 ⇒ 扫描面加上它（三份都必须非空，防 include 失效时变成恒真）。
    let raw = format!(
        "{}\n{}\n{}",
        include_str!("lib.rs"),
        include_str!("app_window.rs"),
        include_str!("app_events.rs")
    );
    let src = raw.replace("\r\n", "\n");
    let src = src.as_str();
    let start = src.find("fn release_cursor_immediately").expect("必须有统一释放实现");
    let end = start + src[start..].find("\n    }\n").expect("helper 必须有函数结尾");
    let body = &src[start..end];
    assert!(
        body.contains("CursorIcon::Default"),
        "释放光标时必须恢复 CursorIcon::Default（否则 Windows 上指针隐形）"
    );
    // 🔴 **这里原本写 `>= 4`，而它一直在数自己**：`matches("self.release_cursor_immediately(")`
    // 这行字面量本身就出现在被扫描的 lib.rs 里（当时测试还在 lib.rs 的 `mod tests` 中）⇒
    // 3 个真实调用点 + 1 个自身字面量 = 4，"刚好达到"阈值。2026-11-13 把测试搬到
    // `app_tests.rs` 后，字面量不再落在扫描面里 ⇒ 计数掉到 3，真相才显出来。
    // **这正是教训 14 的形态**（自指的判据等于恒真）。现在按事实钉 3：
    //   sync_cursor / ESC 菜单 / 设置菜单（Menu 键）。
    let calls = src.matches("self.release_cursor_immediately(").count();
    assert_eq!(
        calls, 3,
        "释放点必须共用该 helper（sync_cursor + ESC 菜单 + 设置菜单 = 正好 3 处调用；\
         定义本身是 `fn release_cursor_immediately`，不计入），实际 {calls}"
    );
}

/// 🔴 **ESC 菜单用其它键关闭后必须 return**（2026-10-20 第 14 批复核第 7 条）：
/// 否则关闭菜单后 `in_gameplay` 立刻变 true，同一按键继续穿透：
/// 按 R 关菜单会重开一局、按 X 会吃掉一个医疗包、按 G 会当场扔雷。
#[test]
fn esc_menu_consumes_the_key_that_closes_it() {
    // 🔴 2026-11-13：`window_event` 的实现体（含 ESC 菜单导航块）已搬到 `app_events.rs`
    // ⇒ 只扫 lib.rs 会报「ESC 菜单导航块必须存在」。两个文件都扫。
    let joined = format!("{}\n{}", include_str!("lib.rs"), include_str!("app_events.rs"));
    let src = joined.as_str();
    // 从 ESC 菜单那一块起找（文件里还有一处 StartMenu 的 "非 Escape" 判断，别抓错）
    let base = src
        .find("// ESC 菜单导航：Tab 切换选项")
        .expect("ESC 菜单导航块必须存在");
    let i = base
        + src[base..]
            .find("if pressed && key_code != KeyCode::Escape {")
            .expect("ESC 菜单兜底分支必须存在");
    let seg = src_window(&src, i, 1200);
    assert!(
        seg.contains("return;"),
        "关闭 ESC 菜单后必须 return，不能把同一按键继续传给玩法层"
    );
}

/// 🔴 **重开/切关必须复位 GameApp 层瞬时状态**（2026-10-20 第 14 批复核第 2 条）：
/// `Game::start_run` 管不到按键锁存/开火意图/尸体快照/伤害飘字/枪摆动。
#[test]
fn restart_paths_reset_gameapp_transients() {
    // 🔴 2026-11-13：`window_event` 的实现体已搬到 `src/app_events.rs`，其中也有复位调用点
    // ⇒ 只扫 lib.rs 会把"搬家"误报成"少了一个入口"。两个文件都扫。
    let joined = format!("{}\n{}", include_str!("lib.rs"), include_str!("app_events.rs"));
    let src = joined.as_str();
    let calls = src.matches(".reset_session_transients()").count();
    assert!(calls >= 3, "重开与切关路径都必须调用复位（方法定义 + 3 个入口），实际 {calls}");
    let i = src.find("fn reset_session_transients").expect("必须有复位方法");
    let body = src_window(&src, i, 1500);
    for needle in [
        "key_state.reset()",
        "fire_requested = false",
        "command_open = false",
        "corpses.clear()",
        "last_npc_snapshot.clear()",
        "hit_damage_popups.clear()",
        "gun_sway.reset_for_new_run()",
    ] {
        assert!(body.contains(needle), "reset_session_transients 缺少 {needle}");
    }
}
use super::*;

/// 🔴 **`GameApp` 里 `renderer` 必须声明在 `window` 前面。**
///
/// 存在理由（2026-10-16 真机实测，一次 300 秒跑法留下核心转储）：
/// **Rust 按【声明顺序】析构结构体字段。** 原来是 `window` 在前 ⇒ 退出时
/// **winit 窗口（wayland 的 `wl_surface`/`wl_display` 代理）先销毁，
/// `Renderer` 后销毁** ⇒ 驱动销毁 Vulkan 对象（交换链）时回头去碰**已死的代理**
/// ⇒ **`SIGSEGV`（`SEGV_MAPERR`）**。核心转储的栈：
/// ```text
/// #0..#3  libwayland-client.so.0   wl_proxy_marshal_flags
/// #4..#7  libnvidia-glcore.so.615.78.08
/// #8      <Renderer as Drop>::drop
/// #9      drop_in_place<Renderer>
/// #10     drop_in_place<GameApp>
/// #11     run_steel_front
/// ```
///
/// ⚠️ **它难办的地方在于日志是干净的**：引擎最后三行是
/// 「窗口关闭请求 / 性能日志已保存 / **程序正常退出**」——
/// 主循环干净退出，崩在 `main` 返回**之后** ⇒
/// **只读日志的门禁对它看不见**（当时 `smoke_linux.sh` 就把这次判成了 `ALL-OK`，
/// 见同批修的 `--exit-status`）。
///
/// ⇒ 这条判据只能**看声明顺序**：这件事没有便宜的运行时观测点，
///   而真机退出恰恰是冒烟最容易漏掉的一段。
#[test]
fn renderer_field_drops_before_the_window() {
    let src = include_str!("lib.rs");
    let start = src
        .find("struct GameApp {")
        .expect("找不到 struct GameApp —— 判据的前提不成立");
    let body = &src[start..];
    let end = body.find('}').expect("struct GameApp 没有闭合");
    let body = &body[..end];

    let renderer_at = body
        .find("renderer: Option<Renderer>")
        .expect("GameApp 里没有 renderer 字段 —— 判据的前提不成立");
    let window_at = body
        .find("window: Option<Window>")
        .expect("GameApp 里没有 window 字段 —— 判据的前提不成立");
    assert!(
        renderer_at < window_at,
        "`window` 声明在 `renderer` 前面 ⇒ 退出时**窗口先销毁、交换链后销毁**\n\
         ⇒ 驱动销毁 Vulkan 对象时去碰已死的 wayland 代理 ⇒ SIGSEGV。\n\
         而那次崩溃**日志是干净的**（「程序正常退出」之后就没了），只读日志的门禁抓不到它。"
    );
}

/// 🔴 **判据：`main()` 里的致命错误路径不许以退出码 0 结束。**
///
/// 2026-10-03 实测踩到：在一个**没有图形会话环境变量**的 shell 里
/// （`XDG_SESSION_TYPE=tty`，缺 `WAYLAND_DISPLAY`/`DISPLAY`），引擎报
/// "创建事件循环失败" 然后 `return` ⇒ **退出码 0**。
/// `perf_run.sh` 只看到"游戏提前退出（code 0）"，无法把「一帧都没渲染」
/// 和「正常结束」区分开 —— 教训 46 的形态：工具必须能说"我没跑成"。
/// 同一晚还发现 `run_app` 出错后会继续往下打出"程序正常退出"（日志说正常、
/// 退出码说成功，而实际是异常终止），一并钉住。
///
/// ⚠️ 这是**源码扫描**型判据，两个坑都要防：
/// 1. **必须先证明它真的扫到了东西** —— 文件被搬走/改名/那段被重写时，
///    `find` 返回 `None` 会让断言静默恒真；
/// 2. **必须先去注释** —— 修这个 bug 时写的注释里就引用了「程序正常退出」，
///    不去注释的话扫描会先撞上注释、让判据自行满足（本仓 `no_unbounded_wait_on_vulkan_calls`
///    同样先做 `is_comment` 过滤）。
#[test]
fn fatal_startup_paths_never_exit_zero() {
    // 🔴 2026-11-12：`resumed` 的实现体已搬到 `src/app_events.rs`（"创建窗口失败"/"渲染器
    // 初始化失败"两个锚点跟着搬走了），而这条判据原本只扫 `lib.rs` ⇒ 会报"检查失效"。
    // 判据要跟着代码走：把两个文件拼起来扫。
    //
    // ⚠️ **必须各自先切掉测试模块再拼**：`split("#[cfg(test)]")` 在拼接后的整串上做时，
    // 会在 **lib.rs 的测试模块**处截断 ⇒ app_events.rs 整份被丢掉、锚点又找不到了
    // （我第一次就是这么写的，判据立刻红）。这也是"拼接式扫描"的一个通用坑。
    let lib_prod = include_str!("lib.rs").split("#[cfg(test)]").next().unwrap_or("");
    let events = include_str!("app_events.rs");
    let joined = format!("{lib_prod}\n{events}");
    let prod = joined.as_str();
    assert!(
        prod.contains("fn handle_resumed") && prod.contains("impl ApplicationHandler for GameApp"),
        "扫描面不完整：lib.rs 生产段与 app_events.rs 都必须在内"
    );
    // 去注释：行首是 // 的一律丢掉
    let code: String = prod
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let anchors = ["创建事件循环失败", "run_app(&mut app)"];
    for a in anchors {
        assert!(
            code.contains(a),
            "检查失效：去注释后扫不到锚点 {a:?}（源码被改名/搬走了？）"
        );
    }

    // 🔴 **必须把两半切开**：第一版写成 `code[i..].contains("process::exit")`，
    // 扫的是"锚点之后的全部内容" ⇒ **第二处修复把第一处的断言喂饱了**，
    // 把事件循环那条改回 `return` 时测试照样绿（反证实测发现）。
    // 这正是本仓教训 14 的形态：恒真的断言等于没写。
    let split = code.find("run_app(&mut app)").unwrap();
    let (startup, running) = code.split_at(split);

    // 1) 事件循环创建失败之后必须出现 process::exit（只在**这一半**里找）
    let i = startup.find("创建事件循环失败").unwrap();
    assert!(
        startup[i..].contains("process::exit"),
        "事件循环创建失败是致命错误，不能只 return（`fn main` 正常返回 = 退出码 0）"
    );

    // 2) run_app 出错分支也必须有 process::exit，且**必须早于**「程序正常退出」那句
    let exit_at = running.find("process::exit");
    let normal_at = running.find("程序正常退出");
    assert!(
        normal_at.is_some(),
        "检查失效：去注释后扫不到「程序正常退出」那句，说明扫描窗口不对"
    );
    assert!(
        exit_at.is_some(),
        "run_app 出错是致命错误，不能退 0"
    );
    assert!(
        exit_at.unwrap() < normal_at.unwrap(),
        "run_app 出错后必须先退出，不能掉到「程序正常退出」那句（日志说正常、退出码说成功，\
         而实际是异常终止）"
    );

    // 3) **循环内部**的致命错误也必须被记进 `fatal`。
    //    这两处只能 `event_loop.exit()`，而它让 `run_app` 返回 `Ok` ⇒ 上面那条
    //    `run_app` 检查抓不到它们。2026-10-03 实测：解包后跑错目录（缺 assets/mesh.spv）
    //    报的就是「渲染器初始化失败」+「程序正常退出」+ 退出码 0。
    for anchor in ["创建窗口失败", "渲染器初始化失败"] {
        let i = code.find(anchor).unwrap_or_else(|| {
            panic!("检查失效：去注释后扫不到锚点 {anchor:?}（源码被改名/搬走了？）")
        });
        let tail = &code[i..(i + 300).min(code.len())];
        assert!(
            tail.contains("self.fatal = Some"),
            "{anchor} 是致命错误，必须记进 `self.fatal` —— 否则 event_loop.exit() 会让 \
             run_app 返回 Ok，引擎打出「程序正常退出」并以 0 退出"
        );
    }

    // 4) `fatal` 必须在循环结束后被消费掉（只记不查等于没记）
    assert!(
        running.contains("app.fatal"),
        "`fatal` 记了却没人查 —— 循环结束后必须据此非零退出"
    );
}

/// 🔴 判据：`RV3D_SHOT_AT` 的解析与"取走到点"两条（2026-10-03）。
///
/// 加这个开关的理由是**平台不对称**：Windows 侧的 `run_resize_probe.ps1` 能用
/// `PostMessage` 发 F12 截图，而 Linux 上唯一的截图触发就是 F12 按键，
/// Wayland 下注入按键要抢焦点，违反铁律 C 的鼠标安全协议
/// ⇒ Linux 的探针只能停在"没有画面取证"。给一个不需要输入的触发把这条补平。
///
/// 两条各钉一个方向，第 3 条是真正要命的那个：
/// 1. 解析要**去重升序**并丢弃非法项（取证开关不该因为写错一个数就让整局起不来）；
/// 2. 非正数（0 / 负数）必须丢 —— `0` 会让 `retain(t > elapsed)` 在 elapsed=0 时
///    把它留下、之后又永远取不走，变成每帧都截一张；
/// 3. 🔴 **一帧跨过多个时刻时不能漏**：低帧率或长卡顿下，某一帧的 elapsed 可能
///    同时越过 5s 和 15s。用"判等"实现会永久漏掉被跨过的那个 ——
///    而截图是取证用的，漏一张就少一份证据，**且不会报错**。
#[test]
fn shot_at_parsing_and_due_taking_are_falsifiable() {
    // 1) 去重升序 + 丢弃非法项
    assert_eq!(parse_shot_at("15,5,5,abc,-2,0"), vec![5.0, 15.0]);
    assert_eq!(parse_shot_at("  3.5 , 1 "), vec![1.0, 3.5]);
    assert!(parse_shot_at("").is_empty());
    assert!(parse_shot_at("nope").is_empty());
    assert!(parse_shot_at("NaN,inf,-1,0").is_empty(), "NaN/inf/非正数都要丢");

    // 2) 只取走"已到点"的，其余原样保留
    let mut p = parse_shot_at("5,10,15");
    assert_eq!(take_due_shots(&mut p, 4.9), 0);
    assert_eq!(p, vec![5.0, 10.0, 15.0]);
    assert_eq!(take_due_shots(&mut p, 5.0), 1, "到点即取（含等于）");
    assert_eq!(p, vec![10.0, 15.0]);

    // 3) **一帧跨过两个时刻**：必须一次取走两个，不能只取一个
    assert_eq!(take_due_shots(&mut p, 12.0), 1);
    assert_eq!(p, vec![15.0]);
    let mut q = parse_shot_at("1,2,3");
    assert_eq!(take_due_shots(&mut q, 100.0), 3, "跨过全部时一次全取走");
    assert!(q.is_empty());
    assert_eq!(take_due_shots(&mut q, 100.0), 0, "取空之后不再重复计数");
}

/// 🔴 判据：后台帧率上限（2026-09-27「游戏一开整机就卡」）。
///
/// 四条各钉一个方向，缺一条就会出现"限了但限错"：
/// 1. 默认（两侧都 0）必须是**不限制** —— 压测/采集路径逐字节不变；
/// 2. 未聚焦且设了 bg 时必须被夹住（这是修的那个 bug）；
/// 3. 聚焦时 bg **不许**生效（否则玩家自己玩的时候也被限）；
/// 4. bg 只能**收紧**不能放宽（fg=30、bg=120 时仍取 30）。
#[test]
fn frame_cap_is_disabled_by_default_and_only_tightens() {
    // 1) 默认不限制
    assert_eq!(effective_frame_cap(0.0, 0.0, true), 0.0);
    assert_eq!(effective_frame_cap(0.0, 0.0, false), 0.0);
    // 2) 未聚焦：bg 夹住前台上限（含 fg=0 即"前台不限"的两侧组合）
    assert_eq!(effective_frame_cap(300.0, 20.0, false), 20.0);
    assert_eq!(effective_frame_cap(0.0, 20.0, false), 20.0);
    // 3) 聚焦时 bg 不生效
    assert_eq!(effective_frame_cap(300.0, 20.0, true), 300.0);
    assert_eq!(effective_frame_cap(0.0, 20.0, true), 0.0);
    // 4) bg 只能收紧：更宽的 bg 不改变更严的前台上限
    assert_eq!(effective_frame_cap(30.0, 120.0, false), 30.0);
    // 非法的负值按"不设限"处理，绝不允许变成负 fps
    assert_eq!(effective_frame_cap(-5.0, -1.0, false), 0.0);
}

/// 🔴 **Linux 适配判据**：窗口请求尺寸必须是**物理像素**，且超屏时等比缩。
///
/// 修的是 `LogicalSize::new(w / 1.5, h / 1.5)` —— 那个 1.5 是 Windows 那台机器
/// `scale_factor = 1.5` 的硬编码补偿，在 scale=1.0 的 Linux 上把 2560x1600
/// 变成 1706x1066，而 Wayland 下窗口尺寸直接决定交换链尺寸 ⇒ 整条渲染尺寸链都错。
///
/// 三条断言各自对应一个会被写错的方向（都不是恒真断言）：
/// 1. **不缩放**：请求值原样返回 —— 任何乘除常数都会让这条红；
/// 2. **超屏等比缩**：且比例保持（缩完仍宽高比一致），不是只夹一边；
/// 3. **下限**：0 或极小值不许原样透出去（合成器会拒绝 0 尺寸窗口）。
#[test]
fn window_request_is_physical_and_clamped() {
    // 1) 显式分辨率（物理）原样透传 —— 显示器足够大时一个像素都不许改
    assert_eq!(window_physical_request(2560, 1600, Some((2560, 1600))), (2560, 1600));
    assert_eq!(window_physical_request(1920, 1080, Some((3840, 2160))), (1920, 1080));
    // 2) 超屏等比缩：3840x2160 请求放进 1920x1080 => 恰好一半，宽高比不变
    let (w, h) = window_physical_request(3840, 2160, Some((1920, 1080)));
    assert_eq!((w, h), (1920, 1080));
    // 只超高（不超宽）时也要缩，且按**较小的**那个比例缩（否则会溢出另一边）
    let (w, h) = window_physical_request(1920, 4000, Some((1920, 1080)));
    assert!(w <= 1920 && h <= 1080, "缩完不许仍溢出：{w}x{h}");
    assert!(w > 0 && h > 0);
    // 3) 拿不到显示器（Wayland 的 primary_monitor() 恒 None）=> 如实请求，不猜
    assert_eq!(window_physical_request(2560, 1600, None), (2560, 1600));
    // 4) 下限：0 与极小值不许透传
    assert_eq!(window_physical_request(0, 0, None), (320, 200));
    assert_eq!(window_physical_request(100, 50, None), (320, 200));
}

/// 🔴 **Linux 适配判据**：`RV3D_BACKEND=x11` 在**原生 Linux** 上必须能生效。
///
/// 旧代码的判据是 `is_wsl && WAYLAND_DISPLAY.is_some()` ⇒ 原生 Arch/KDE Wayland 上
/// **恒为 false**，而 winit 0.30 已删除 `WINIT_UNIX_BACKEND` ⇒ 用户**没有**任何
/// 退回 XWayland 的手段。这不是"以防万一"的开关：Wayland 下 `Locked` 抓取的失败
/// 是**静默**的（winit 忽略合成器的确认事件），XWayland 是唯一可验证的退路。
///
/// 每条断言对应一个**会被写错的方向**：
/// 1. **非 WSL 也必须能选到 x11**（这一条就是整个修复）；
/// 2. 显式变量**压过** WSL 自动判定（两个方向都测：wsl 上选 wayland、非 wsl 上选 x11）；
/// 3. 大小写/空白不敏感（`X11`、` xwayland ` 都要认）；
/// 4. 未设时不改变旧行为（WSL 自动选 x11；原生 Linux 保持 Auto）。
#[cfg(target_os = "linux")]
#[test]
fn backend_choice_x11_is_reachable_on_native_linux() {
    // 1) 就是这个修复：非 WSL 的原生 Linux + 显式 x11
    assert_eq!(backend_choice(Some("x11"), false), BackendChoice::X11);
    // 2) 显式变量压过自动判定（两个方向）
    assert_eq!(backend_choice(Some("wayland"), true), BackendChoice::Auto);
    assert_eq!(backend_choice(Some("x11"), true), BackendChoice::X11);
    // 3) 大小写与空白
    assert_eq!(backend_choice(Some("X11"), false), BackendChoice::X11);
    assert_eq!(backend_choice(Some(" xwayland "), false), BackendChoice::X11);
    assert_eq!(backend_choice(Some("Wayland"), true), BackendChoice::Auto);
    // 4) 未设 / 不认识的取值 ⇒ 旧行为不变
    assert_eq!(backend_choice(None, true), BackendChoice::X11, "WSL 自动判定保持不变");
    assert_eq!(backend_choice(None, false), BackendChoice::Auto, "原生 Linux 默认不强制");
    assert_eq!(backend_choice(Some("nonsense"), false), BackendChoice::Auto);
}

/// 🔴 **Linux 适配判据**：锁定态的「假成功」必须能被**行为证据**判出来。
///
/// `set_cursor_grab(Locked)` 返回 `Ok` 不代表锁生效（Wayland 下指针尚未 enter 时
/// winit 什么都没做也返回 `Ok`，且它忽略合成器的确认事件）。失败症状与"鼠标坏了"
/// 一模一样（零 raw 事件 ⇒ 视角不动），所以**唯一**可信的判据是"锁定之后真的
/// 收到过相对增量"。
///
/// 每条断言对应一个**会被写错的方向**：
/// 1. 观察窗内没增量 ⇒ `Pending`（**不许**提前下结论 —— 用户可能只是还没动鼠标，
///    这一条挂了就会把正常玩家误判成假成功并去补抓，反而干扰他）；
/// 2. 增量**优先于时间**：就算第 1ms 就来，也立刻 `Confirmed`（最强的正面证据，
///    没有理由再等满观察窗）；
/// 3. 到点仍为零 ⇒ `NoMotion`（这才是要报警的那条）；
/// 4. 边界：恰好等于窗口长度算到点（`>=` 而不是 `>` —— 写成 `>` 会让判定
///    永远晚一帧，在 165fps 下无感，但语义上"满窗"就该有结论）。
#[test]
fn lock_observation_needs_evidence_not_just_ok() {
    const W: u128 = 1500;
    // 1) 窗口内、无增量 ⇒ 不许下结论
    assert_eq!(lock_observation(0, 0, W), LockObservation::Pending);
    assert_eq!(lock_observation(W - 1, 0, W), LockObservation::Pending);
    // 2) 增量优先于时间：第 1ms 就来也立刻确认
    assert_eq!(lock_observation(1, 1, W), LockObservation::Confirmed);
    assert_eq!(lock_observation(0, 7, W), LockObservation::Confirmed);
    // 3) 到点仍为零 ⇒ 假成功
    assert_eq!(lock_observation(W, 0, W), LockObservation::NoMotion);
    // 4) 边界：恰好满窗算到点；超时很久也仍是 NoMotion（不是 Confirmed）
    assert_eq!(lock_observation(u128::MAX, 0, W), LockObservation::NoMotion);
    // 反向保险：确认过之后再多的零增量也不该翻回 NoMotion（delta 是单调的）
    assert_eq!(lock_observation(u128::MAX, 3, W), LockObservation::Confirmed);
}

/// 观察窗长度不许是 0 —— 那会让判定在第一帧就报假成功，把"还没动鼠标"当成故障。
#[test]
fn lock_observe_window_is_not_degenerate() {
    assert!(LOCK_OBSERVE_MS >= 200, "太短会把正常玩家误判成假成功");
    assert!(LOCK_OBSERVE_MS <= 5000, "太长则用户看着不动的视角干等");
}

/// 🔴 **Linux 适配判据**：`RV3D_NO_CAPTURE=1` 必须能**无条件**关掉光标抓取。
///
/// 为什么这条在 Linux 上是必需品：Windows 侧的鼠标安全协议靠 `PostMessage`
/// 投键 + 不抢前台（用户 2026-09-03 的要求），于是自动化不必抓光标；Linux 没有
/// 这条路，而引擎在 `Playing` 态一定会抓（`Locked` 或 `Confined`）⇒ 跑一局冒烟
/// 就等于把用户的指针锁进游戏窗口。把"关掉捕获"做成一条判据，自动化不碰用户
/// 输入才是**代码保证**，而不是靠调用方自觉。
///
/// 每条断言对应一个**会被写错的方向**：
/// 1. `allowed=false` ⇒ 恒假（**这一条就是那个开关**；漏掉它 = 开关无效）；
/// 2. 四个条件的**每一个**单独为假都要能关掉捕获 —— 特别是 `esc_menu_open`：
///    漏掉它就是"菜单里鼠标被锁死，只能 Alt+F4"（本仓 2026-08-15 修过一次）；
/// 3. 全真才为真（防止把 `||` 写成 `&&` 之类的反向错误）。
#[test]
fn capture_wanted_obeys_the_no_capture_switch_and_the_menu_gates() {
    // 1) 开关优先：其它条件全真也必须是假
    assert!(!capture_wanted(true, true, false, false, false), "RV3D_NO_CAPTURE=1 必须关掉捕获");
    // 2) 四个条件逐个单独为假
    assert!(!capture_wanted(false, true, false, false, true), "失焦不许抓");
    assert!(!capture_wanted(true, false, false, false, true), "非 Playing 不许抓");
    assert!(!capture_wanted(true, true, true, false, true), "设置面板打开不许抓");
    assert!(!capture_wanted(true, true, false, true, true), "ESC 菜单打开不许抓");
    // 3) 全真才抓
    assert!(capture_wanted(true, true, false, false, true));
}

/// 🔴 判据：中继注册失败时**不许**报「已注册」（教训 46：日志不许把「没跑成」写成成功）。
///
/// 两个方向都要：成功时文案里必须有房间名与端口（现场要用它核对），
/// 失败时文案里必须**没有**成功字样，且必须带出原始错误。
#[test]
fn relay_registration_report_never_claims_success_on_failure() {
    let ok: std::io::Result<()> = Ok(());
    let (okv, msg) = rdv_register_report("10.0.0.1:9000", "steel", 27015, &ok);
    assert!(okv, "成功必须报成功: {msg}");
    assert!(msg.contains("已向中继") && msg.contains("steel"), "{msg}");
    assert!(msg.contains("27015"), "端口要出现在现场日志里: {msg}");

    let err: std::io::Result<()> = Err(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        "no route",
    ));
    let (okv2, msg2) = rdv_register_report("10.0.0.1:9000", "steel", 27015, &err);
    assert!(!okv2, "失败不许报成功: {msg2}");
    assert!(msg2.contains("失败"), "{msg2}");
    assert!(
        !msg2.contains("已向中继"),
        "失败文案里不许留下成功字样: {msg2}"
    );
    assert!(msg2.contains("no route"), "要把原始错误带出来: {msg2}");
}

/// 🔴 判据：`scripts/*.ps1` 的**每一行都不许以非 ASCII 字节结尾**。
///
/// 存在理由（2026-09-26 实测，5 个文件全中）：Windows PowerShell 5.1 把**无 BOM** 的
/// `.ps1` 按系统 ANSI（本机 GBK）解码。某行若以中文/全角字符结尾，其最后一个字节落在
/// GBK 首字节区间，解码器就把**行尾**当成它的第二个字节吃掉 ⇒ 下一行被并进这一行；
/// 若这一行恰好是注释，**下一行代码就被静默注释掉**，脚本照跑但少了一条语句：
///   * `compile_pt.ps1` 吃掉 `$ErrorActionPreference = 'Stop'`（编译失败不再中止）；
///   * `pt_power_ab.ps1` 吃掉 `function Run-Case(...)`（整个脚本不可用）；
///   * `ask_qianwen.ps1` 吃掉 `$ix = ...`（点击落在 x=0）；
///   * `release_input.ps1` 吃掉 `$alive = 0`；
///   * `run_gameplay_smoke.ps1` 吃掉四处代码（含启动游戏与跑冒烟的调用）。
/// 判据只看**行尾字节**：文件里出现中文没关系（`send_work.ps1` 的消息体就是中文，
/// 但它后面跟着 ASCII 的 `}`），被吃掉的一定是"行尾非 ASCII"的那些行。
#[test]
fn powershell_scripts_never_end_a_line_with_a_non_ascii_byte() {
    // 🔴 2026-09-26：扫描面以前只有 `scripts/`，而 `tools/shot_diff.ps1`（**已入库**）
    // 当时就有 6 行以中文结尾 ⇒ 它下一行（`$ErrorActionPreference` / `$src = ...` /
    // `$step = 7`）全被注释掉了 —— `tools/` 不在扫描面里，所以这条判据一直是绿的。
    // **判据的扫描面必须覆盖"所有入库的 .ps1"**，否则它只保护它恰好记得的那一个目录。
    let dirs = ["scripts", "tools"];
    let mut checked = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for dir in dirs {
        for entry in std::fs::read_dir(dir).expect("目录必须存在") {
            let path = entry.expect("读取目录项失败").path();
            if path.extension().and_then(|s| s.to_str()) != Some("ps1") {
                continue;
            }
            checked += 1;
            let bytes = std::fs::read(&path).expect("读 .ps1 失败");
            let name = format!(
                "{}/{}",
                dir,
                path.file_name().and_then(|s| s.to_str()).unwrap_or("?")
            );
            for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
                // 结尾的 CR 不算行尾内容（CRLF 文件）
                let end = if line.last() == Some(&b'\r') { line.len().saturating_sub(1) } else { line.len() };
                if end == 0 {
                    continue;
                }
                if line[end - 1] >= 0x80 {
                    let text = String::from_utf8_lossy(&line[..end]).to_string();
                    bad.push(format!("{name}:{}  …{}", i + 1, &text[text.len().saturating_sub(40)..]));
                }
            }
        }
    }
    // 目录走空时这条会静默通过 —— 那正是教训 27 的形态（"没测到"与"测到 0"分不清）
    assert!(checked >= 10, "只扫到 {checked} 个 .ps1，路径大概不对");
    assert!(
        bad.is_empty(),
        "有 {} 行以非 ASCII 字节结尾（PS 5.1 按 ANSI 读 ⇒ 会吞掉下一行）：\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// 🔴 判据：**入库的文本文件里不许有 NUL 字节**（2026-09-26 实测命中 1 处）。
///
/// 存在理由：`docs/HANDOFF-soldier.md` 里藏着一个 NUL —— 就在「· 0 警告 ·」那句里，
/// `0` 被写成了 `0x00`。后果是**文本工具到此为止**：`read` 直接判定 "binary file" 拒绝读取，
/// 按行处理的脚本也会在那行出怪事 —— 而在终端里它只显示成空白，**完全看不出来**。
/// 这与教训 36 同族：文件"看着好好的"，工具却读不了。
///
/// 扫描面 = `docs/*.md` + `src/**/*.rs` + `build.rs`/`build_spv_rt.rs`；
/// 自检：扫到的文件数必须 ≥ 30，否则路径写错也会静默通过（教训 27）。
#[test]
fn tracked_text_files_contain_no_nul_byte() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
    }
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("docs") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("md") {
                files.push(p);
            }
        }
    }
    walk(std::path::Path::new("src"), &mut files);
    for extra in ["build.rs", "build_spv_rt.rs"] {
        let p = std::path::PathBuf::from(extra);
        if p.exists() {
            files.push(p);
        }
    }
    assert!(files.len() >= 30, "只扫到 {} 个文本文件，路径大概不对", files.len());
    let bad: Vec<String> = files
        .iter()
        .filter_map(|p| {
            let b = std::fs::read(p).ok()?;
            let at = b.iter().position(|x| *x == 0)?;
            let line = b[..at].iter().filter(|x| **x == b'\n').count() + 1;
            Some(format!("{} 第 {line} 行附近（字节偏移 {at}）", p.display()))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "文本文件里含 NUL 字节 ⇒ 文本工具会把它当二进制拒绝读取：\n{}",
        bad.join("\n")
    );
}

/// 🔴 跨文件重复的着色器常量必须逐值一致（2026-10-01）。
///
/// 存在理由：本会话修掉的**两条真缺陷是同一个模式**——同一约定存在若干份物理副本，
/// 改了生产者、漏改消费者：
/// - `pt_set_scene_markers` 的 `* 0.5` 是 2026-09-17 之前"渲染盒 = 2×AABB"的遗留，
///   生产者改了、PT 这个消费者没跟上 ⇒ **PT 的每个 marker 盒整体小一半**（§22.14）；
/// - `main.rs` 占领底盘 `from_scale(10.0,…)` 配注释"半径 5.0 → scale 10.0"，
///   同样是旧约定遗留 ⇒ **领地底盘画成真实占领圈的 2 倍**（§23.13）。
/// 两条都**从画面上看不出来**、只能靠把值对起来算。既然算得出来，就该在测试里算，
/// 而不是等下一位再花两小时反推。
///
/// 自检（教训 27：判据必须能红）：**每份副本都必须真的被找到**。若某处改了名、
/// 挪了文件或正则不匹配，"找到 0 份"绝不能算通过——那正是本仓反复踩的恒真断言。
#[test]
fn duplicated_shader_constants_stay_in_sync() {
    /// 取某个标识符在**非注释行**上的所有声明值。
    /// 跳过注释是必须的：说明性注释里满是"必须与 X 同值"这类句子，
    /// 不跳过的话注释里提到的数字会被当成一份副本，把测试变成噪音。
    fn decls(src: &str, name: &str) -> Vec<f32> {
        let mut out = Vec::new();
        for line in src.lines() {
            let t = line.trim_start();
            if t.starts_with("//") || !line.contains(name) {
                continue;
            }
            let Some(eq) = line.find('=') else { continue };
            // 必须先 trim：`const X: f32 = 1.5;` 的等号后紧跟一个空格，
            // 不 trim 则 take_while 首字符即失败、解析出空串 ⇒ 一份都找不到。
            // （本函数第一版就栽在这里，靠下面"必须找到 4 份"的自检才没静默通过。）
            let rest = line[eq + 1..].trim_start();
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(v) = num.parse::<f32>() {
                out.push(v);
            }
        }
        out
    }

    let read = |p: &str| -> String {
        std::fs::read_to_string(p)
            .unwrap_or_else(|e| panic!("读 {p} 失败：{e}（测试工作目录应为仓库根）"))
    };
    let build = read("build.rs");
    let ray = read("src/engine/ray_tracer.rs");
    let ptshader = read("assets/rt/pt_panorama.glsl");

    // ① 砌块皮肤尺寸门：光栅 WGSL 两份（顶点着色器 + mesh 着色器各一份）
    //    + PT 的 Rust 常量 + PT 的 GLSL 常量 = 4 份。
    let mut gate: Vec<(&str, f32)> = Vec::new();
    for (i, v) in decls(&build, "MASONRY_MIN_SPAN").into_iter().enumerate() {
        gate.push((if i == 0 { "build.rs(VS)" } else { "build.rs(mesh)" }, v));
    }
    for v in decls(&ray, "MASONRY_MIN_SPAN") {
        gate.push(("ray_tracer.rs", v));
    }
    for v in decls(&ptshader, "MASONRY_MIN_SPAN") {
        gate.push(("pt_panorama.glsl", v));
    }
    assert_eq!(
        gate.len(),
        4,
        "MASONRY_MIN_SPAN 应有 4 份声明，实际找到 {} 份（{:?}）\
         ⇒ 有副本被改名/挪走/删掉了，这个测试本身需要跟着更新，不要直接放宽断言",
        gate.len(),
        gate
    );
    let want = crate::engine::ray_tracer::MASONRY_MIN_SPAN;
    let off: Vec<String> = gate
        .iter()
        .filter(|(_, v)| (*v - want).abs() > 1e-6)
        .map(|(w, v)| format!("{w} = {v}"))
        .collect();
    assert!(
        off.is_empty(),
        "MASONRY_MIN_SPAN 各副本与 ray_tracer 的 {want} 不一致：{}\
         ⇒ 尺寸门会在光栅/PT 两侧给出不同判定（§22.14 同族错法）",
        off.join("；")
    );

    // ② 皮肤 tile 尺寸：光栅用字面量、PT 用常量，两处必须同为 1.6 × 0.8。
    //    这是 §22.4b 那套"砖块尺度"的唯一真值来源，任一侧改动都会让两侧砖大小不同。
    assert!(
        build.contains("vec2<f32>(1.6, 0.8)"),
        "build.rs 里找不到皮肤 tile 字面量 `vec2<f32>(1.6, 0.8)`\
         ⇒ 若改了写法（例如换成常量），请同步更新本测试与 PT 侧，不要删断言"
    );
    assert!(
        ptshader.contains("SKIN_TILE_M = vec2(1.6, 0.8)"),
        "pt_panorama.glsl 里找不到 `SKIN_TILE_M = vec2(1.6, 0.8)`\
         ⇒ 两侧皮肤 tile 已脱钩，砖块尺度会光栅/PT 不一致"
    );
}

/// 🔴 尺寸门用的 **1.05 必须"除门本身以外与 1.0 不可区分"**（§22.7）。
///
/// `flat_flag` 是片元着色器的材质分派值：0=地面、1.0=marker、1.25=外部建模、
/// 2.0=NPC、3.0=枪。2026-09-30 起多了一个 **1.05 = "太小、不发砌块皮肤"的 marker**
/// （`MASONRY_MIN_SPAN` 尺寸门，护柱/消防栓这类小件走它）。
///
/// 风险不在门本身，而在**将来新加的那一条分支**：只要有人写出
/// `flat_flag > 1.0` 或 `flat_flag <= 1.03` 这种**在 1.0 与 1.05 上取值不同**的判据，
/// 被门挡下的 1.05 就会**悄悄走进与 1.0 不同的分支** —— 症状是"护柱忽然少了一层
/// 效果"，而没人会去查一个 0.05 的差。⇒ 把它变成硬约束：
/// **片元里每条 flat_flag 判据在 1.0 与 1.05 上取值必须相同，
/// 唯一例外是阈值恰为 1.02 的那条（尺寸门自己）。**
///
/// ⚠️ 判据是"**取值不同**"，不是"阈值落在 1.0~1.05 之间"：`flat_flag < 1.1`
/// 两条都成立 ⇒ **无害**，本测试不该报它（第一版把例子写成 `< 1.1`，
/// 被 `target/guardlogic.py` 的合成用例当场否掉）。
///
/// 自检（教训 27：判据必须能红）：解析到的判据数必须 **>= 8**，且必须**至少找到
/// 一条阈值 1.02 的门** —— 否则"没有例外"会因为"什么都没解析到"而恒真通过。
#[test]
fn gated_marker_flag_is_indistinguishable_except_at_the_gate() {
    /// 把非注释行里的 `flat_flag <op> <num>` 解析成 (算子, 阈值, 行号)。
    fn atoms(src: &str) -> Vec<(String, f32, usize)> {
        let mut out = Vec::new();
        for (ln, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut base = 0usize;
            while let Some(k) = line[base..].find("flat_flag") {
                let i = base + k + "flat_flag".len();
                let rest = line[i..].trim_start();
                let op = ["<=", ">=", "==", "!=", "<", ">"]
                    .iter()
                    .find(|o| rest.starts_with(**o))
                    .copied();
                let Some(op) = op else {
                    base = i;
                    continue;
                };
                let num: String = rest[op.len()..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if let Ok(v) = num.parse::<f32>() {
                    out.push((op.to_string(), v, ln + 1));
                }
                base = i;
            }
        }
        out
    }

    fn holds(op: &str, flag: f32, rhs: f32) -> bool {
        match op {
            "<=" => flag <= rhs,
            ">=" => flag >= rhs,
            "==" => flag == rhs,
            "!=" => flag != rhs,
            "<" => flag < rhs,
            ">" => flag > rhs,
            other => panic!("未知算子 {other}"),
        }
    }

    let build = std::fs::read_to_string("build.rs")
        .expect("读 build.rs 失败（测试工作目录应为仓库根）");
    let lines: Vec<&str> = build.lines().collect();
    let from = lines
        .iter()
        .position(|l| l.contains("const FRAGMENT_SHADER_WGSL"))
        .expect("找不到 FRAGMENT_SHADER_WGSL 起点");
    let to = lines[from + 1..]
        .iter()
        .position(|l| l.contains("const MESH_SHADER_WGSL"))
        .map(|p| p + from + 1)
        .expect("找不到 FRAGMENT_SHADER_WGSL 终点（MESH_SHADER_WGSL 之后）");

    let a = atoms(&lines[from..to].join("\n"));
    assert!(
        a.len() >= 8,
        "片元里只解析到 {} 条 flat_flag 判据（应 >= 8）⇒ 着色器改了形态或解析器坏了，\
         本测试会因'没解析到'而假通过，必须先修解析器再下结论",
        a.len()
    );
    assert!(
        a.iter().any(|(_, v, _)| (*v - 1.02).abs() < 1e-6),
        "一条阈值 1.02 的尺寸门都没找到 ⇒ 门被删了或本测试期望已过时（§22.7）"
    );

    let diff: Vec<String> = a
        .iter()
        .filter(|(op, v, _)| {
            (v - 1.02).abs() > 1e-6 && holds(op, 1.0, *v) != holds(op, 1.05, *v)
        })
        .map(|(op, v, ln)| format!("build.rs:{ln}  flat_flag {op} {v}"))
        .collect();
    assert!(
        diff.is_empty(),
        "这些 flat_flag 判据会让 1.05（被尺寸门挡下的小件）与 1.0 走不同分支：{}\
         ⇒ 要么把阈值挪出 (1.0, 1.05] 区间，要么显式写成对 1.05 也成立",
        diff.join("；")
    );
}

/// 🔴 每把 GLB 枪模的索引必须落在顶点数以内，且顶点不得含 NaN/Inf。
///
/// 存在理由（2026-09-15）：**按 2 切枪（AK-104）会直接把设备打掉**
/// （`vkQueueSubmit` → `VK_ERROR_DEVICE_LOST`）。越界索引在 GPU 上是**顶点抓取越界**：
/// 不报 VUID、不 panic，只是整台设备消失 —— 而索引是 GLB 解析器算出来的，
/// 只要合并多 primitive 时漏加基址偏移就会整段偏出去。
/// 这条测试把"能不能安全上传"变成上传**之前**就能判的纯数字判据。
#[test]
fn gun_glb_indices_all_in_range() {
    let mut checked = 0;
    for entry in std::fs::read_dir("assets/guns").expect("assets/guns 目录必须存在") {
        let path = entry.expect("读取 assets/guns 项失败").path();
        if path.extension().and_then(|s| s.to_str()) != Some("glb") {
            continue;
        }
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("枪模文件名必须是 UTF-8")
            .to_string();
        let Some((verts, indices)) = GameApp::load_gun_glb(&key) else {
            continue;
        };
        checked += 1;
        assert!(!verts.is_empty(), "{key}: 顶点不得为空");
        let max_i = indices.iter().copied().max().unwrap_or(0);
        assert!(
            (max_i as usize) < verts.len(),
            "{key}: 索引越界 —— 最大索引 {max_i} ≥ 顶点数 {}（GPU 顶点抓取越界 = device lost）",
            verts.len()
        );
        for (i, v) in verts.iter().enumerate() {
            assert!(
                v.pos.iter().all(|c| c.is_finite()) && v.color.iter().all(|c| c.is_finite()),
                "{key}: 顶点 #{i} 含 NaN/Inf（pos={:?} color={:?}）",
                v.pos,
                v.color
            );
        }
    }
    // 2026-10-30：许可清理后仓库只剩 **8 件** 第三方枪模（其中 ak12 是 GLB 自带
    // CC BY 元数据的那件）；另外 8 件追不到原始出处，已从仓库移出（见
    // `docs/WEAPON-LICENCE-AUDIT.md` §3）。这条断言的本意是"确实扫到了一批文件"，
    // 不是钉死某个数字，所以跟着资产清单走 8；**不要**为了让数字好看而把模型加回来。
    assert!(checked >= 8, "至少应校验到 8 把 GLB 枪模，实际 {checked}");
}

/// 键与资产文件名必须对得上（2026-10-31 实测踩过）：`ak12m` 的 GLB 叫 `ak12.glb`，
/// 少了映射 ⇒ `exists()` 恒 false ⇒ 主力枪静默掉到程序化模型。
#[test]
fn gun_key_maps_to_the_shipped_glb_name() {
    assert_eq!(GameApp::gun_glb_stem("ak12m"), "ak12");
    assert_eq!(GameApp::gun_glb_stem("pkm"), "pkm");
    // 凡是仓库里真有 GLB 的键，映射后必须能找到文件（反向：把映射改坏 ⇒ 这里红）
    for entry in std::fs::read_dir("assets/guns").expect("assets/guns 必须存在") {
        let name = entry.expect("读目录项").file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".glb") else { continue };
        let key = if stem == "ak12" { "ak12m" } else { stem };
        assert_eq!(
            GameApp::gun_glb_stem(key),
            stem,
            "键 {key} 映射到 {}，但库里是 {stem}.glb",
            GameApp::gun_glb_stem(key)
        );
    }
    assert!(
        GameApp::load_gun_glb("ak12m").is_some(),
        "ak12m 有 assets/guns/ak12.glb，却返回 None（映射又丢了？）"
    );
}

/// **每把武器都必须有自己的模型来源**（2026-10-31）：要么 `assets/guns/{key}.glb` 在，
/// 要么 `engine::guns` 里有它的程序化模型。两者都没有的键只能借用 `ak12.glb`，
/// 换枪就等于没换外观 —— 这正是许可清理后暴露出来的问题。
#[test]
fn every_weapon_has_a_model_source() {
    let mut procedural_only = Vec::new();
    for spec in crate::engine::weapon_data::ALL_WEAPONS.iter() {
        let glb = format!("assets/guns/{}.glb", spec.key);
        let has_glb = std::path::Path::new(&glb).exists();
        let has_proc = crate::engine::guns::has_procedural_mesh(spec.key);
        assert!(
            has_glb || has_proc,
            "武器 '{}' 既没有 {} 也没有程序化模型 ⇒ 会静默借用 ak12 的外形",
            spec.key,
            glb
        );
        if !has_glb {
            procedural_only.push(spec.key);
        }
    }
    // 至少要有 8 把走程序化（2026-10-30 许可清理移出的那批），否则说明这个判据
    // 在"GLB 全在"的机器上恒真、测不到东西。
    assert!(
        procedural_only.len() >= 8,
        "只有 {} 把武器走程序化模型，预期 >=8（被移出仓库的枪应当靠自己的程序化模型）",
        procedural_only.len()
    );
}

/// 被移出仓库的 8 件枪模，**必须回落到自己的程序化模型**，而不是 `ak12.glb`。
/// 反向验证：把 `load_gun_glb` 里 `has_procedural_mesh` 那条分支删掉 ⇒ 这里拿到 `Some`
/// （= ak12 的网格）⇒ 红。
#[test]
fn removed_gun_glbs_use_their_own_procedural_mesh() {
    for key in [
        "ak104", "ash12", "pkm", "pkp", "rpk16", "saiga12", "sv98", "svd12",
    ] {
        assert!(
            !std::path::Path::new(&format!("assets/guns/{key}.glb")).exists(),
            "{key}.glb 又回到仓库了？许可清理的结论见 docs/WEAPON-LICENCE-AUDIT.md"
        );
        assert!(
            GameApp::load_gun_glb(key).is_none(),
            "{key} 应当返回 None（走自己的程序化模型），却拿到了 GLB 网格"
        );
        assert!(
            crate::engine::guns::has_procedural_mesh(key),
            "{key} 没有程序化模型，回退链会掉到 ak12"
        );
    }
    // 有 GLB 的键照旧走 GLB：这条防止"把回退顺序改反了"。
    // ⚠ 这一条在测试进程里**不能**断言 "ak12m 返回 Some"：`load_gun_glb` 走
    // `asset_source::global()`，而 10 号那把枪的 GLB 有 2.3 MB、解析要走完整 glTF 路径，
    // 单元测试只该钉住**回退分支**。反过来钉一条不容易被误改的事实：ak12m 的 GLB 在库里。
    assert!(
        std::path::Path::new("assets/guns/ak12.glb").exists(),
        "ak12m 的 GLB 不在库里 ⇒ 回退链的最后一级也没了"
    );
}

/// raw 不可用的平台（本机 Windows）**连试都不许试** `Locked`。
/// 闭包写成 panic 而不是返回 false，是为了把"没被调用"也钉住 ——
/// 若哪天有人把 `raw_motion &&` 去掉，这条测试会立刻炸，而不是静默退化成
/// "试了 Locked 拿到 Ok，于是 cursor_locked = true，视角从此无输入"。
#[test]
fn grab_plan_never_tries_locked_without_raw_motion() {
    let (locked, grabbed) = cursor_grab_plan(
        false,
        || panic!("raw 不可用的平台不得尝试 Locked"),
        || true,
    );
    assert!(!locked, "raw 不可用时 cursor_locked 必须为 false");
    assert!(grabbed, "应退到 Confined 并报告已抓住");

    let (locked, grabbed) = cursor_grab_plan(
        false,
        || panic!("raw 不可用的平台不得尝试 Locked"),
        || false,
    );
    assert!(!locked);
    assert!(!grabbed, "Confined 也失败时应如实报告未抓住");
}

/// raw 可用（X11/Wayland/macOS）时保持既有优先级：Locked → Confined。
#[test]
fn grab_plan_prefers_locked_where_raw_motion_exists() {
    assert_eq!(cursor_grab_plan(true, || true, || false), (true, true));
    assert_eq!(cursor_grab_plan(true, || false, || true), (false, true));
    assert_eq!(cursor_grab_plan(true, || false, || false), (false, false));
}

/// Windows 上 RAW_MOUSE_MOTION 必须是 false：winit 的 Windows 后端不构造
/// DeviceEvent::MouseMotion，为真会让 capture 走进无输入的死路。
#[test]
#[cfg(target_os = "windows")]
fn windows_backend_has_no_raw_mouse_motion() {
    assert!(
        !RAW_MOUSE_MOTION,
        "winit 的 Windows 后端只发 Added/Removed；这里若为真，锁定即等于视角失效"
    );
}

/// 以恒定速度直行 `secs` 秒，返回积分后的摆动状态。
/// `dt` 故意可传入不同帧率，用于断言"同一行程得到同一状态"。
/// 进循环前先 tick 一次播种 `prev_pos`，否则两种帧率会各少吃一步的行程。
///
/// 帧数必须**四舍五入**：`(0.5f32 / (1.0f32 / 30.0)) as usize` 截断成 14 而不是 15
/// （f32 里 1/30 的倒数乘回来是 14.999999），于是两种帧率模拟的根本不是同一段时长，
/// 这条测试就从"验证帧率无关"变成了"验证两个不同时长相等"，必然红。
fn walk(dt: f32, secs: f32, speed: f32) -> GunSway {
    let mut s = GunSway::new();
    let right = glam::Vec3::X;
    let fwd = glam::Vec3::NEG_Z;
    let mut pos = glam::Vec3::ZERO;
    s.tick(dt, pos, right, fwd, false, false);
    let step = speed * dt;
    for _ in 0..(secs / dt).round() as usize {
        pos += fwd * step;
        s.tick(dt, pos, right, fwd, false, false);
    }
    s
}

/// 帧率无关性：以 6 m/s 直行 0.5 秒，165 fps 与 30 fps 必须得到同样的相位和包络
/// （旧实现用 `anim_clock * 7.5` 累积时间相位 + 逐帧原始速度，两者都与帧率耦合）。
/// 0.5 s 让总相位落在 π 附近——刻意避开 2π 回绕点，否则比较的是回绕后的余数。
#[test]
fn gun_sway_is_framerate_independent() {
    let fast = walk(1.0 / 165.0, 0.5, 6.0);
    let slow = walk(1.0 / 30.0, 0.5, 6.0);
    assert!(
        (fast.stride - slow.stride).abs() < 0.05,
        "同样行程后相位应一致：{} vs {}",
        fast.stride,
        slow.stride
    );
    assert!(
        (fast.speed - slow.speed).abs() < 0.25,
        "低通后的速度应基本与帧率无关：{} vs {}",
        fast.speed,
        slow.speed
    );
}

/// 判据：静止呼吸微摆**有界、两轴都真的在动、且 y 走 2× 频率**（画"∞"而不是来回直线）。
/// 并钉住"长时间运行不漂"（纯函数、无累积量）。
#[test]
fn idle_sway_is_bounded_and_figure_eight() {
    let mut max_x = 0.0f32;
    let mut max_y = 0.0f32;
    let mut x_changed = false;
    let mut y_changed = false;
    let mut prev = idle_sway(0.0);
    for i in 0..=2000 {
        let t = i as f32 * 0.01;
        let (x, y) = idle_sway(t);
        assert!(
            (-1.0..=1.0).contains(&x) && (-1.0..=1.0).contains(&y),
            "单位轨迹必须有界：({x}, {y})"
        );
        max_x = max_x.max(x.abs());
        max_y = max_y.max(y.abs());
        if (x - prev.0).abs() > 1e-3 {
            x_changed = true;
        }
        if (y - prev.1).abs() > 1e-3 {
            y_changed = true;
        }
        prev = (x, y);
    }
    assert!(x_changed && y_changed, "两轴都必须真的在动");
    assert!(max_x > 0.99, "x 轴应达到满幅，实际 {max_x}");
    assert!((max_y - 0.6).abs() < 0.01, "y 轴应是 0.6 倍幅值，实际 {max_y}");
    // 2× 频率：x 走半个周期时 y 回到同号（一圈"∞"的两个环在 y 上同相）
    let period_x = 1.0 / GUN_IDLE_HZ;
    let (x0, y0) = idle_sway(0.1);
    let (x1, y1) = idle_sway(0.1 + period_x * 0.5);
    assert!((x1 + x0).abs() < 0.02, "x 半周期后应反相：{x0} vs {x1}");
    assert!((y1 - y0).abs() < 0.02, "y 是 2× 频率 ⇒ 半周期后同相：{y0} vs {y1}");
}

/// 判据：冲刺姿态包络**必须帧率无关地收敛**，且松开 Shift 后回到精确 0
/// （0 是 A/B 与"姿态项彻底不参与"的判据）。
#[test]
fn gun_sprint_pose_converges_and_is_framerate_independent() {
    let run = |dt: f32, secs: f32, sprinting: bool| {
        let mut s = GunSway::new();
        for _ in 0..(secs / dt).round() as usize {
            s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, sprinting);
        }
        s
    };
    let fast = run(1.0 / 165.0, 0.5, true);
    let slow = run(1.0 / 30.0, 0.5, true);
    assert!(fast.sprint > 0.99, "0.5 s 后应基本收敛到满姿态：{}", fast.sprint);
    assert!(
        (fast.sprint - slow.sprint).abs() < 0.02,
        "冲刺姿态低通应与帧率无关：{} vs {}",
        fast.sprint,
        slow.sprint
    );
    // 松开 Shift：回到精确 0（不是"接近 0"）
    let mut s = fast;
    for _ in 0..200 {
        s.tick(1.0 / 165.0, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, false);
    }
    assert_eq!(s.sprint, 0.0, "松开冲刺后包络应收敛到精确 0");
    // 传送帧不该抹掉姿态目标（冲刺状态与位置无关）
    let mut t = run(1.0 / 165.0, 0.3, true);
    t.tick(
        1.0 / 165.0,
        glam::Vec3::new(80.0, 0.0, 80.0),
        glam::Vec3::X,
        glam::Vec3::NEG_Z,
        false,
        true,
    );
    assert!(t.sprint > 0.9, "传送帧后冲刺姿态应保持：{}", t.sprint);
}

/// 有界性 + 相位回绕：长时间运行后相位仍在 [0, 2π)、速度不发散、包络 ≤1。
///
/// 这里**不**用紧容差查"不过冲"：600 s × 6 m/s 把坐标累加到 3.6e3 m，
/// `now_pos - prev_pos` 在该量级下每帧带约一个 ulp（≈2.4e-4）的舍入误差，
/// 再除以 dt=1/165 s 放大成约 7e-3 m/s 的**输入噪声**。低通本身单调逼近、不会过冲，
/// 超的是这个噪声。紧的那条断言在下面的 `gun_sway_low_pass_does_not_overshoot`。
#[test]
fn gun_sway_stays_bounded_and_wrapped() {
    let s = walk(1.0 / 165.0, 600.0, 6.0);
    assert!(s.stride >= 0.0 && s.stride < std::f32::consts::TAU);
    assert!(
        s.speed <= 6.05,
        "长时间运行后速度发散，说明低通或相位累加失去了有界性：{}",
        s.speed
    );
    assert!(s.kick <= 1.0);
}

/// 不过冲（紧容差）：短时运行下坐标只有十几米，浮点噪声比容差小三个数量级，
/// 因此这条能真正守住"指数低通单调逼近、绝不越过输入值"。
#[test]
fn gun_sway_low_pass_does_not_overshoot() {
    let s = walk(1.0 / 165.0, 3.0, 6.0);
    assert!(
        s.speed <= 6.0 + 1e-4,
        "指数低通是单调逼近，不应过冲：{}",
        s.speed
    );
    assert!(s.speed > 5.99, "3 秒后应已收敛到满幅，实际 {}", s.speed);
}

/// 瞬移/重生保护：单帧几十米的位移不得被当成巨型速度（旧实现没有这层保护，
/// 而且 `player_speed()` 恒为 0，两种错法都会让摆动不可信）
#[test]
fn gun_sway_ignores_teleport() {
    let mut s = walk(1.0 / 165.0, 0.5, 6.0);
    assert!(s.speed > 5.0, "前置条件：应先积分出满幅速度");
    s.tick(
        1.0 / 165.0,
        glam::Vec3::new(0.0, 0.0, -50.0),
        glam::Vec3::X,
        glam::Vec3::NEG_Z,
        false,
        false,
    );
    assert!(s.speed < 1e-3, "传送帧速度应归零，实际 {}", s.speed);
}

/// 后坐包络连续：击发后逐帧单调下降，单帧变化量 ≤8%
/// （旧实现在 0.25 s 整点把阻尼从 0.15 阶跃到 1.0，单帧变化 0.85 = 位置跳变）
#[test]
fn gun_recoil_kick_decays_continuously() {
    let mut s = GunSway::new();
    let dt = 1.0 / 165.0;
    s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, true, false);
    assert!(s.kick > 0.9, "击发帧应接近满幅后坐");
    let mut prev = s.kick;
    for _ in 0..60 {
        s.tick(dt, glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::NEG_Z, false, false);
        assert!(s.kick <= prev, "包络不得回升：{} > {}", s.kick, prev);
        assert!(
            prev - s.kick < 0.08,
            "单帧后坐变化量应连续，实际 {}",
            prev - s.kick
        );
        prev = s.kick;
    }
    assert!(s.kick < 0.01, "0.36 s 后应基本归零，实际 {}", s.kick);
}

/// 枪模顶点色：坏资产（baseColorFactor 0.057）经反照率补偿后必须给出
/// 可用的明暗区间，而不是旧公式的 0.049..0.066（梯度 ±0.017 = 纯黑剪影）
#[test]
fn gun_bake_color_keeps_a_readable_gradient() {
    let boost = GUN_REF_ALBEDO / 0.0768; // ak12.glb 实测最亮材质
    let raw = [0.0573, 0.0573, 0.0573];
    let dirs = [
        glam::Vec3::new(-0.45, 0.80, -0.30), // 迎光面（= 主光方向）
        glam::Vec3::Y,
        glam::Vec3::NEG_Y,
        glam::Vec3::X,
        glam::Vec3::NEG_X,
        glam::Vec3::NEG_Z, // 朝向镜头的侧面
        glam::Vec3::Z,     // 枪口方向
    ];
    let mut lo = f32::MAX;
    let mut hi: f32 = 0.0;
    for d in dirs {
        let n = d.normalize();
        let c = fp_gun_bake_color(n, raw, boost);
        for ch in c {
            assert!(ch.is_finite() && (0.0..=1.0).contains(&ch), "{:?} → {:?}", n, c);
            lo = lo.min(ch);
            hi = hi.max(ch as f32);
        }
    }
    assert!(lo > 0.03, "最暗面不应是纯黑，实际 {}", lo);
    assert!(hi < 0.99, "最亮面不得削顶，实际 {}", hi);
    assert!(
        hi / lo >= 4.0,
        "明暗比过小说明仍是平面剪影：{:.4} / {:.4}",
        hi,
        lo
    );
}
