//! 骨骼动画采样（引擎化 P4b，见 `docs/ENGINE-PLAN.md`）。
//!
//! glTF 的动画模型：`animations[].channels[]` 把某个 `node` 的某个属性
//! （translation / rotation / scale）绑到一个 `sampler`，sampler 含
//! `input`（时间关键帧）、`output`（值）、`interpolation`（STEP / LINEAR / CUBICSPLINE）。
//!
//! 三种插值的**语义差别很大**，而且写错时的表现都是"动画看起来不太对"，很难定位：
//! - **STEP**：取**前一个**关键帧的值，整段不插值（开关式动画，如可见性、机械步进）；
//! - **LINEAR**：线性插值；旋转必须用 **slerp**（四元数线性插值在 180° 附近会塌陷）；
//! - **CUBICSPLINE**：Hermite样条，`output` 每个关键帧存**三个** vec4：入切线、值、出切线。
//!
//! 时间轴外的采样一律**夹到端点**（glTF 规范要求，也是播放器的通行做法）——
//! 超出范围的 `t` 返回第一/最后一个关键帧，而不是外推（外推会让骨骼飞出模型）。

use glam::{Quat, Vec4};

/// 插值方式（与 glTF `sampler.interpolation` 一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Step,
    Linear,
    CubicSpline,
}

impl Interp {
    pub fn parse(s: &str) -> Interp {
        match s {
            "STEP" => Interp::Step,
            "CUBICSPLINE" => Interp::CubicSpline,
            // glTF 默认是 LINEAR（字段可省略）
            _ => Interp::Linear,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Interp::Step => "STEP",
            Interp::Linear => "LINEAR",
            Interp::CubicSpline => "CUBICSPLINE",
        }
    }
}

/// 找 `t` 落在哪一段：返回 `(i, u)`，`i` 是左侧关键帧下标，`u ∈ [0,1]` 是段内比例。
///
/// **夹取语义**：`t <= times[0]` ⇒ `(0, 0)`；`t >= times[last]` ⇒ `(last-1, 1)`。
pub fn locate(times: &[f32], t: f32) -> Option<(usize, f32)> {
    if times.len() < 2 {
        return None;
    }
    if t <= times[0] {
        return Some((0, 0.0));
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return Some((last - 1, 1.0));
    }
    // 线性查找：骨骼动画的关键帧通常只有几十个，二分不划算也更容易写错边界
    for i in 0..last {
        if t >= times[i] && t <= times[i + 1] {
            let span = times[i + 1] - times[i];
            let u = if span > 0.0 { (t - times[i]) / span } else { 0.0 };
            return Some((i, u));
        }
    }
    Some((last - 1, 1.0))
}

/// Hermite基函数（CUBICSPLINE 用）。`t` 是段内比例。
fn hermite(t: f32) -> (f32, f32, f32, f32) {
    let t2 = t * t;
    let t3 = t2 * t;
    (
        2.0 * t3 - 3.0 * t2 + 1.0,  // h00
        t3 - 2.0 * t2 + t,         // h10
        -2.0 * t3 + 3.0 * t2,      // h01
        t3 - t2,                   // h11
    )
}

/// 采样一条**标量/向量**轨道（translation / scale）。
///
/// `values` 是**逐个关键帧**一个 vec4。CUBICSPLINE 请用 [`sample_cubic`]。
pub fn sample_vec(times: &[f32], values: &[Vec4], interp: Interp, t: f32) -> Vec4 {
    if values.is_empty() {
        return Vec4::ZERO;
    }
    if values.len() == 1 {
        return values[0];
    }
    // 🔴 端点必须显式夹取：`locate` 为了返回"段"会把 t>=末帧落到 (last-1, 1.0)，
    // 而 **STEP** 在这一段取的是左端值 ⇒ 末帧上仍然返回倒数第二帧的值（错）。
    // 这条是判据 `anim_step_holds_the_previous_keyframe` 抓出来的。
    if let (Some(&first), Some(&last)) = (times.first(), times.last()) {
        if t <= first {
            return values[0];
        }
        if t >= last {
            return values[values.len().min(times.len()) - 1];
        }
    }
    let Some((i, u)) = locate(times, t) else {
        return values[0];
    };
    match interp {
        Interp::Step => values[i],
        Interp::Linear => values[i].lerp(values[i + 1], u),
        // CUBICSPLINE 走另一条函数；这里退化成线性而不是 panic（调用方给错了也只损失平滑度）
        Interp::CubicSpline => values[i].lerp(values[i + 1], u),
    }
}

/// 采样 CUBICSPLINE 轨道。`values` 每帧 **3 个 vec4**：`[入切线, 值, 出切线]`。
pub fn sample_cubic(times: &[f32], values: &[Vec4], t: f32) -> Vec4 {
    let n = times.len();
    if n == 0 || values.len() < n * 3 {
        return Vec4::ZERO;
    }
    if n == 1 {
        return values[1];
    }
    let (i, u) = match locate(times, t) {
        Some(v) => v,
        None => return values[1],
    };
    let dt = times[i + 1] - times[i];
    let v0 = values[i * 3 + 1];
    let out_tan = values[i * 3 + 2];
    let v1 = values[(i + 1) * 3 + 1];
    let in_tan = values[(i + 1) * 3];
    let (h00, h10, h01, h11) = hermite(u);
    v0 * h00 + out_tan * (h10 * dt) + v1 * h01 + in_tan * (h11 * dt)
}

/// 采样**旋转**轨道：四元数必须用 slerp，线性插值在 180° 附近会把长度拉短、姿态发飘。
///
/// `values` 是逐关键帧的四元数（xyzw 打包在 vec4 里）。
pub fn sample_quat(times: &[f32], values: &[Vec4], interp: Interp, t: f32) -> Quat {
    if values.is_empty() {
        return Quat::IDENTITY;
    }
    if values.len() == 1 {
        return to_quat(values[0]);
    }
    if let (Some(&first), Some(&last)) = (times.first(), times.last()) {
        if t <= first {
            return to_quat(values[0]);
        }
        if t >= last {
            return to_quat(values[values.len().min(times.len()) - 1]);
        }
    }
    let Some((i, u)) = locate(times, t) else {
        return to_quat(values[0]);
    };
    let a = to_quat(values[i]);
    let b = to_quat(values[i + 1]);
    match interp {
        Interp::Step => a,
        Interp::Linear => a.slerp(b, u),
        // 旋转的 CUBICSPLINE 在 glTF 里也是四元数Hermite + 归一化；本仓暂按 slerp 处理，
        // 差别只在"带切线的手感"，等真有带切线的资产再补（现在没有可验证的数据）。
        Interp::CubicSpline => a.slerp(b, u),
    }
}

fn to_quat(v: Vec4) -> Quat {
    let q = Quat::from_xyzw(v.x, v.y, v.z, v.w);
    if q.length_squared() < 1e-12 {
        Quat::IDENTITY
    } else {
        q.normalize()
    }
}

/// 一条动画通道（node + 属性 + 采样数据）。
#[derive(Debug, Clone)]
pub struct Channel {
    pub node: usize,
    pub path: String,
    pub interp: Interp,
    pub times: Vec<f32>,
    /// CUBICSPLINE 时是 `times.len() * 3` 个；其余是 `times.len()` 个。
    pub values: Vec<Vec4>,
}

impl Channel {
    /// 采样这个通道在 `t` 时刻的值（rotation 走 slerp）。
    pub fn sample(&self, t: f32) -> Vec4 {
        if self.path == "rotation" {
            let q = sample_quat(&self.times, &self.values, self.interp, t);
            Vec4::new(q.x, q.y, q.z, q.w)
        } else if self.interp == Interp::CubicSpline {
            sample_cubic(&self.times, &self.values, t)
        } else {
            sample_vec(&self.times, &self.values, self.interp, t)
        }
    }

    /// 时长（最后一个关键帧的时间）。
    pub fn duration(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }
}

/// 从 GLB 读全部动画通道。没有 `animations` 时返回空 vec（**不是错误**）。
pub fn parse_glb_animations(bytes: &[u8]) -> Result<Vec<Channel>, String> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err("非 GLB 文件（缺少 glTF magic）".into());
    }
    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_end = 20usize
        .checked_add(json_len)
        .filter(|&e| e <= bytes.len())
        .ok_or_else(|| format!("GLB JSON 块长度越界：头里写 {json_len} 字节"))?;
    let json_str = std::str::from_utf8(&bytes[20..json_end]).map_err(|_| "GLB JSON 非 UTF-8")?;
    let json =
        crate::llm_cmd::parse_json_fn(json_str).map_err(|e| format!("GLB JSON 解析失败: {e}"))?;
    let bin: &[u8] = if json_end + 8 <= bytes.len() {
        let blen = u32::from_le_bytes([
            bytes[json_end],
            bytes[json_end + 1],
            bytes[json_end + 2],
            bytes[json_end + 3],
        ]) as usize;
        &bytes[json_end + 8..(json_end + 8 + blen).min(bytes.len())]
    } else {
        &[]
    };
    let mut out = Vec::new();
    let Some(anims) = json.get("animations").and_then(|a| a.as_arr()) else {
        return Ok(out);
    };
    for anim in anims {
        let samples = anim.get("samplers").and_then(|s| s.as_arr());
        let Some(channels) = anim.get("channels").and_then(|c| c.as_arr()) else {
            continue;
        };
        for ch in channels {
            let Some(target) = ch.get("target") else { continue };
            let node = target.get("node").and_then(|n| n.as_f64()).unwrap_or(0.0) as usize;
            let path = target
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("translation")
                .to_string();
            let Some(si) = ch.get("sampler").and_then(|s| s.as_f64()) else {
                continue;
            };
            let Some(smp) = samples.and_then(|s| s.get(si as usize)) else {
                continue;
            };
            let Some(in_acc) = smp.get("input").and_then(|v| v.as_f64()) else {
                continue;
            };
            let Some(out_acc) = smp.get("output").and_then(|v| v.as_f64()) else {
                continue;
            };
            let interp = Interp::parse(smp.get("interpolation").and_then(|v| v.as_str()).unwrap_or("LINEAR"));
            let times = crate::engine::skin::read_acc_f32(&json, bin, in_acc as usize, 1)?;
            // rotation / translation / scale 都是 vec4（rotation 是四元数；translation/scale 第 4 分量无用）
            let raw = crate::engine::skin::read_acc_f32(&json, bin, out_acc as usize, 4)?;
            let values: Vec<Vec4> = raw
                .chunks_exact(4)
                .map(|c| Vec4::new(c[0], c[1], c[2], c[3]))
                .collect();
            out.push(Channel {
                node,
                path,
                interp,
                times,
                values,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// 🔴 **解析器判据**：通道的 node / path / 插值 / 时长必须从 GLB 里正确读出，
    /// 且在 t=0.5 采到中点（手算 1.0）。
    ///
    /// 光测采样数学不够：accessor 偏移读错时会静默拿到错误数据（本仓教训 21 的同形），
    /// 所以"读"也要有一条会红的判据。
    ///
    /// 反向验证：把 `smp.get("input")` 的 accessor 换成一个越界下标 ⇒ 解析报错，红。
    #[test]
    fn glb_animation_channel_parses_and_samples_midpoint() {
        // 最小 GLB：两条时间 [0,1] + 两个 vec4 值 (0,0,0,0) / (2,0,0,0)
        let mut bin: Vec<u8> = Vec::new();
        bin.extend_from_slice(&0.0f32.to_le_bytes());
        bin.extend_from_slice(&1.0f32.to_le_bytes());
        for v in [[0.0f32, 0.0, 0.0, 0.0], [2.0, 0.0, 0.0, 0.0]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        let json = r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"byteLength": 40}],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 8},
    {"buffer": 0, "byteOffset": 8, "byteLength": 32}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR"},
    {"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC4"}
  ],
  "nodes": [{"translation": [0, 0, 0]}],
  "animations": [{
    "samplers": [{"input": 0, "output": 1, "interpolation": "LINEAR"}],
    "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]
  }],
  "scenes": [{"nodes": [0]}]
}"#;
        let mut js = json.as_bytes().to_vec();
        while js.len() % 4 != 0 {
            js.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + js.len() + 8 + bin.len();
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(js.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&js);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);

        let ch = parse_glb_animations(&glb).expect("解析");
        assert_eq!(ch.len(), 1, "应有一条通道");
        let c = &ch[0];
        assert_eq!(c.node, 0);
        assert_eq!(c.path, "translation");
        assert_eq!(c.interp, Interp::Linear);
        assert!((c.duration() - 1.0).abs() < 1e-6, "时长应为 1.0");
        assert!((c.sample(0.5).x - 1.0).abs() < 1e-5, "中点应为 1.0，实际 {:?}", c.sample(0.5));
        assert!((c.sample(0.0).x).abs() < 1e-6);
        assert!((c.sample(1.0).x - 2.0).abs() < 1e-6);
    }

    /// 🔴 LINEAR：中点必须是两端的中值。手算：(0,0,0) → (2,0,0) 在 t=0.5 得 (1,0,0)。
    ///
    /// 反向验证：把 `lerp` 的 `u` 写成常数 0 ⇒ 中点仍返回左端，红。
    #[test]
    fn anim_linear_interpolates_at_midpoint() {
        let times = [0.0, 1.0];
        let vals = [Vec4::ZERO, Vec4::new(2.0, 0.0, 0.0, 0.0)];
        let v = sample_vec(&times, &vals, Interp::Linear, 0.5);
        assert!((v.x - 1.0).abs() < 1e-6, "中点该是 1.0，实际 {v:?}");
    }

    /// 🔴 STEP：**取前一个关键帧**，不插值。t=0.99 仍必须是 0，t=1.0 才跳到 2。
    ///
    /// 反向验证：把 `Interp::Step => values[i]` 改成 `values[i+1]` ⇒ 0.99 时返回 2，红。
    #[test]
    fn anim_step_holds_the_previous_keyframe() {
        let times = [0.0, 1.0];
        let vals = [Vec4::ZERO, Vec4::new(2.0, 0.0, 0.0, 0.0)];
        let a = sample_vec(&times, &vals, Interp::Step, 0.99);
        let b = sample_vec(&times, &vals, Interp::Step, 1.0);
        assert_eq!(a.x, 0.0, "STEP 在段内必须保持左端值");
        assert_eq!(b.x, 2.0, "到关键帧才跳变");
    }

    /// 时间轴外必须**夹到端点**，不许外推。
    #[test]
    fn anim_sampling_clamps_outside_the_range() {
        let times = [1.0, 2.0];
        let vals = [Vec4::new(5.0, 0.0, 0.0, 0.0), Vec4::new(7.0, 0.0, 0.0, 0.0)];
        assert_eq!(sample_vec(&times, &vals, Interp::Linear, -10.0).x, 5.0);
        assert_eq!(sample_vec(&times, &vals, Interp::Linear, 99.0).x, 7.0);
    }

    /// 🔴 CUBICSPLINE 的**手算值**：v0=v1=0、出切线=1、入切线=0、dt=1，
    /// 在 u=0.5 处 `h10 = 0.125` ⇒ 值 = 0.125。
    ///
    /// 反向验证：把 `h10 * dt` 写成 `h10`（漏乘 dt）⇒ dt=2 时差一倍，红（见下一条）。
    #[test]
    fn anim_cubic_matches_hand_computed_hermite() {
        let times = [0.0, 1.0];
        // 每帧 3 个：入切线 / 值 / 出切线
        let vals = [
            Vec4::ZERO,                      // 帧0 入切线
            Vec4::ZERO,                      // 帧0 值
            Vec4::new(1.0, 0.0, 0.0, 0.0),   // 帧0 出切线
            Vec4::ZERO,                      // 帧1 入切线
            Vec4::ZERO,                      // 帧1 值
            Vec4::new(9.0, 0.0, 0.0, 0.0),   // 帧1 出切线（不参与第一段）
        ];
        let v = sample_cubic(&times, &vals, 0.5);
        assert!((v.x - 0.125).abs() < 1e-5, "手算该是 0.125，实际 {v:?}");
    }

    /// 切线必须**乘以段长**：同样的切线、dt 翻倍，则位移翻倍。
    /// 这条专门抓"漏乘 dt"——那是最常见的一处实现错误，而且只在段长 ≠ 1 时暴露。
    #[test]
    fn anim_cubic_scales_tangents_by_segment_length() {
        let vals = [
            Vec4::ZERO,
            Vec4::ZERO,
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::ZERO,
            Vec4::ZERO,
            Vec4::ZERO,
        ];
        let a = sample_cubic(&[0.0, 1.0], &vals, 0.5).x;
        let b = sample_cubic(&[0.0, 2.0], &vals, 1.0).x; // u=0.5 但 dt=2
        assert!((b - a * 2.0).abs() < 1e-5, "dt 翻倍，位移必须翻倍：{a} vs {b}");
    }

    /// 🔴 旋转必须 **slerp**，不能是"线性插值再归一化"（nlerp）。
    ///
    /// ⚠️ 第一版判据用 180° 夹具，**方向不唯一**：slerp 走了 −90°，而期望写的是 +90° ——
    /// 两者是不同姿态但都合法（测地线不唯一），判据本身过度指定了。
    /// 换成 120° 之后，slerp 在 u=0.25 处角位移**恰为 30°**（w = cos15° = 0.96593），
    /// 而 nlerp 给 0.97063 —— 差 0.0047，用 1e-3 的容差能可靠区分。
    ///
    /// 反向验证：把 `a.slerp(b, u)` 换成 `a.lerp(b, u).normalize()` ⇒ 红。
    #[test]
    fn anim_rotation_uses_slerp_not_lerp() {
        let q0 = Quat::IDENTITY;
        let q1 = Quat::from_rotation_y(PI * 2.0 / 3.0); // 120°
        let times = [0.0, 1.0];
        let vals = [
            Vec4::new(q0.x, q0.y, q0.z, q0.w),
            Vec4::new(q1.x, q1.y, q1.z, q1.w),
        ];
        let mid = sample_quat(&times, &vals, Interp::Linear, 0.25);
        let w = mid.w.abs();
        assert!(
            (w - 15.0f32.to_radians().cos()).abs() < 1e-3,
            "slerp 在 u=0.25 处该转 30°（w≈0.96593），实际 w={w}（nlerp 会给 0.97063）"
        );
    }
}
