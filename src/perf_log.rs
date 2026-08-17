//! 性能日志系统（2026-08-16）：每次启动创建一份独立性能日志（logs/perf_*.log），
//! 记录硬件信息 + 运行期帧率/阶段耗时采样 + 退出汇总，便于后续调试与性能回归。
use std::io::Write;
use std::path::PathBuf;

pub struct PerfLog {
    file: std::fs::File,
    path: PathBuf,
    start: std::time::Instant,
    frames: u64,
    fps_sum: f64,
    fps_min: f64,
    fps_max: f64,
    last_sample: std::time::Instant,
}

impl PerfLog {
    /// 创建性能日志文件（logs/perf_YYYYMMDD_HHMMSS.log），写入头部信息
    pub fn create(header: &str) -> Option<Self> {
        let dir = PathBuf::from("logs");
        let _ = std::fs::create_dir_all(&dir);
        let ts = chrono_like_timestamp();
        let path = dir.join(format!("perf_{}.log", ts));
        let mut file = std::fs::File::create(&path).ok()?;
        let _ = writeln!(file, "==== 钢铁前线 性能日志 =====");
        let _ = writeln!(file, "{}", header);
        let _ = writeln!(file, "----------------------------------");
        let _ = writeln!(file, "时间(s)\tfps\tframe_us\tcull\tterrain\twait\tacquire\trecord\tsubmit\tpresent\tnear");
        Some(Self {
            file,
            path,
            start: std::time::Instant::now(),
            frames: 0,
            fps_sum: 0.0,
            fps_min: f64::MAX,
            fps_max: 0.0,
            last_sample: std::time::Instant::now(),
        })
    }

    /// 每帧调用（内部 1s 采样一次写一行）；snap 为渲染阶段耗时
    pub fn frame(&mut self, fps: f64, near: u32, snap: &crate::engine::renderer::PerfSnapshot) {
        self.frames += 1;
        self.fps_sum += fps;
        self.fps_min = self.fps_min.min(fps);
        self.fps_max = self.fps_max.max(fps);
        if self.last_sample.elapsed().as_secs_f32() >= 1.0 {
            let t = self.start.elapsed().as_secs_f32();
            let _ = writeln!(
                self.file,
                "{:.1}\t{:.1}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                t, fps, snap.frame_us, snap.cull_us, snap.terrain_us, snap.wait_fence_us,
                snap.acquire_us, snap.record_us, snap.submit_us, snap.present_us, near
            );
            let _ = self.file.flush();
            self.last_sample = std::time::Instant::now();
        }
    }

    /// 退出时写汇总
    pub fn finish(&mut self) {
        let dur = self.start.elapsed();
        let avg = if self.frames > 0 { self.fps_sum / self.frames as f64 } else { 0.0 };
        let min = if self.fps_min == f64::MAX { 0.0 } else { self.fps_min };
        let _ = writeln!(self.file, "----------------------------------");
        let _ = writeln!(
            self.file,
            "汇总: 运行 {:.1}s, 帧数 {}, 平均 fps {:.1}, 最低 {:.1}, 最高 {:.1}",
            dur.as_secs_f32(), self.frames, avg, min, self.fps_max
        );
        let _ = self.file.flush();
        log::info!("性能日志已保存: {}", self.path.display());
    }
}

/// 可读时间 YYYY-MM-DD HH:MM:SS（头部用）
pub fn now_human() -> String {
    let ts = chrono_like_timestamp();
    format!(
        "{}-{}-{} {}:{}:{}",
        &ts[0..4], &ts[4..6], &ts[6..8], &ts[9..11], &ts[11..13], &ts[13..15]
    )
}

/// 本地时间戳 YYYYMMDD_HHMMSS（不引入 chrono 依赖）
fn chrono_like_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 用本地时间：通过简单换算（UTC+8 固定偏移，够用）
    let secs = now + 8 * 3600;
    let days = secs / 86400;
    let rem = secs % 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, m, d, hh, mm, ss)
}

/// 天数 → 公历日期（Howard Hinnant 算法）
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d as i64)
}