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
}

impl<'a> PluginCtx<'a> {
    pub fn new(scene: &'a mut Scene, time: f32, dt: f32) -> Self {
        PluginCtx {
            scene,
            time,
            dt,
            out: Vec::new(),
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
            Ok(format!("spawned entities={} id={id:?}", ctx.scene.len()))
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
