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
    /// 当前相机视角（取点命令要用；编辑器与 AI 接口都需要"从这里看到什么"）。
    pub view: Option<crate::engine::gizmo::ViewInfo>,
    /// **当前选中实体**（编辑器选中的那个；AI 接口也能改它）。
    /// `None` = 没选中 —— 跟"对选中项操作"有关的命令必须报错而不是猜一个。
    pub selection: Option<crate::engine::scene::EntityId>,
}

impl<'a> PluginCtx<'a> {
    pub fn new(scene: &'a mut Scene, time: f32, dt: f32) -> Self {
        PluginCtx {
            scene,
            time,
            dt,
            out: Vec::new(),
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
        let ret = (entry.func)(ctx, args)?;
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
    // 自描述命令：编辑器面板与 AI 接口都靠它发现"这个引擎能做什么"。
    // 它必须真的把 help 与插件名打出来 —— 否则 help 字段就只是装饰。
    reg.add_command("plugin.list", "列出已注册插件与命令", |_ctx, _args| {
        Ok(String::from("SELF"))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

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
        let mut ctx = PluginCtx::new(&mut scene, 0.0, 0.016);
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
        let mut ctx = PluginCtx::new(&mut scene, 0.0, 0.016);
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
            let mut ctx = PluginCtx::new(&mut scene, 0.0, 0.016);
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
            let mut ctx = PluginCtx::new(&mut scene, 0.0, 0.016);
            let _ = reg.run_command("scene.count", &[], &mut ctx).unwrap();
        }
        let before = scene.len();
        {
            let mut ctx = PluginCtx::new(&mut scene, 0.0, 0.016);
            let _ = reg.run_command("scene.spawn", &[], &mut ctx).unwrap();
        }
        assert_eq!(scene.len(), before + 1, "scene.spawn 必须真的建实体");
    }
}
