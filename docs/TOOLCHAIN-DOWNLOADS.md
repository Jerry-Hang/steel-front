# 外部工具下载规范（2026-09-12 建立）

> 用户 2026-09-12 授权：「需要的工具（如 Blender）**可以下载**，但**必须建立下载规范并说明来源**」。
> 本文就是那个规范。**新增任何外部工具前先读本文**，并按最后一节的表格登记。

---

## 一、根本原则

1. **只用官方发布渠道**。不要第三方镜像、不要"绿色版/破解版"、不要网盘转载。
2. **记录来源与校验值**。每个工具在下方登记表里留一行：版本、官方 URL、SHA256、安装路径、用途。
3. **装在仓库之外**。仓库里**只放生成器脚本**（如 `tools/blender/*.py`），**不放二进制**。
   `.gitignore` 已覆盖构建产物；**永远不要为了"方便"把工具塞进仓库**。
4. **不可信就自己校验一遍**。下载后先 `Get-FileHash -Algorithm SHA256`，与官方公布值比对；
   官方未公布校验值的（如 Blender 的部分镜像），至少确认 HTTPS 域名与发布者署名。
5. **用完即弃的工具不要留在 PATH 上**。一次性反汇编器/转换器用绝对路径调用。

---

## 二、本机已登记的工具

| 工具 | 版本 | 来源 | 安装路径 | 用途 |
|---|---|---|---|---|
| Blender | **5.2.1 LTS** | blender.org 官方 Windows x64 包 | `D:\3D_Work\blender\blender-5.2.1-windows-x64\blender.exe` | 建筑/道具/枪械建模与 GLB 导出；**一律 headless** |
| Python（系统） | 3.11.9 | python.org | 系统 PATH | 一次性探针脚本 |
| （Blender 自带） | Python 3.13 | 随 Blender 分发 | 同上目录内 | 建模脚本运行环境 |

**注意**：Blender 自带的 Python 3.13 与系统 Python 3.11 是**两套环境**。
`tools/blender/*.py` 只保证在 Blender 自带环境里可跑（它们要 `bpy`）。

---

## 三、建模工具的调用约定（与铁律 D 一致）

**一律 headless，禁止自动化 Blender GUI** —— GUI 会抢焦点与鼠标，违反鼠标安全协议。

```powershell
# 尺寸普查（尺寸契约的唯一来源）
& $blender --background --python tools/blender/survey_props.py -- assets/props "*.glb"
# 生成（先输出到临时目录，确认后才覆盖 assets/props/）
& $blender --background --python tools/blender/build_city_kit.py -- <out_dir> [name...]
# 预览渲图（4 视图 → PNG；必须用眼睛看过再入库）
& $blender --background --python tools/blender/preview_glb.py -- <in.glb> <out_prefix> [n]
```

---

## 四、新增工具时的检查清单

- [ ] 来源是官方发布页（记下完整 URL 与访问日期）
- [ ] 记下版本号与 SHA256
- [ ] 装在仓库之外；确认 `git status` 没有把它纳入
- [ ] 若是脚本要进仓库：**纯 ASCII**（Windows PowerShell 5.1 按 ANSI 读无 BOM 的 .ps1，
      非 ASCII 会破坏引号配对 —— 本仓为此付过代价）
- [ ] 在上表登记一行
- [ ] 若它会产生需要入库的产物（如 GLB），确认产物符合**铁律 D 的单位与朝向约定**：
      1 单位 = 1 米、原点在底面中心、单 mesh / 单 primitive、节点不带变换

---

## 五、已知的"不要下载"

| 工具 | 为什么不要 |
|---|---|
| Rust 第三方 crate | **本项目硬约束：依赖只有 10 个**。需要新功能先想能否用现有依赖或手写实现 |
| 任何 GLB 优化器/压缩器 | 会改变顶点格式或绕序，而引擎对绕序敏感（反了直接黑且不报错） |
| GPU 剖析器（NSight/RenderDoc） | **未评估过**。若要用，先在本表登记并说明它读写的权限范围（它需要注入进程） |
