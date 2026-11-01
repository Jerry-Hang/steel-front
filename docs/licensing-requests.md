# 授权记录（对外许可请求的处置留档）

> 这份文件回答一个问题：**我们对外给过什么许可、范围多大、有没有改变仓库的整体许可。**
> 目的很简单：将来任何人（包括接手项目的人）翻仓库就能看到边界，不会被误读成“整仓换了许可”。

仓库整体许可**始终**是：

- 代码：**AGPL-3.0-or-later**（`LICENSE`）；
- 附加许可：小团队可闭源使用（`LICENSE-EXCEPTION.md`，按 AGPL §7 的 additional permission）；
- 商业授权：大公司另行签约（`LICENSE-COMMERCIAL.md`）；
- 第三方素材：见 `THIRD-PARTY-NOTICES.md`（CC0 为主，枪模为 CC BY）。

**任何单次授权都不改变上面这四条。**

---

## 1. XERJ Corpus Hub — 单个提交快照的 MIT 授权（2026-11-02）

### 请求方

| 项 | 内容 |
|---|---|
| 项目 | XERJ（AI 代码/文档检索 + MCP server） |
| 组织 | `xerj-org`，仓库 `github.com/xerj-org/xerj`（Apache-2.0，公开） |
| 站点 | `xerj.org` |
| 请求人 | Ivan N（founders@xerj.org，经 `bounce.xerj.org` 邮件列表转发） |
| 用途 | 把指定提交收录进 **Corpus Hub**，供 AI 代理检索、引用 |

### 授权内容

- **对象**：仅提交 `b050ec9c2683bdeb3812e7cc780919f676f68aa1`（`feat(render): LOD 做成三档`）的**那一个快照**；
- **方式**：对该快照中的 Rust 源码，允许按 **MIT** 条款使用（便于其语料库与 Apache-2.0/MIT 生态兼容）；
- **素材**：该提交里 4 个 `*_lod2.glb` 由 **CC0** 源生成，本身不含额外限制；
- **条件**：必须署名 **Evernight / Jerry-Hang/steel-front**，并回链 `https://github.com/Jerry-Hang/steel-front`。

### 边界（重要）

1. **不覆盖后续提交**，也不覆盖仓库其余任何部分；
2. **不构成概括授权**：将来其他提交如需收录，逐次单独确认；
3. 本授权**不改变**仓库整体的 AGPL-3.0-or-later + 小团队例外 + 商业授权结构；
4. 若对方把该快照**改写/演绎**后再分发，其演绎部分不因此获得本仓库其他代码的许可。

### 备注

- 对方最初的邮件写“5 files / ~280 lines”，与实际不符：该提交实为 **10 个文件、191 行新增 / 113 行删除**（含 4 个二进制 GLB）。已在回信中说明，并建议其**只索引 Rust 源码**（`src/engine/props.rs`、`renderer.rs`、`renderer/{device,parts,record}.rs`），`docs/PROGRESS.md` 是中文开发日志，对语料无价值。
- 之所以同意：这段代码是 LOD 框架（约 190 行），单独看**不构成**本项目的商业资产；换来的是一份带署名的公开曝光，且范围被钉死在单个快照上。

---

## 模板（将来再有人来问，照这个写）

```
对象：<提交 hash / 文件清单>
方式：<AGPL 收录 / 快照 MIT / 拒绝>
条件：<署名 + 回链>
边界：不覆盖后续提交；不构成概括授权；不改变仓库整体许可
```
