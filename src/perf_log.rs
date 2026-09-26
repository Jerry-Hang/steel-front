//! 性能日志系统（2026-08-16）：每次启动创建一份独立性能日志（logs/perf_*.log），
//! 记录硬件信息 + 运行期帧率/阶段耗时采样 + 退出汇总，便于后续调试与性能回归。
use std::io::Write;
use std::path::PathBuf;

/// 列定义（**唯一真源**）：`scripts\perf_run.ps1` 按**下标**解析这些列，
/// 改这里的顺序必须同步那个脚本的 `$f[n]` 映射与它的 Columns 注释。
///
/// 🔴 2026-09-26：加 `dt_us`（**帧间隔**）并**改掉 fps 的口径**。旧实现把
/// `1/上一帧 dt`（瞬时值）当 fps 写进日志，与同一行的 `frame_us`（**本帧** render 耗时）
/// 根本是两帧的量 ⇒ 日志里长期存在「163.8 fps 配 7550µs 帧耗时」这种自相矛盾的行，
/// 而"两次跑同一二进制差 48%"的假象就是从这来的（见下方 `window_fps`）。
/// 现在 `fps` = **采样窗口内的真实帧率**，`dt_us` = 本帧间隔，`frame_us` = 本帧 render 耗时，
/// 三者同源于一帧、彼此可校对。
pub const PERF_LOG_COLUMNS: [&str; 12] = [
    "时间(s)",
    "fps",
    "dt_us",
    "frame_us",
    "cull",
    "terrain",
    "wait",
    "acquire",
    "record",
    "submit",
    "present",
    "near",
];

/// 采样窗口的真实帧率 = 窗口内帧数 / 窗口时长（纯函数，可单测）。
///
/// ⚠️ **不能**用"某一帧的 `1/dt`"代表帧率：那既丢掉了窗口内其他帧，又与同一行的
/// `frame_us` 不同源。判定性能改动一律看这个值，别再看瞬时倒数。
fn window_fps(frames: u64, elapsed_secs: f32) -> f64 {
    if elapsed_secs <= 1e-6 {
        return 0.0; // 除零保护：宁可写 0，也不要 NaN/inf 进日志（会把统计全污染）
    }
    frames as f64 / elapsed_secs as f64
}

pub struct PerfLog {
    file: std::fs::File,
    path: PathBuf,
    start: std::time::Instant,
    frames: u64,
    /// 本次采样窗口内累计的帧数（`frame()` 每次 +1，写行时清零）
    window_frames: u64,
    /// 已写出的采样窗口数（= 日志行数）。`fps_sum` 是**逐窗口**累加的，
    /// 所以求平均必须除它 —— 除 `frames`（帧数）会把平均值按帧数缩小（旧实现的遗留算法）。
    samples: u64,
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
        let _ = writeln!(file, "{}", PERF_LOG_COLUMNS.join("\t"));
        Some(Self {
            file,
            path,
            start: std::time::Instant::now(),
            frames: 0,
            window_frames: 0,
            samples: 0,
            fps_sum: 0.0,
            fps_min: f64::MAX,
            fps_max: 0.0,
            last_sample: std::time::Instant::now(),
        })
    }

    /// 每帧调用（内部 1s 采样一次写一行）；`dt_us` = 本帧间隔，snap 为渲染阶段耗时。
    ///
    /// 返回值就是写进日志的窗口帧率（0 = 本帧没写行），调用方不必自己算 fps。
    pub fn frame(&mut self, dt_us: u64, near: u32, snap: &crate::engine::renderer::PerfSnapshot) -> f64 {
        self.frames += 1;
        self.window_frames += 1;
        let elapsed = self.last_sample.elapsed().as_secs_f32();
        if elapsed < 1.0 {
            return 0.0;
        }
        // 窗口帧率在这里算：调用方给不了更好的数（它手上只有"这一帧"的信息）。
        let fps = window_fps(self.window_frames, elapsed);
        self.window_frames = 0;
        self.samples += 1;
        self.fps_sum += fps;
        self.fps_min = self.fps_min.min(fps);
        self.fps_max = self.fps_max.max(fps);
        let t = self.start.elapsed().as_secs_f32();
        let _ = writeln!(
            self.file,
            "{:.1}\t{:.1}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            t, fps, dt_us, snap.frame_us, snap.cull_us, snap.terrain_us, snap.wait_fence_us,
            snap.acquire_us, snap.record_us, snap.submit_us, snap.present_us, near
        );
        let _ = self.file.flush();
        self.last_sample = std::time::Instant::now();
        fps
    }

    /// 退出时写汇总
    pub fn finish(&mut self) {
        let dur = self.start.elapsed();
        // avg = **逐窗口帧率**的算术平均（每行一个窗口）。⚠️ 分母是 `samples`（窗口数），
        // 不是 `frames`（帧数）：后者是旧实现的遗留，会把平均值按帧数缩成垃圾。
        // 另外再给一个"总帧数 / 总时长"——它是**吞吐**口径，和逐窗口平均互为对照
        // （两者不等时说明窗口长度不齐，适合用来发现日志本身的异常）。
        let avg = if self.samples > 0 { self.fps_sum / self.samples as f64 } else { 0.0 };
        let throughput = if dur.as_secs_f64() > 1e-6 {
            self.frames as f64 / dur.as_secs_f64()
        } else {
            0.0
        };
        let min = if self.fps_min == f64::MAX { 0.0 } else { self.fps_min };
        let _ = writeln!(self.file, "----------------------------------");
        let _ = writeln!(
            self.file,
            "汇总: 运行 {:.1}s, 帧数 {}, 采样 {} 个窗口, 窗口平均 fps {:.1}, 吞吐 fps {:.1}, 最低 {:.1}, 最高 {:.1}",
            dur.as_secs_f32(), self.frames, self.samples, avg, throughput, min, self.fps_max
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

#[cfg(test)]
mod tests {
    use super::{window_fps, PERF_LOG_COLUMNS};

    /// 判据：帧率**必须**由"窗口内帧数 / 窗口时长"给出。
    ///
    /// 依据（2026-09-26）：旧实现写的是 `1/上一帧 dt`，与同一行的 `frame_us`（本帧 render
    /// 耗时）不是同一帧 ⇒ 日志里出现 `163.8 fps` 配 `frame_us=7550`（=132 fps）这种行。
    /// 更要命的是它让"同一二进制两次跑"看起来能差 48%：偶尔采到长帧就记一个低 fps。
    /// 这条测试钉住两件事：**帧数进分子**、**除零返回 0 而不是 NaN/inf**。
    #[test]
    fn window_fps_measures_the_window_not_the_last_frame() {
        assert!((window_fps(60, 1.0) - 60.0).abs() < 1e-9);
        assert!((window_fps(120, 1.0) - 120.0).abs() < 1e-9, "帧数进分子");
        assert!((window_fps(30, 0.5) - 60.0).abs() < 1e-9, "按真实时长归一");
        assert_eq!(window_fps(0, 1.0), 0.0, "一帧没渲染的窗口是 0，不是 NaN");
        let degenerate = window_fps(10, 0.0);
        assert_eq!(degenerate, 0.0);
        assert!(degenerate.is_finite(), "除零不许产出 NaN/inf 污染统计");
    }

    /// 判据：列定义是 **`scripts\perf_run.ps1` 的解析契约**。
    ///
    /// 那个脚本按**下标**取列（`$f[1]`=fps、`$f[2]`=dt_us…），所以顺序不是排版问题而是接口。
    /// 谁插/换一列而没同步脚本，perf 统计就会静默对不上号（本仓已发生过同类"尺子量错东西"）。
    #[test]
    fn perf_log_columns_are_the_script_parse_contract() {
        assert_eq!(PERF_LOG_COLUMNS.len(), 12, "perf_run.ps1 要求 12 列");
        assert_eq!(PERF_LOG_COLUMNS[0], "时间(s)");
        assert_eq!(PERF_LOG_COLUMNS[1], "fps", "列 1：窗口帧率");
        assert_eq!(PERF_LOG_COLUMNS[2], "dt_us", "列 2：本帧间隔（与 fps 同源）");
        assert_eq!(PERF_LOG_COLUMNS[3], "frame_us", "列 3：本帧 render 耗时");
        assert_eq!(PERF_LOG_COLUMNS[11], "near", "列 11：实例场 near 计数");
        assert_eq!(
            PERF_LOG_COLUMNS.iter().filter(|c| c.is_empty()).count(),
            0,
            "空列名会让 join('\\t') 产出连续制表符 = 少一列，脚本直接错位"
        );
    }
}