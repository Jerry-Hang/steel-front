//! 插件注册表（引擎化 P7a，见 `docs/ENGINE-PLAN.md` §5）。
//!
//! **为什么是"同进程注册"而不是动态库**：Rust 没有稳定 ABI，`dlopen` 出来的 Rust `dylib`
//! 在编译器升级后会静默错位（不崩不报，只是字段读到别处）。所以插件与引擎一起编译，
//! 靠这里的注册表接线；"可替换"的部分走 AI 接口（P5）而不是 ABI。
//!
//! 三张表，**同一份数据两个入口**（用户要求的"人机共编"）：
//! - `systems`：按阶段顺序执行的每帧系统；
//! - `commands`：具名命令 —— 编辑器按钮、`RV3D_PLUGIN_CMD` 诊断、将来的 AI 接口
//!   调的都是**同一张表里的同一个函数**，不允许有"只有 API 能做"的暗路（铁律 4）；
//! - `names`：注册了哪些插件（诊断与编辑器面板用）。

use crate::engine::scene::Scene;
use glam::Vec3;

/// 系统执行阶段。顺序固定（`PreUpdate → Update → PostUpdate → PreRender`），
/// 与 `runtime` 将来的帧结构一一对应；**不要**按注册顺序执行 —— 那会让"谁先 add
/// 谁先跑"这种隐式依赖混进插件里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    PreUpdate = 0,
    Update = 1,
    PostUpdate = 2,
    PreRender = 3,
}

impl Phase {
    pub const ALL: [Phase; 4] = [
        Phase::PreUpdate,
        Phase::Update,
        Phase::PostUpdate,
        Phase::PreRender,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::PreUpdate => "pre_update",
            Phase::Update => "update",
            Phase::PostUpdate => "post_update",
            Phase::PreRender => "pre_render",
        }
    }
}

/// 系统拿到的上下文。**刻意收窄**：插件碰不到 `Game` 的私有字段，只能改场景、读时间、
/// 写缓冲。将来要放开什么，就往这里加一个字段 —— 加字段的地方就是权限清单。
pub struct PluginCtx<'a> {
    pub scene: &'a mut Scene,
    /// 本帧的累计游戏时间（秒）。
    pub time: f32,
    /// 本帧 dt（秒）。
    pub dt: f32,
    /// 输出缓冲（诊断/上报用；编辑器面板与 AI 接口读同一份）。
    pub out: Vec<String>,
    /// 骨骼姿势烘焙请求 `(资产路径, 时间 t)`。
    ///
    /// 为什么是"请求"而不是当场烘：烘焙要往**道具套件**里塞一件新网格，
    /// 而套件由 `App` 持有（`PluginCtx` 只借到 `Scene`）。命令只登记意图，
    /// `App` 在帧末统一处理 —— 与 `selection` 的写回是同一套模式。
    pub bake_requests: Vec<(String, f32)>,
    /// 撤销栈（编辑器与 AI 共用；`run_command` 在改动前后自动记录）。
    pub undo: &'a mut crate::engine::undo::UndoStack,
    /// 当前 gizmo 模式（编辑器与 AI 共用；命令 `gizmo.mode` 改它）。
    pub gizmo_mode: crate::engine::gizmo::Mode,
    /// 当前相机视角（取点命令要用；编辑器与 AI 接口都需要"从这里看到什么"）。
    pub view: Option<crate::engine::gizmo::ViewInfo>,
    /// **当前选中实体**（编辑器选中的那个；AI 接口也能改它）。
    /// `None` = 没选中 —— 跟"对选中项操作"有关的命令必须报错而不是猜一个。
    pub selection: Option<crate::engine::scene::EntityId>,
}

impl<'a> PluginCtx<'a> {
    /// 撤销栈由调用方持有并传进来：**上下文不持有它**，
    /// 因为上下文是每帧临时的，而撤销栈要活过整个会话。
    pub fn new(
        scene: &'a mut Scene,
        undo: &'a mut crate::engine::undo::UndoStack,
        time: f32,
        dt: f32,
    ) -> Self {
        PluginCtx {
            scene,
            undo,
            time,
            dt,
            out: Vec::new(),
            bake_requests: Vec::new(),
            gizmo_mode: crate::engine::gizmo::Mode::Translate,
            view: None,
            selection: None,
        }
    }

    /// 往上报缓冲里写一行（机读格式由调用方决定）。
    pub fn emit(&mut self, line: impl Into<String>) {
        self.out.push(line.into());
    }
}

pub type SystemFn = fn(&mut PluginCtx<'_>);

/// 命令：`args` 是空格切开的参数（与将来的文本协议一致）。
/// 返回 `Ok(回显)` 或 `Err(原因)`；**不要**在这里吞错误 —— 编辑器与 AI 都要看到失败原因。
pub type CommandFn = fn(&mut PluginCtx<'_>, &[&str]) -> Result<String, String>;

struct SystemEntry {
    phase: Phase,
    name: &'static str,
    func: SystemFn,
}

struct CommandEntry {
    name: &'static str,
    help: &'static str,
    func: CommandFn,
}

/// 取点半径（米）。场景层还没有包围盒组件，先用固定球半径：
/// 太小要像素级对准，太大相邻实体会被抢走。等实体带上包围盒后换成盒判据。
const PICK_RADIUS: f32 = 0.9;

/// 插件注册表。生命周期 = 进程；构造后不再增删（不做热重载，见计划 §5.3）。
#[derive(Default)]
pub struct PluginRegistry {
    systems: Vec<SystemEntry>,
    commands: Vec<CommandEntry>,
    plugins: Vec<&'static str>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记一个插件名（诊断/编辑器面板列出"装了哪些"）。
    pub fn add_plugin(&mut self, name: &'static str) {
        if !self.plugins.contains(&name) {
            self.plugins.push(name);
        }
    }

    pub fn add_system(&mut self, phase: Phase, name: &'static str, func: SystemFn) {
        // 同名系统重复注册 = 编程错误，当场暴露（否则"跑了两遍"很难查）。
        assert!(
            !self.systems.iter().any(|s| s.name == name),
            "系统名重复注册：{name}"
        );
        self.systems.push(SystemEntry {
            phase,
            name,
            func,
        });
    }

    pub fn add_command(
        &mut self,
        name: &'static str,
        help: &'static str,
        func: CommandFn,
    ) -> &mut Self {
        assert!(
            !self.commands.iter().any(|c| c.name == name),
            "命令名重复注册：{name}"
        );
        self.commands.push(CommandEntry { name, help, func });
        self
    }

    pub fn plugin_names(&self) -> &[&'static str] {
        &self.plugins
    }

    /// 命令名清单（编辑器列表 / AI 接口的 capabilities 都读它）。
    pub fn command_names(&self) -> Vec<&'static str> {
        self.commands.iter().map(|c| c.name).collect()
    }

    /// `name` 与 `help` 的对照（给编辑器提示与 AI 自描述用）。
    pub fn command_help(&self) -> Vec<(&'static str, &'static str)> {
        self.commands.iter().map(|c| (c.name, c.help)).collect()
    }

    /// 按阶段执行本帧所有系统。**同一阶段内按注册顺序**，阶段之间按 `Phase` 的顺序。
    pub fn run_phase(&self, phase: Phase, ctx: &mut PluginCtx<'_>) {
        for s in self.systems.iter().filter(|s| s.phase == phase) {
            (s.func)(ctx);
        }
    }

    /// 一次跑完四个阶段（帧循环的正常用法）。
    pub fn run_frame(&self, ctx: &mut PluginCtx<'_>) {
        for p in Phase::ALL {
            self.run_phase(p, ctx);
        }
    }

    /// 执行一条具名命令。**唯一入口** —— 编辑器按钮与 AI 接口都走这里。
    pub fn run_command(&self, name: &str, args: &[&str], ctx: &mut PluginCtx<'_>) -> Result<String, String> {
        let entry = self
            .commands
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| {
                format!(
                    "没有这条命令：{name}；已注册：{:?}",
                    self.command_names()
                )
            })?;
        // 🔴 **撤销记录放在这一处**，而不是每个命令自己记：
        // 命令表是编辑器与 AI 的**唯一入口**，记在这里就保证两条入口都能撤销，
        // 也不会出现"新加的命令忘了记录"（那类遗漏在编辑器里极难发现 ——
        // 表现为"有些操作能撤销、有些不能"，而没人知道是哪些）。
        //
        // 撤销/重做自身要排除，否则它们会被自己记录 ⇒ 撤销一步之后栈顶是自己。
        let track = name != "scene.undo" && name != "scene.redo";
        let before = if track {
            crate::engine::scene_io::to_toml(ctx.scene).ok()
        } else {
            None
        };
        let ret = (entry.func)(ctx, args);
        if track {
            if ret.is_ok() {
                if let Some(before) = before {
                    // 只有**真的变了**才记录：否则一次只读取的命令也会压一格，
                    // 用户按两次撤销却什么都没变（"撤销没反应"的典型来源）。
                    let after = crate::engine::scene_io::to_toml(ctx.scene).unwrap_or_default();
                    let was = crate::engine::scene_io::to_toml(ctx.scene).is_ok();
                    if was && after != before {
                        ctx.undo.record(before);
                    }
                }
            }
        }
        let ret = ret?;
        // `plugin.list` 需要看到注册表自身（函数指针拿不到 `self`），
        // 这里用约定标记替换成真正的自描述 —— 编辑器与 AI 读到的就是这份。
        if ret == "SELF" {
            let mut out = format!("plugins={:?}", self.plugin_names());
            for (n, h) in self.command_help() {
                out.push_str(&format!("; {n} = {h}"));
            }
            return Ok(out);
        }
        Ok(ret)
    }
}

/// 内置调试插件：证明注册表真的在跑（系统 + 一条命令），也给编辑器一个最小的可用命令。
///
/// ⚠️ 它不是"示例代码"：`RV3D_PLUGIN_CMD=scene.count` 走的就是它，
/// 所以这条链路每一轮回归都在被真实调用。
pub fn register_debug_plugin(reg: &mut PluginRegistry) {
    reg.add_plugin("debug");
    // 每帧系统：只在被要求时输出，避免刷屏（`RV3D_PLUGIN_DIAG=1`）。
    reg.add_system(Phase::Update, "debug.tick", |ctx| {
        if crate::syscfg::flag("RV3D_PLUGIN_DIAG") {
            ctx.emit(format!(
                "plugin: tick phase={} t={:.2} dt={:.4} entities={}",
                Phase::Update.name(),
                ctx.time,
                ctx.dt,
                ctx.scene.len()
            ));
        }
    });
    reg.add_command(
        "scene.count",
        "打印场景实体数：scene.count",
        |ctx, _args| Ok(format!("entities={}", ctx.scene.len())),
    );
    reg.add_command(
        "scene.spawn",
        "在原点新建一个实体：scene.spawn",
        |ctx, _args| {
            let id = ctx
                .scene
                .spawn(None, crate::engine::scene::Transform::IDENTITY);
            // **新建的实体接管选中项**：这样 AI/脚本可以"spawn → drag → save"走通一整条
            // 不依赖玩家身体的链路。玩家实体是「玩家体位置的镜像」会被每帧覆盖，
            // 场景自己的实体不会 —— 这是在 P1b/P6 把权威方向翻过来之前的正确用法。
            ctx.selection = Some(id);
            Ok(format!("spawned entities={} id={id:?}", ctx.scene.len()))
        },
    );
    // ---- P2：三轴 gizmo（射线取点与拖动）----
    //
    // 这两条命令就是编辑器鼠标拖动时走的**同一个函数**（`gizmo.rs` 的纯数学）。
    // `gizmo.pick` 给出沿轴的米制参数，`gizmo.drag` 用两次取点的差写回玩家平移。
    reg.add_command(
        "gizmo.pick",
        "沿轴取点：gizmo.pick <X|Y|Z> <ox> <oy> <oz> <dx> <dy> <dz>",
        |_ctx, args| {
            let axis = args
                .first()
                .and_then(|s| crate::engine::gizmo::Axis::parse(s))
                .ok_or_else(|| "缺轴或轴非法（X/Y/Z）".to_string())?;
            if args.len() < 8 {
                return Err("参数不足：需要 轴 + 射线原点3 + 方向3".to_string());
            }
            let f = |i: usize| -> Result<f32, String> {
                args[i]
                    .parse::<f32>()
                    .map_err(|_| format!("第 {i} 个参数不是数字：{}", args[i]))
            };
            let o = glam::Vec3::new(f(1)?, f(2)?, f(3)?);
            let d = glam::Vec3::new(f(4)?, f(5)?, f(6)?);
            match crate::engine::gizmo::axis_param(o, d, glam::Vec3::ZERO, axis.dir()) {
                Some(t) => Ok(format!("t={t:.6} axis={}", axis.name())),
                None => Err("射线与轴近平行，拒绝取点".to_string()),
            }
        },
    );
    reg.add_command(
        "gizmo.drag",
        "把玩家沿轴拖到新参数：gizmo.drag <X|Y|Z> <t0> <t1>",
        |ctx, args| {
            let axis = args
                .first()
                .and_then(|s| crate::engine::gizmo::Axis::parse(s))
                .ok_or_else(|| "缺轴或轴非法（X/Y/Z）".to_string())?;
            if args.len() < 3 {
                return Err("参数不足：需要 轴 + t0 + t1".to_string());
            }
            let t0: f32 = args[1].parse().map_err(|_| "t0 不是数字".to_string())?;
            let t1: f32 = args[2].parse().map_err(|_| "t1 不是数字".to_string())?;
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let cur = ctx
                .scene
                .world_matrix(sel)
                .map(|m| m.w_axis.truncate())
                .map_err(|_| "选中实体句柄失效".to_string())?;
            // 构造两条"恰好取点到 t0 / t1"的射线：与轴垂直、交于 axis*t。
            // 这样无鼠标的入口（脚本 / AI / 自检）走的也是**鼠标那条数学路径**，
            // 不是另写一份"直接加增量"的绕过路径。
            let ray_at = |t: f32| {
                (
                    axis.dir() * t + Vec3::new(1.0, 1.0, 0.0),
                    -Vec3::new(1.0, 1.0, 0.0),
                )
            };
            let (o0, d0) = ray_at(t0);
            let start = crate::engine::gizmo::TranslateDrag::begin(axis, cur, o0, d0)
                .ok_or_else(|| "起始射线与轴近平行".to_string())?;
            let (o1, d1) = ray_at(t1);
            let moved = start
                .drag(o1, d1)
                .ok_or_else(|| "结束射线与轴近平行".to_string())?;
            ctx.scene
                .set_local(sel, crate::engine::scene::Transform::from_translation(moved))
                .map_err(|_| "写回 Transform 失败".to_string())?;
            Ok(format!(
                "drag axis={} delta={:.4} from=({:.3},{:.3},{:.3}) to=({:.3},{:.3},{:.3})",
                start.axis().name(),
                t1 - t0,
                cur.x,
                cur.y,
                cur.z,
                moved.x,
                moved.y,
                moved.z
            ))
        },
    );
    // ---- P3：场景存盘 / 读回（编辑器的"改完能留下"，也是 P5 往返判据的前提）----
    reg.add_command(
        "scene.save",
        "把场景存成 TOML：scene.save <路径>",
        |ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let text = crate::engine::scene_io::to_toml(ctx.scene)
                .map_err(|e| format!("存盘失败：{e}"))?;
            std::fs::write(path, text).map_err(|e| format!("写文件失败：{e}"))?;
            Ok(format!("saved entities={} path={path}", ctx.scene.len()))
        },
    );
    reg.add_command(
        "scene.load",
        "从 TOML 读回场景并统计：scene.load <路径>（只读校验，不改当前场景）",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("读文件失败：{e}"))?;
            let (scene, _map) =
                crate::engine::scene_io::from_toml(&text).map_err(|e| format!("读回失败：{e}"))?;
            Ok(format!("loaded entities={}", scene.len()))
        },
    );
    // ---- P3c：鼠标取点（编辑器点选与 AI 接口共用）----
    reg.add_command(
        "pick.at",
        "按屏幕坐标点选：pick.at <ndc_x> <ndc_y>（-1..1，屏幕中心 0,0）",
        |ctx, args| {
            let view = ctx.view.ok_or_else(|| "当前没有相机视角".to_string())?;
            if args.len() < 2 {
                return Err("参数不足：需要 ndc_x ndc_y".to_string());
            }
            let x: f32 = args[0].parse().map_err(|_| "ndc_x 不是数字".to_string())?;
            let y: f32 = args[1].parse().map_err(|_| "ndc_y 不是数字".to_string())?;
            match view.pick(ctx.scene, x, y, PICK_RADIUS) {
                Some((id, t)) => {
                    ctx.selection = Some(id);
                    Ok(format!(
                        "picked #{} t={t:.3}",
                        ctx.scene.index_of(id)
                    ))
                }
                None => Ok("picked none".to_string()),
            }
        },
    );
    // ---- P3d：鼠标式拖拽（与编辑器左键拖动同一条实现）----
    reg.add_command(
        "gizmo.mouse_drag",
        "模拟鼠标拖动把手：gizmo.mouse_drag <ndc0x> <ndc0y> <ndc1x> <ndc1y>",
        |ctx, args| {
            let view = ctx.view.ok_or_else(|| "当前没有相机视角".to_string())?;
            if args.len() < 4 {
                return Err("参数不足：需要 4 个 NDC 分量".to_string());
            }
            let f = |i: usize| -> Result<f32, String> {
                args[i].parse::<f32>().map_err(|_| format!("第 {i} 个参数不是数字"))
            };
            let (x0, y0, x1, y1) = (f(0)?, f(1)?, f(2)?, f(3)?);
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let cur = ctx
                .scene
                .local(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?
                .translation;
            match crate::engine::gizmo::mouse_drag_translate(&view, cur, (x0, y0), (x1, y1)) {
                Some((axis, moved)) => {
                    ctx.scene
                        .set_local(
                            sel,
                            crate::engine::scene::Transform {
                                translation: moved,
                                ..ctx.scene.local(sel).map_err(|_| "句柄失效".to_string())?
                            },
                        )
                        .map_err(|_| "写回失败".to_string())?;
                    Ok(format!(
                        "dragged axis={} from=({:.2},{:.2},{:.2}) to=({:.2},{:.2},{:.2})",
                        axis.name(), cur.x, cur.y, cur.z, moved.x, moved.y, moved.z
                    ))
                }
                None => Ok("dragged none（光标没落在把手上）".to_string()),
            }
        },
    );
    // ---- P4：骨骼探针（导入骨骼的第一个可用入口）----
    reg.add_command(
        "asset.skin_probe",
        "读 GLB 并报告骨骼与骨骼变换：asset.skin_probe <路径>",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let bytes = std::fs::read(path).map_err(|e| format!("读文件失败：{e}"))?;
            match crate::engine::skin::parse_glb_skin(&bytes) {
                Ok(None) => Ok(format!("{path}: 没有骨骼（skins 为空）")),
                Ok(Some((skin, positions))) => {
                    let err = crate::engine::skin::bind_pose_error(&skin, &positions);
                    Ok(format!(
                        "{path}: joints={} vertices={} bind_err={err:.6}",
                        skin.joint_count(),
                        skin.vertex_count()
                    ))
                }
                Err(e) => Err(format!("{path}: {e}")),
            }
        },
    );
    // ---- P4b：动画探针 ----
    reg.add_command(
        "asset.anim_probe",
        "读 GLB 并报告骨骼动画通道与采样：asset.anim_probe <路径>",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let bytes = std::fs::read(&path[..]).map_err(|e| format!("读文件失败：{e}"))?;
            let channels = crate::engine::anim::parse_glb_animations(&bytes)?;
            if channels.is_empty() {
                return Ok(format!("{path}: 没有动画"));
            }
            let dur = channels
                .iter()
                .map(|c| c.duration())
                .fold(0.0f32, f32::max);
            let mid: Vec<String> = channels
                .iter()
                .take(3)
                .map(|c| {
                    let v = c.sample(dur * 0.5);
                    format!(
                        "node{}:{}[{}] mid=({:.3},{:.3},{:.3},{:.3})",
                        c.node,
                        c.path,
                        c.interp.name(),
                        v.x,
                        v.y,
                        v.z,
                        v.w
                    )
                })
                .collect();
            Ok(format!(
                "{path}: channels={} duration={dur:.3} {}",
                channels.len(),
                mid.join(" ")
            ))
        },
    );
    // ---- P4c：姿势探针（动画 → 关节世界矩阵 → 骨骼变换顶点，整条 CPU 链）----
    reg.add_command(
        "asset.pose_probe",
        "按时间采样姿势并报告顶点位移：asset.pose_probe <路径> <t>",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let t: f32 = args
                .get(1)
                .ok_or_else(|| "缺时间参数 t".to_string())?
                .parse()
                .map_err(|_| "t 不是数字".to_string())?;
            let bytes = std::fs::read(&path[..]).map_err(|e| format!("读文件失败：{e}"))?;
            // 重复解析一次 JSON（探针命令，性能不必在意；不值得为它改解析器签名）
            let (json, _bin) = crate::engine::skin::split_glb(&bytes)?;
            let Some((skin, positions)) = crate::engine::skin::parse_glb_skin(&bytes)? else {
                return Ok(format!("{path}: 没有骨骼"));
            };
            let channels = crate::engine::anim::parse_glb_animations(&bytes)?;
            if channels.is_empty() {
                return Ok(format!("{path}: 有骨骼但没有动画"));
            }
            let joints: Vec<usize> = json
                .get("skins")
                .and_then(|s| s.as_arr())
                .and_then(|s| s.first())
                .and_then(|s| s.get("joints"))
                .and_then(|j| j.as_arr())
                .map(|a| {
                    a.iter()
                        .map(|v| v.as_f64().unwrap_or(0.0) as usize)
                        .collect()
                })
                .unwrap_or_default();
            let worlds = crate::engine::anim::sample_pose_worlds(&json, 0, &channels, t);
            let posed = crate::engine::anim::posed_positions(&skin, &joints, &positions, &worlds);
            let mut worst = 0.0f32;
            let mut worst_i = 0;
            for (i, (a, b)) in positions.iter().zip(&posed).enumerate() {
                let d = (*b - *a).length();
                if d > worst {
                    worst = d;
                    worst_i = i;
                }
            }
            Ok(format!(
                "{path}: joints={} channels={} t={t:.2} max_disp={worst:.4} at v{worst_i}",
                joints.len(),
                channels.len()
            ))
        },
    );
    // ---- P3f：检视面板的写入口（面板与 AI 接口共用）----
    //
    // 检视面板显示的就是 `editor::inspector_lines` 的四个字段；这里提供**同一个**写入口，
    // 保证"面板上改的"与"AI 改的"落到同一处、走同一条校验。
    reg.add_command(
        "scene.set",
        "设置选中实体的平移：scene.set <x> <y> <z>",
        |ctx, args| {
            if args.len() < 3 {
                return Err("参数不足：需要 x y z".to_string());
            }
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let mut v = [0.0f32; 3];
            for i in 0..3 {
                v[i] = args[i]
                    .parse()
                    .map_err(|_| format!("第 {i} 个参数不是数字：{}", args[i]))?;
            }
            let local = ctx
                .scene
                .local(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            ctx.scene
                .set_local(
                    sel,
                    crate::engine::scene::Transform {
                        translation: glam::Vec3::new(v[0], v[1], v[2]),
                        ..local
                    },
                )
                .map_err(|_| "写回失败".to_string())?;
            Ok(format!(
                "set #{} pos=({:.3},{:.3},{:.3})",
                ctx.scene.index_of(sel),
                v[0],
                v[1],
                v[2]
            ))
        },
    );
    reg.add_command(
        "scene.nudge",
        "按轴微调选中实体：scene.nudge <X|Y|Z> <增量>",
        |ctx, args| {
            let axis = args
                .first()
                .and_then(|s| crate::engine::gizmo::Axis::parse(s))
                .ok_or_else(|| "缺轴或轴非法（X/Y/Z）".to_string())?;
            let d: f32 = args
                .get(1)
                .ok_or_else(|| "缺增量".to_string())?
                .parse()
                .map_err(|_| "增量不是数字".to_string())?;
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let mut local = ctx
                .scene
                .local(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            local.translation += axis.dir() * d;
            let t = local.translation;
            ctx.scene
                .set_local(sel, local)
                .map_err(|_| "写回失败".to_string())?;
            Ok(format!(
                "nudge {} {:.3} → ({:.3},{:.3},{:.3})",
                axis.name(),
                d,
                t.x,
                t.y,
                t.z
            ))
        },
    );
    // 读回并**替换当前场景**（`scene.load` 仍然只做校验，两者界限分开）：
    // 编辑器里"打开一个场景"= 换掉整个世界；而"看看这个文件坏没坏"不该动现场。
    reg.add_command(
        "scene.load_into",
        "读回 TOML 并替换当前场景：scene.load_into <路径>",
        |ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let text =
                std::fs::read_to_string(&path[..]).map_err(|e| format!("读文件失败：{e}"))?;
            let n = crate::engine::scene_io::apply_to(ctx.scene, &text)
                .map_err(|e| format!("读回失败：{e}"))?;
            // 选中项可能指向被清掉的实体：清空而不是留一个悬空句柄
            ctx.selection = None;
            Ok(format!("loaded_into entities={n}"))
        },
    );
    // ---- P3h：资产列表（命令面，与面板同一份数据）----
    //
    // 列表本体在 `App` 里（要读盘、要懒扫描），命令只操作**一份共享的列表
    // 这里用"按路径重扫 + 分屏 + 选中"的极简形态：命令面能验证排序/分屏/选中，
    // 面板显示的就是同一份 `panel_lines()`。
    reg.add_command(
        "asset.browse",
        "列资产（分屏）：asset.browse <屏号> [每屏数]",
        |_ctx, args| {
            let page: usize = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let per: usize = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(8);
            let mut b = crate::engine::asset_browser::AssetBrowser::scan("assets/props", per)?;
            for _ in 0..page {
                b.next_page();
            }
            let mut out = b.panel_lines().join(" | ");
            if let Some(sel) = b.selected() {
                out.push_str(&format!(" ; selected={}", sel.path));
            }
            Ok(out)
        },
    );
    reg.add_command(
        "asset.count",
        "报告资产数量：asset.count",
        |_ctx, _args| {
            let b = crate::engine::asset_browser::AssetBrowser::scan("assets/props", 8)?;
            Ok(format!(
                "assets={} pages={} empty={}",
                b.len(),
                b.page_count(),
                b.is_empty()
            ))
        },
    );
    // 翻屏/选中的**行为**要给一条能被脚本驱动的入口：`asset.step <前后翻屏 -1/1> <选中位移>`
    // —— 三个方法（prev_page / move_selection / page）都在这条路径上被真实调用。
    reg.add_command(
        "asset.step",
        "翻屏并移动选中：asset.step <翻屏次数 ±> <选中位移 ±>",
        |_ctx, args| {
            let turn: isize = args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let shift: isize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let mut b = crate::engine::asset_browser::AssetBrowser::scan("assets/props", 8)?;
            for _ in 0..turn.abs() {
                if turn > 0 {
                    b.next_page();
                } else {
                    b.prev_page();
                }
            }
            b.move_selection(shift);
            Ok(format!(
                "page={} selected={} path={:?}",
                b.page(),
                b.selected_index(),
                b.selected().map(|e| e.path.as_str())
            ))
        },
    );
    // ---- P3i：把资产实例化进场景（导入模型的数据层）----
    //
    // 这一步做的是"场景里记住用的是哪个模型 + 尺寸与资产一致"，
    // **还不负责画**（渲染要等 `SceneDrawItem` 接上网格引用，那是下一步）。
    // 分两步做是有意的：仿真实体尺寸对不对可以精确判，而"画得像不像"不能。
    reg.add_command(
        "scene.instantiate",
        "按资产路径实例化：scene.instantiate <assets/props/x.glb>",
        |ctx, args| {
            let path = args.first().ok_or_else(|| "缺资产路径".to_string())?.to_string();
            // 🔴 走 AssetSource：Android 上 std::fs 读不到打包资源（架构判据也在管着）
            let bytes = crate::engine::asset_source::global().read(&path)?;
            let mesh = crate::engine::assets::parse_glb(&bytes)?;
            // 包围盒：由真实顶点算，实例尺寸必须与资产一致（这是本步的验收口径）
            let (lo, hi) = crate::engine::assets::mesh_bounds(&mesh)
                .ok_or_else(|| format!("{path}: 没有顶点"))?;
            let (lo, hi) = (
                glam::Vec3::new(lo[0], lo[1], lo[2]),
                glam::Vec3::new(hi[0], hi[1], hi[2]),
            );
            let half = ((hi - lo) * 0.5).max(glam::Vec3::splat(0.01));
            let center = (hi + lo) * 0.5;
            let id = ctx.scene.spawn(
                None,
                crate::engine::scene::Transform::from_translation(glam::Vec3::new(
                    center.x, lo.y.max(0.0), center.z,
                )),
            );
            ctx.scene.set_mesh(id, Some(path.clone())).ok();
            ctx.selection = Some(id);
            Ok(format!(
                "instantiated #{} mesh={path} verts={} half=({:.3},{:.3},{:.3})",
                ctx.scene.index_of(id),
                mesh.verts.len(),
                half.x,
                half.y,
                half.z
            ))
        },
    );
    // ---- P4e 前置：GPU 侧骨骼变换的**输入清单**探针 ----
    //
    // 它报告"如果现在上 GPU，缓冲里会有什么"：关节矩阵个数 × 16 个 f32、
    // 以及用**着色器算法**（不归一化）算出来的一个顶点位置。
    // GPU 接线之后，这条命令就是"CPU 与 GPU 应该一致"的对照真值。
    reg.add_command(
        "asset.skin_gpu_info",
        "报告 GPU 侧骨骼变换输入：asset.skin_gpu_info <路径> <t>",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let t: f32 = args
                .get(1)
                .ok_or_else(|| "缺时间参数 t".to_string())?
                .parse()
                .map_err(|_| "t 不是数字".to_string())?;
            let bytes = std::fs::read(&path[..]).map_err(|e| format!("读文件失败：{e}"))?;
            let (json, _bin) = crate::engine::skin::split_glb(&bytes)?;
            let Some((skin, positions)) = crate::engine::skin::parse_glb_skin(&bytes)? else {
                return Ok(format!("{path}: 没有骨骼"));
            };
            let channels = crate::engine::anim::parse_glb_animations(&bytes)?;
            let worlds = crate::engine::anim::sample_pose_worlds(&json, 0, &channels, t);
            let joints: Vec<usize> = json
                .get("skins")
                .and_then(|s| s.as_arr())
                .and_then(|s| s.first())
                .and_then(|s| s.get("joints"))
                .and_then(|j| j.as_arr())
                .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as usize).collect())
                .unwrap_or_default();
            // 骨骼矩阵 = 当前世界 × 绑定 inverseBind（与 `posed_positions` 同一公式）
            let mats: Vec<glam::Mat4> = joints
                .iter()
                .enumerate()
                .map(|(k, node)| {
                    worlds.get(*node).copied().unwrap_or(glam::Mat4::IDENTITY)
                        * skin.inverse_bind.get(k).copied().unwrap_or(glam::Mat4::IDENTITY)
                })
                .collect();
            let buf = crate::engine::skin::joint_matrix_buffer(&mats);
            let nw = crate::engine::skin::normalized_weights(&skin);
            let p0 = positions.first().copied().unwrap_or(glam::Vec3::ZERO);
            let q = crate::engine::skin::shader_reference_skin(
                p0,
                skin.joints.first().copied().unwrap_or([0; 4]),
                nw.first().copied().unwrap_or([0.0; 4]),
                &mats,
            );
            Ok(format!(
                "{path}: t={t:.2} joints={} buffer_floats={} v0=({:.3},{:.3},{:.3})",
                mats.len(),
                buf.len(),
                q.x,
                q.y,
                q.z
            ))
        },
    );
    // ---- P2b：旋转（绕 Y）----
    //
    // 与平移 gizmo 同一套做法：编辑器用鼠标走它，AI/脚本用命令走它，**同一个函数**。
    reg.add_command(
        "gizmo.rotate",
        "绕 Y 旋转选中实体：gizmo.rotate <ndc0x> <ndc0y> <ndc1x> <ndc1y>",
        |ctx, args| {
            let view = ctx.view.ok_or_else(|| "当前没有相机视角".to_string())?;
            if args.len() < 4 {
                return Err("参数不足：需要 4 个 NDC 分量".to_string());
            }
            let f = |i: usize| -> Result<f32, String> {
                args[i].parse::<f32>().map_err(|_| format!("第 {i} 个参数不是数字"))
            };
            let (x0, y0, x1, y1) = (f(0)?, f(1)?, f(2)?, f(3)?);
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let local = ctx
                .scene
                .local(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            let d = crate::engine::gizmo::rotate_delta_y(
                local.translation,
                &view,
                (x0, y0),
                (x1, y1),
            )
            .ok_or_else(|| "光标离物体太近或物体在相机背后".to_string())?;
            // 叠在**现有**旋转上（绕世界 Y 用左乘）：这样连续拖多次能累加
            let q = glam::Quat::from_rotation_y(d) * local.rotation;
            ctx.scene
                .set_local(
                    sel,
                    crate::engine::scene::Transform {
                        rotation: q,
                        ..local
                    },
                )
                .map_err(|_| "写回失败".to_string())?;
            Ok(format!(
                "rotated #{} by {:.1}° (yaw now {:.1}°)",
                ctx.scene.index_of(sel),
                d.to_degrees(),
                q.to_euler(glam::EulerRot::YXZ).0.to_degrees()
            ))
        },
    );
    reg.add_command(
        "gizmo.mode",
        "切换 gizmo 模式：gizmo.mode <translate|rotate>",
        |ctx, args| {
            let m = args
                .first()
                .and_then(|s| crate::engine::gizmo::Mode::parse(s))
                .ok_or_else(|| "缺模式或模式非法（translate/rotate）".to_string())?;
            ctx.gizmo_mode = m;
            Ok(format!("gizmo_mode={}", m.name()))
        },
    );
    reg.add_command(
        "gizmo.scale",
        "按轴缩放选中实体：gizmo.scale <X|Y|Z> <沿轴位移米数>",
        |ctx, args| {
            let axis = args
                .first()
                .and_then(|s| crate::engine::gizmo::Axis::parse(s))
                .ok_or_else(|| "缺轴或轴非法（X/Y/Z）".to_string())?;
            let delta: f32 = args
                .get(1)
                .ok_or_else(|| "缺位移".to_string())?
                .parse()
                .map_err(|_| "位移不是数字".to_string())?;
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let mut local = ctx
                .scene
                .local(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            let f = crate::engine::gizmo::scale_factor_from_drag(delta);
            // 等比缩放（三轴同倍）：`Scale` 模式下把手是"整体缩"，不做单轴拉伸。
            local.scale *= f;
            let sc = local.scale;
            ctx.scene
                .set_local(sel, local)
                .map_err(|_| "写回失败".to_string())?;
            Ok(format!(
                "scaled #{} {} by {:.3}x → ({:.3},{:.3},{:.3})",
                ctx.scene.index_of(sel),
                axis.name(),
                f,
                sc.x,
                sc.y,
                sc.z
            ))
        },
    );
    // ---- P3k：撤销 / 重做 ----
    //
    // 记录在 `run_command` 里统一做（命令表是编辑器与 AI 的唯一入口），
    // 所以这两条命令**不需要**自己记，只需把快照换回去。
    //
    // ⚠️ `apply_to` 是"清空重建" ⇒ 撤销之后**旧句柄全部失效**，选中项必须清掉。
    reg.add_command(
        "scene.undo",
        "撤销上一步场景改动：scene.undo",
        |ctx, _args| {
            let current = crate::engine::scene_io::to_toml(ctx.scene)
                .map_err(|e| format!("取快照失败：{e}"))?;
            let Some(prev) = ctx.undo.undo(current) else {
                return Ok("undo: 没有可撤销的步骤".to_string());
            };
            let n = crate::engine::scene_io::apply_to(ctx.scene, &prev)
                .map_err(|e| format!("恢复失败：{e}"))?;
            ctx.selection = None;
            Ok(format!(
                "undo → entities={n} 剩余可撤销={}",
                ctx.undo.past_len()
            ))
        },
    );
    reg.add_command(
        "scene.redo",
        "重做被撤销的一步：scene.redo",
        |ctx, _args| {
            let current = crate::engine::scene_io::to_toml(ctx.scene)
                .map_err(|e| format!("取快照失败：{e}"))?;
            let Some(next) = ctx.undo.redo(current) else {
                return Ok("redo: 没有可重做的步骤".to_string());
            };
            let n = crate::engine::scene_io::apply_to(ctx.scene, &next)
                .map_err(|e| format!("恢复失败：{e}"))?;
            ctx.selection = None;
            Ok(format!(
                "redo → entities={n} 可重做={}",
                ctx.undo.future_len()
            ))
        },
    );
    reg.add_command(
        "scene.undo_state",
        "报告撤销栈状态：scene.undo_state",
        |ctx, _args| {
            Ok(format!(
                "past={} future={} can_undo={} can_redo={}",
                ctx.undo.past_len(),
                ctx.undo.future_len(),
                ctx.undo.can_undo(),
                ctx.undo.can_redo()
            ))
        },
    );
    // ---- P4f：把某个时刻的骨骼姿势**烘成一件网格** ----
    //
    // 为什么要烘：渲染侧的道具管线吃的是**静态合并网格**（一条 draw call 画一堆件），
    // 没有逐顶点关节属性。GPU 侧骨骼变形要改整条 mesh 着色器（所有几何都依赖它），
    // 所以在动它之前，先用"把姿势烘进几何"得到**看得见的骨骼动画结果**。
    // 这条路是实在的：顶点由 CPU 侧同一套公式算（`anim::posed_positions`），
    // 上屏的确实是变形后的几何，不是占位盒。
    reg.add_command(
        "asset.pose_bake",
        "把骨骼姿势烘成可用资产：asset.pose_bake <路径> <t>",
        |ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?.to_string();
            let t: f32 = args
                .get(1)
                .ok_or_else(|| "缺时间参数 t".to_string())?
                .parse()
                .map_err(|_| "t 不是数字".to_string())?;
            ctx.bake_requests.push((path.clone(), t));
            Ok(format!("bake_requested {path} t={t:.3}"))
        },
    );
    // ---- P4g：**骨骼能力边界自查** ----
    //
    // 存在的理由：实现分了几步（解析 / 采样 / CPU 姿势 / 烘焙上屏 / GPU 逐顶点），
    // 每一步的完成度都能从"命令跑通了"被误解成"动画能播了"。
    // 这条命令把**能做什么、不能做什么**印成一行，谁读都不会高估。
    reg.add_command(
        "asset.skin_status",
        "报告骨骼动画能力边界：asset.skin_status <路径>",
        |_ctx, args| {
            let path = args.first().ok_or_else(|| "缺路径".to_string())?;
            let bytes = std::fs::read(&path[..]).map_err(|e| format!("读文件失败：{e}"))?;
            let (json, _bin) = crate::engine::skin::split_glb(&bytes)?;
            let Some((skin, positions)) = crate::engine::skin::parse_glb_skin(&bytes)? else {
                return Ok(format!("{path}: 无骨骼（静态几何）"));
            };
            let channels = crate::engine::anim::parse_glb_animations(&bytes).unwrap_or_default();
            let dur = channels
                .iter()
                .filter_map(|c| c.times.last().copied())
                .fold(0.0f32, f32::max);
            let worlds = crate::engine::anim::sample_pose_worlds(&json, 0, &channels, 0.0);
            let joints: Vec<usize> = json
                .get("skins")
                .and_then(|s| s.as_arr())
                .and_then(|s| s.first())
                .and_then(|s| s.get("joints"))
                .and_then(|j| j.as_arr())
                .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as usize).collect())
                .unwrap_or_default();
            // 真算一次姿势（不是只看字段）：能算出顶点就说明这条链在这件资产上可用
            let posed = crate::engine::anim::posed_positions(&skin, &joints, &positions, &worlds);
            Ok(format!(
                "{path}: joints={} verts={} posed={} channels={} dur={dur:.3}s | 解析=yes 采样=yes CPU姿势=yes 烘焙上屏=yes 连续播放=no GPU逐顶点=no | 依据: 合并网格重建一次约224ms(实测)",
                skin.joint_count(),
                skin.vertex_count(),
                posed.len(),
                channels.len()
            ))
        },
    );
    // `RV3D_DEBUG_NUDGE=<米>`：每帧把第 0 个实体沿 X 推一点。
    //
    // **它只用于"证明测量本身灵敏"**：编辑态自证要求"600 帧哈希全同"，
    // 而一个永远不变的测量也可能只是"根本没东西会变"——那就成了恒真判据。
    // 打开这个开关哈希必须变，否则说明自证通道坏了。
    reg.add_system(Phase::Update, "debug.nudge", |ctx| {
        let Some(step) = crate::syscfg::cfg("RV3D_DEBUG_NUDGE")
            .and_then(|s| s.parse::<f32>().ok())
        else {
            return;
        };
        let ids: Vec<_> = ctx.scene.iter_alive().map(|(id, _)| id).collect();
        if let Some(&id) = ids.first() {
            if let Ok(mut local) = ctx.scene.local(id) {
                local.translation.x += step;
                ctx.scene.set_local(id, local).ok();
            }
        }
    });

    // 可见性 / 网格绑定：这两个字段**也要能被 AI 改**，否则 P5 的"逐字段核对"
    // 根本覆盖不到它们（判据里造不出来 ⇒ 等于没测）。
    reg.add_command(
        "scene.visible",
        "设置选中实体可见性：scene.visible <0|1>",
        |ctx, args| {
            let on = match args.first().map(|s| &s[..]) {
                Some("1") | Some("on") | Some("true") => true,
                Some("0") | Some("off") | Some("false") => false,
                _ => return Err("参数应是 0 或 1".to_string()),
            };
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            ctx.scene
                .set_visible(sel, on)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            Ok(format!("visible #{} = {on}", ctx.scene.index_of(sel)))
        },
    );
    reg.add_command(
        "scene.mesh",
        "绑定/解绑网格资产：scene.mesh <路径|clear>",
        |ctx, args| {
            let arg = args.first().ok_or_else(|| "缺路径（或 clear）".to_string())?;
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let mesh = if *arg == "clear" {
                None
            } else {
                Some(arg.to_string())
            };
            ctx.scene
                .set_mesh(sel, mesh.clone())
                .map_err(|_| "选中实体句柄失效".to_string())?;
            Ok(match mesh {
                Some(m) => format!("mesh #{} = {m}", ctx.scene.index_of(sel)),
                None => format!("mesh #{} cleared", ctx.scene.index_of(sel)),
            })
        },
    );

    // 删除选中实体（**级联**：子实体一起走）。撤销栈在 `run_command` 里统一记录，
    // 所以误删能按 Backspace 回来 —— 这也是"能放心给 Del 键"的前提。
    reg.add_command(
        "scene.despawn",
        "删除选中实体（含子实体）：scene.despawn",
        |ctx, _args| {
            let sel = ctx.selection.ok_or_else(|| "没有选中实体".to_string())?;
            let idx = ctx.scene.index_of(sel);
            let before = ctx.scene.len();
            ctx.scene
                .despawn(sel)
                .map_err(|_| "选中实体句柄失效".to_string())?;
            let removed = before - ctx.scene.len();
            ctx.selection = None;
            Ok(format!("despawned #{idx}（连同子实体共 {removed} 个）"))
        },
    );

    // 自描述命令：编辑器面板与 AI 接口都靠它发现"这个引擎能做什么"。
    // 它必须真的把 help 与插件名打出来 —— 否则 help 字段就只是装饰。
    reg.add_command("plugin.list", "列出已注册插件与命令", |_ctx, _args| {
        Ok(String::from("SELF"))
    });
}

#[cfg(test)]
mod tests {
    /// 🔴 **删除要级联，且必须能被撤销回来**：
    /// 删上一级实体连子实体一起走；`scene.undo` 之后实体数回到删除前、字段也回来。
    ///
    /// 反向验证：把 `scene.despawn` 换成 `scene.visible 0`（只藏不删）⇒ 实体数不变，红。
    #[test]
    fn despawn_cascades_and_undo_restores() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let (root, child, other) = {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            let root = ctx.scene.spawn(None, crate::engine::scene::Transform::IDENTITY);
            let child = ctx.scene.spawn(Some(root), crate::engine::scene::Transform::IDENTITY);
            let other = ctx.scene.spawn(None, crate::engine::scene::Transform::IDENTITY);
            (root, child, other)
        };
        let _ = (child, other);
        assert_eq!(scene.len(), 3);

        // 删上一级：子要跟着走
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            ctx.selection = Some(root);
            let msg = reg
                .run_command("scene.despawn", &[], &mut ctx)
                .expect("删除应成功");
            assert!(msg.contains("共 2 个"), "应删掉上一级+子两个：{msg}");
            assert_eq!(ctx.selection, None, "删完必须清空选中（句柄已失效）");
        }
        assert_eq!(scene.len(), 1, "只剩无关的那一个");

        // 撤销：三个实体全回来
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            reg.run_command("scene.undo", &[], &mut ctx).expect("撤销");
        }
        assert_eq!(scene.len(), 3, "撤销后实体数必须回到 3");
    }

    /// 没有选中时删除必须**报错**，不许猜一个实体去删。
    #[test]
    fn despawn_without_selection_is_an_error() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        let mut undo = crate::engine::undo::UndoStack::new(64);
        scene.spawn(None, crate::engine::scene::Transform::IDENTITY);
        let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
        assert!(
            reg.run_command("scene.despawn", &[], &mut ctx).is_err(),
            "没选中必须报错"
        );
        assert_eq!(ctx.scene.len(), 1, "报错时不许动场景");
    }


    /// 🔴 **P5 的严格核对**：`spawn → set → nudge → scale → save → load_into` 之后，
    /// **每个实体的每个字段逐一相等**（平移 / 旋转 / 缩放 / 上一层 / 可见性 / 网格）。
    ///
    /// 为什么不比 TOML 文本：文本相等能证明"存盘读回一致"，但不能证明"字段都对"——
    /// 两者都丢了同一个字段时，文本照样相等。这里比的是**字段值**。
    ///
    /// 反向验证：把 `apply_to` 里搬网格那两行删掉 ⇒ 网格字段不等，红。
    #[test]
    fn ai_roundtrip_matches_every_field_of_every_entity() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let path = std::env::temp_dir().join("steel_front_p5_roundtrip.toml");
        let path_s = path.to_string_lossy().to_string();

        // 通过命令表做全套改动（与 AI 接口同一条路）
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            // 视角：`gizmo.rotate` 要它（先摆好相机，再让实体出现在相机前方）
            ctx.view = Some(crate::engine::gizmo::ViewInfo {
                eye: glam::Vec3::new(0.0, 0.0, 5.0),
                forward: glam::Vec3::new(0.0, 0.0, -1.0),
                right: glam::Vec3::X,
                up: glam::Vec3::Y,
                half_fov_y: std::f32::consts::FRAC_PI_4,
                aspect: 1.0,
            });
            reg.run_command("scene.spawn", &[], &mut ctx).expect("spawn");
            // 🔴 必须**真的转一下**：不转的话"旋转字段相等"是恒真的
            // （注入 `rotation: IDENTITY` 居然是绿的 —— 因为两边都是恒等）。
            reg.run_command("scene.set", &["0", "0", "0"], &mut ctx).expect("set0");
            reg.run_command("gizmo.rotate", &["0.2", "0", "0", "0.2"], &mut ctx)
                .expect("rotate");
            // 第二条：证明多实体的字段也逐一对比（不是只有一条时凑巧相等）
            reg.run_command("scene.spawn", &[], &mut ctx).expect("spawn2");
            reg.run_command("scene.set", &["1.5", "-2.25", "3"], &mut ctx)
                .expect("set");
            reg.run_command("scene.nudge", &["Z", "4"], &mut ctx)
                .expect("nudge");
            reg.run_command("gizmo.scale", &["X", "1.0"], &mut ctx)
                .expect("scale");
            // 🔴 这两个字段必须**在测试里真的被设过**，否则"逐字段相等"覆盖不到它们
            // （第一版就是没设，注入"不搬网格/不搬可见性"居然然都绿）。
            reg.run_command("scene.mesh", &["assets/props/barrel_metal.glb"], &mut ctx)
                .expect("mesh");
            reg.run_command("scene.visible", &["0"], &mut ctx)
                .expect("visible");
            reg.run_command("scene.save", &[&path_s], &mut ctx)
                .expect("save");
        }

        // 逐字段快照
        #[derive(Debug, PartialEq)]
        struct Snap {
            t: glam::Vec3,
            r: glam::Quat,
            s: glam::Vec3,
            visible: bool,
            mesh: Option<String>,
            parent: Option<usize>,
        }
        let snap = |sc: &Scene| -> Vec<Snap> {
            sc.iter_alive()
                .map(|(id, local)| Snap {
                    t: local.translation,
                    r: local.rotation,
                    s: local.scale,
                    visible: sc.is_visible(id),
                    mesh: sc.mesh_of(id),
                    parent: sc.parent(id).ok().flatten().map(|p| sc.index_of(p)),
                })
                .collect()
        };
        let before = snap(&scene);
        assert!(before.len() >= 2, "至少要有根与新建实体");

        // 读回到**同一个**场景（AI 的实际用法）
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            reg.run_command("scene.load_into", &[&path_s], &mut ctx)
                .expect("load_into");
        }
        let after = snap(&scene);
        assert_eq!(before.len(), after.len(), "实体数必须一致");
        // 🔴 四元数**不能要求逐位相等**：`from_toml` 读回时会 `normalize()`（刻意行为 ——
        // 场景文件可能被手工编辑，非单位四元数必须被拉回），而 normalize 会动最后一位
        // （实测 `0.70710677` → `0.7071068`）。这条是我用"注入 rotation: IDENTITY"做正对照时
        // 顺手发现的：判据一开始就在比较浮点位，只是恰好没转过东西所以没暴露。
        //
        // 所以分两层判：
        //   ① 数值层：平移/缩放/旋转都用 1e-6 容差（这才是"字段相等"的合理口径）；
        //   ② 结构层：上一层 / 可见性 / 网格必须**完全相等**（它们是离散值，没有精度问题）。
        for (i, (a, b)) in before.iter().zip(&after).enumerate() {
            assert!(
                (a.t - b.t).length() < 1e-6,
                "第 {i} 个实体的平移不等：{:?} vs {:?}",
                a.t,
                b.t
            );
            assert!(
                (a.s - b.s).length() < 1e-6,
                "第 {i} 个实体的缩放不等：{:?} vs {:?}",
                a.s,
                b.s
            );
            assert!(
                (a.r.normalize().to_array())
                    .iter()
                    .zip(b.r.normalize().to_array().iter())
                    .map(|(x, y)| (x - y).abs())
                    .fold(0.0f32, f32::max)
                    < 1e-6,
                "第 {i} 个实体的旋转不等：{:?} vs {:?}",
                a.r,
                b.r
            );
            assert_eq!(a.visible, b.visible, "第 {i} 个实体的可见性不等");
            assert_eq!(a.mesh, b.mesh, "第 {i} 个实体的网格不等");
            assert_eq!(a.parent, b.parent, "第 {i} 个实体的上一层不等");
        }
        std::fs::remove_file(&path).ok();
    }


    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    /// 🔴 **命令级的撤销链路**：改 → 撤销 → 场景回到原状（走命令表，与编辑器/AI 同一条路）。
    ///
    /// 反向验证：把 `run_command` 里的 `ctx.undo.record(before)` 删掉 ⇒ 撤销栈永远是空的，红。
    #[test]
    fn command_level_undo_restores_the_previous_scene() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        let id = scene.spawn(None, crate::engine::scene::Transform::IDENTITY);
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let before = crate::engine::scene_io::to_toml(&scene).unwrap();
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            ctx.selection = Some(id);
            reg.run_command("scene.set", &["3", "4", "5"], &mut ctx)
                .expect("设置应成功");
            assert_eq!(ctx.undo.past_len(), 1, "改一次应记一步");
        }
        assert_ne!(
            crate::engine::scene_io::to_toml(&scene).unwrap(),
            before,
            "改动应真的落到场景里"
        );
        {
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            reg.run_command("scene.undo", &[], &mut ctx).expect("撤销应成功");
        }
        assert_eq!(
            crate::engine::scene_io::to_toml(&scene).unwrap(),
            before,
            "撤销后必须回到改动前的逐字节状态"
        );
    }

    /// 🔴 **只读命令不许压栈**：否则用户按两次撤销却什么都没变。
    ///
    /// 反向验证：把 `after != before` 判断去掉（改成总是 record）⇒ 第一步就记了，红。
    #[test]
    fn read_only_commands_do_not_push_undo_steps() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        scene.spawn(None, crate::engine::scene::Transform::IDENTITY);
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
        reg.run_command("scene.count", &[], &mut ctx).expect("应成功");
        reg.run_command("plugin.list", &[], &mut ctx).expect("应成功");
        assert_eq!(ctx.undo.past_len(), 0, "只读命令不许压栈");
    }

    fn sys_a(_ctx: &mut PluginCtx<'_>) {
        ORDER.lock().unwrap().push("a");
    }
    fn sys_b(_ctx: &mut PluginCtx<'_>) {
        ORDER.lock().unwrap().push("b");
    }
    fn sys_c(_ctx: &mut PluginCtx<'_>) {
        ORDER.lock().unwrap().push("c");
    }

    /// 🔴 系统必须按**阶段顺序**执行，而不是按注册顺序。
    ///
    /// 注册顺序刻意打乱（PreRender 先注册、PreUpdate 最后注册），执行顺序仍须是
    /// `pre → update → post → pre_render`。反向验证：把 `run_frame` 改成按注册顺序遍历
    /// ⇒ 立刻红（顺序会变成 c,a,b）。
    #[test]
    fn plugin_systems_run_in_phase_order() {
        ORDER.lock().unwrap().clear();
        let mut reg = PluginRegistry::new();
        reg.add_system(Phase::PreRender, "c", sys_c);
        reg.add_system(Phase::Update, "a", sys_a);
        reg.add_system(Phase::PostUpdate, "b", sys_b);
        let mut scene = Scene::new();
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
        reg.run_frame(&mut ctx);
        assert_eq!(
            *ORDER.lock().unwrap(),
            vec!["a", "b", "c"],
            "阶段顺序必须是 update → post_update → pre_render（与注册顺序无关）"
        );
    }

    /// 🔴 命令表是**唯一入口**：按名字查得到、参数照传、错误要带出已注册清单。
    /// 反向验证：把 `run_command` 改成"只认第一条"⇒ 第二条查不到会红。
    #[test]
    fn plugin_command_lookup_is_by_name_and_reports_failures() {
        let mut reg = PluginRegistry::new();
        static HITS: AtomicU32 = AtomicU32::new(0);
        fn take(_ctx: &mut PluginCtx<'_>, args: &[&str]) -> Result<String, String> {
            HITS.fetch_add(1, Ordering::Relaxed);
            Ok(format!("n={}", args.len()))
        }
        reg.add_command("t.a", "第一个", take);
        reg.add_command("t.b", "第二个", |_c, _a| Ok("b".to_string()));
        let mut scene = Scene::new();
        let mut undo = crate::engine::undo::UndoStack::new(64);
        let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
        assert_eq!(reg.command_names(), vec!["t.a", "t.b"]);
        assert_eq!(reg.run_command("t.a", &["x", "y"], &mut ctx), Ok("n=2".to_string()));
        assert_eq!(HITS.load(Ordering::Relaxed), 1);
        let err = reg.run_command("nope", &[], &mut ctx).unwrap_err();
        assert!(err.contains("没有这条命令：nope"), "错误要可读：{err}");
        assert!(err.contains("t.a"), "错误里要列出已注册命令：{err}");
    }

    /// 🔴 **AI 命令链：spawn → drag → save → load，字段逐一相等**（目标里的验收口径）。
    ///
    /// 这条把"命令表"和"存盘"串起来测，而不是各自测各自的：真实用法就是一条命令接着
    /// 一条命令（中间可能隔好几帧），所以**选中项必须跨命令存活** ——
    /// 曾经写成"每帧都重置成玩家"，结果 `spawn` 之后 `drag` 把玩家拖走了、
    /// 新实体留在原点：**两条命令的回执都对，存盘文件里是错的**。
    ///
    /// 反向验证：把 `scene.spawn` 里 `ctx.selection = Some(id)` 删掉 ⇒ drag 落到玩家身上，
    /// 读回来的 `t` 是 0 而断言要 4.0，立刻红。
    #[test]
    fn ai_command_chain_spawn_drag_save_load_roundtrips() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        let mut scene = Scene::new();
        let path = std::env::temp_dir().join("steel_front_aipc_chain.toml");
        let path_str = path.to_string_lossy().to_string();
        {
            let mut undo = crate::engine::undo::UndoStack::new(64);
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            let r = reg.run_command("scene.spawn", &[], &mut ctx).unwrap();
            assert!(r.contains("entities=1"), "spawn 回执：{r}");
            assert!(ctx.selection.is_some(), "spawn 必须接管选中项");
            let r = reg
                .run_command("gizmo.drag", &["Z", "0", "4.0"], &mut ctx)
                .unwrap();
            assert!(r.contains("delta=4.0000"), "drag 回执：{r}");
            let r = reg
                .run_command("scene.save", &[&path_str], &mut ctx)
                .unwrap();
            assert!(r.contains("entities=1"), "save 回执：{r}");
        }
        // 读回并核对字段（用的是命令背后同一个函数）
        let text = std::fs::read_to_string(&path).unwrap();
        let (back, map) = crate::engine::scene_io::from_toml(&text).unwrap();
        let (_, handle) = map[0];
        let t = back.local(handle).unwrap().translation;
        assert!((t.z - 4.0).abs() < 1e-6, "读回的 z 应为 4.0，实际 {t:?}");
        assert_eq!(t.x, 0.0);
        assert_eq!(t.y, 0.0);
        let _ = std::fs::remove_file(&path);
    }

    /// 内置调试插件必须真的注册上（否则 `RV3D_PLUGIN_CMD` 那条链路是空的）。
    #[test]
    fn debug_plugin_registers_commands_and_system() {
        let mut reg = PluginRegistry::new();
        register_debug_plugin(&mut reg);
        assert_eq!(reg.plugin_names(), &["debug"]);
        let names = reg.command_names();
        for need in ["scene.count", "scene.spawn", "plugin.list"] {
            assert!(names.contains(&need), "缺少命令 {need}：{names:?}");
        }
        // 命令真的能改场景：spawn 之后实体数 +1
        let mut scene = Scene::new();
        {
            let mut undo = crate::engine::undo::UndoStack::new(64);
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            let _ = reg.run_command("scene.count", &[], &mut ctx).unwrap();
        }
        let before = scene.len();
        {
            let mut undo = crate::engine::undo::UndoStack::new(64);
            let mut ctx = PluginCtx::new(&mut scene, &mut undo, 0.0, 0.016);
            let _ = reg.run_command("scene.spawn", &[], &mut ctx).unwrap();
        }
        assert_eq!(scene.len(), before + 1, "scene.spawn 必须真的建实体");
    }
}
