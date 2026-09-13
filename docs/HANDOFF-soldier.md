# 🔖 给"压缩后的我"的恢复note —— 士兵 GLB 接入

> **如果你是在上下文被压缩之后读到这行的：** 你正在做**把士兵 GLB 接进 NPC 渲染**这件事。
> 下面是你需要的全部状态，不用去猜。

## 已完成（都已提交推送，HEAD 会变，看 git log 找 `feat(soldier)`）

| 产物 | 说明 |
|---|---|
| `tools/blender/build_soldier.py` | 生成器，**四轮预览迭代**过 |
| `assets/soldier/soldier.glb` | **1082 顶点 / 540 三角形，1.840 m 高，min_z=0**（原点在底面中心 ✓） |
| `assets/soldier/` | **故意不放 `assets/props/`** —— 那里会被 `PropSet::load_dir` 扫到并当建筑加载 |

**模型已用眼睛看过四轮**（`build/_soldier/soldier_{0..3}_*.png`，该目录已 gitignore）：
有盔（喇叭形帽檐）、有脸（含护目镜条）、双手在枪上、腿分开、靴朝前。
**头/盔是六棱柱**（盒子颅骨在 3-5m 最像机械）。

## 待做：接入

**卡在哪**：`props.rs` 在**加载时把变换烘进顶点**（`merge_binned`），
且 `PROP_INSTANCE_INDEX` 是**单实例槽位** ⇒ **道具不能动态实例化**。
而 NPC 位置**每帧都变** ⇒ 255 人 × 1082 顶点 = 27 万顶点/帧重写，**不可行**。

**所以要动的是实例化路径本身。** 上手前必须先读清这三处（**它们必须同源**，铁律 B 的"静默越界读"）：

1. `renderer.rs` 的槽位常量：`NPC_SLOT_BASE` / `NPC_CYL_SLOT_BASE` / `NPC_SPH_SLOT_BASE`
2. `build.rs` 的 `NPC_INSTANCE_BASE`
3. **mesh 着色器里"每个实例生成什么几何"那段** —— 这是我还没读的关键处

**推荐的落地形态**：**近距用 GLB、远距保留 18 段箱体**（最省，且一个距离阈值就能回退）。

**必守纪律**：
- **改前先跑冒烟**确认基线 `VUID=0`；
- 改后**必须**跑 `scripts/run_smoke_pm.ps1`（判据 `vuid==0 and panics==0 and killed>=1`）；
- **双模式验证**：第一人称 + `RV3D_INSPECT=1`；
- 绕序错了会**静默全黑**，不要靠手推绕序。

## 今晚已经付过代价的三条（别重犯）

1. **盒子 `scale` 是全尺寸**（段表注释写着「圆柱=半径；盒=宽/高/厚」）—— 我按"半宽"改了三轮，全错、全回退；
2. **给"人"烘 AO 的基准是身高（1.79）不是建筑层高（3.15）** —— 用错了裤腿会比上衣亮；
3. **"看起来变好了"不等于"数值变对了"** —— 观感改善可能来自别的因素。

## 复现命令（**路径必须绝对或正斜杠**，否则 Blender 会写到 `C:\build\`）

```powershell
$bl = "D:\3D_Work\blender\blender-5.2.1-windows-x64\blender.exe"
& $bl --background --python tools/blender/build_soldier.py -- "D:/Rust/steel-front/build/_soldier" soldier
& $bl --background --python tools/blender/preview_glb.py -- "D:/Rust/steel-front/build/_soldier/soldier.glb" "D:/Rust/steel-front/build/_soldier/soldier" 4
& $bl --background --python tools/blender/survey_props.py -- assets/soldier "*.glb"
```

