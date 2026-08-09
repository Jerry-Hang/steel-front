//! CPU 拓扑检测与线程亲和性（零第三方依赖：sysfs + CPUID + `sched_setaffinity` FFI）
//!
//! 目标（用户策略）：
//! - AMD 双 CCD：主线程（游戏逻辑/物理/渲染提交）绑首簇（vCPU 前半 = CCD0，默认频率偏高），
//!   次簇（后半 = CCD1）留给 AI/地图生成等未来后台线程。
//! - Intel 混合架构：主线程绑 P-core 组；E-core ≤8 只接音频等轻任务，>8 时 E-core 组
//!   可承担 AI 判定/部分地图生成（决策已编码为 `secondary_set` 语义，供未来线程池接入）。
//! - AVX-512：Zen4/Zen5（7000/9000 系）原生支持，运行时检测后由 renderer 选路启用。
//!
//! 注意：WSL2 虚拟化会抹平 L3/NUMA 分组（sysfs L3 `shared_cpu_list` 全 0-31、仅 node0），
//! 因此双簇推断采用「vCPU 枚举顺序 = 物理枚举顺序」：前半 = 首簇、后半 = 次簇，
//! 可用环境变量 `RV3D_CPU_PIN` 覆盖精确亲和性掩码（如 `RV3D_CPU_PIN=0-7,16-23`）。

use std::arch::x86_64::__cpuid;
use std::sync::OnceLock;

/// CPU 厂商（CPUID leaf 0x0 的 12 字节 vendor 串）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    Amd,
    Intel,
    Other,
}

/// 检测到的 CPU 拓扑摘要（启动时输出日志，供线程调度决策）
pub struct CpuTopology {
    pub vendor: CpuVendor,
    /// 逻辑处理器总数（vCPU）
    pub threads: usize,
    /// 首簇 vCPU 集合（AMD = 前半 CCD0；Intel = P-core 组）
    pub primary_set: Vec<usize>,
    /// 次簇 vCPU 集合（AMD = 后半 CCD1；Intel = E-core 组，≤8 时仅轻任务）
    pub secondary_set: Vec<usize>,
    /// Intel 能效核数量（CPUID leaf 0x1A hybrid；AMD 恒 0）
    pub e_cores: usize,
    pub avx2: bool,
    pub avx512: bool,
}

// Linux `sched_setaffinity`（cpu_set_t = 1024 位，x86_64 下 16×u64）
extern "C" {
    fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u64) -> i32;
}

fn cpu_vendor() -> CpuVendor {
    let r = __cpuid(0);
    let mut v = [0u8; 12];
    v[0..4].copy_from_slice(&r.ebx.to_le_bytes());
    v[4..8].copy_from_slice(&r.edx.to_le_bytes());
    v[8..12].copy_from_slice(&r.ecx.to_le_bytes());
    match &v {
        b"AuthenticAMD" => CpuVendor::Amd,
        b"GenuineIntel" => CpuVendor::Intel,
        _ => CpuVendor::Other,
    }
}

/// 全局缓存：AVX-512 是否允许启用（CPUID 支持 + 厂商/型号过滤 + 环境变量覆盖）
static AVX512_ALLOWED: OnceLock<bool> = OnceLock::new();

/// AVX-512 可用性判定（renderer 选路与日志共用）：
/// - 硬件不支持（`is_x86_feature_detected`）→ false；
/// - `RV3D_DISABLE_AVX512=1` 强制关闭 → false；
/// - Intel 11 代（Rocket Lake 0xA7 / Tiger Lake 0x8C/0x8D）：AVX-512 能效与降频极差，
///   游戏场景负收益，默认关闭（用户策略）；
/// - Intel 12 代起（model ≥ Alder Lake 0x97，含 13/14 代）：大小核，E-core 无 AVX-512，
///   出厂已熔丝禁用（CPUID 通常不报告）；此处防御性过滤，防虚拟化/BIOS 异常透传；
/// - AMD Zen4/Zen5 及 Intel 高性能平台（Ice Lake/Skylake-X 等）→ true。
pub fn avx512_enabled() -> bool {
    *AVX512_ALLOWED.get_or_init(|| {
        if !std::is_x86_feature_detected!("avx512f") {
            return false;
        }
        if std::env::var("RV3D_DISABLE_AVX512").is_ok_and(|v| v == "1" || v == "true") {
            return false;
        }
        if cpu_vendor() == CpuVendor::Intel {
            let eax = __cpuid(1).eax;
            let model = ((eax >> 4) & 0x0f) | ((eax >> 12) & 0xf0);
            // 11 代 Rocket Lake/Tiger Lake 与 12 代起的所有型号
            if model == 0xA7 || model == 0x8C || model == 0x8D || model >= 0x97 {
                return false;
            }
        }
        true
    })
}

/// 从 sysfs `/sys/devices/system/cpu/online` 解析逻辑处理器总数（格式 "0-31" 或 "0,2,4"）
fn cpu_thread_count() -> usize {
    let Ok(s) = std::fs::read_to_string("/sys/devices/system/cpu/online") else {
        return std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    };
    let mut count = 0usize;
    for part in s.trim().split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let (a, b) = (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().unwrap_or(0));
            count += b.saturating_sub(a) + 1;
        } else if !part.is_empty() {
            count += 1;
        }
    }
    count.max(1)
}

/// 临时把当前线程绑到指定 vCPU，读一个 CPUID leaf，再恢复全集合绑定。
/// 用于遍历每核的 hybrid core type（Intel leaf 0x1A）。
fn probe_per_cpu(leaf: u32) -> Vec<u32> {
    let threads = cpu_thread_count();
    let mut out = Vec::with_capacity(threads);
    for cpu in 0..threads {
        let mut mask = [0u64; 16];
        mask[cpu / 64] = 1u64 << (cpu % 64);
        unsafe {
            sched_setaffinity(0, std::mem::size_of::<[u64; 16]>(), mask.as_ptr());
            out.push(__cpuid(leaf).eax);
        }
    }
    // 恢复全集合（避免影响后续线程调度）
    let mut full = [0u64; 16];
    for i in 0..threads {
        full[i / 64] |= 1u64 << (i % 64);
    }
    unsafe {
        sched_setaffinity(0, std::mem::size_of::<[u64; 16]>(), full.as_ptr());
    }
    out
}

/// 解析用户掩码（`RV3D_CPU_PIN`），支持 "0-7"、"0-3,16-19" 形式
fn parse_cpu_list(spec: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        if let Some((a, b)) = part.split_once('-') {
            let a = a.trim().parse::<usize>().ok()?;
            let b = b.trim().parse::<usize>().ok()?;
            if a > b {
                return None;
            }
            for c in a..=b {
                cpus.push(c);
            }
        } else {
            cpus.push(part.parse::<usize>().ok()?);
        }
    }
    Some(cpus)
}

impl CpuTopology {
    /// 启动时检测（CPUID + sysfs），一次调用、毫秒级
    pub fn detect() -> CpuTopology {
        let vendor = cpu_vendor();
        let threads = cpu_thread_count();
        let half = threads / 2;
        let (primary_set, secondary_set, e_cores) = match vendor {
            CpuVendor::Amd => (
                (0..half).collect(),
                (half..threads).collect(),
                0,
            ),
            CpuVendor::Intel => {
                // leaf 0x1A EAX[31:24] = 0x20 表示 E-core（EAX[15:8] = 核内编号，SMT 去重）；
                // AMD（leaf 0x1A 返回 0）与虚拟化未透传时 e_cpus 为空。
                let hybrid = probe_per_cpu(0x1A);
                let mut seen = std::collections::HashSet::new();
                for (i, &eax) in hybrid.iter().enumerate() {
                    if ((eax >> 24) & 0xff) == 0x20 {
                        seen.insert(i);
                    }
                }
                let e_cpus = seen;
                let p: Vec<usize> = (0..threads).filter(|c| !e_cpus.contains(c)).collect();
                let e: Vec<usize> = (0..threads).filter(|c| e_cpus.contains(c)).collect();
                let e_count = e.len();
                (p, e, e_count)
            }
            CpuVendor::Other => ((0..threads).collect(), Vec::new(), 0),
        };
        CpuTopology {
            vendor,
            threads,
            primary_set,
            secondary_set,
            e_cores,
            avx2: std::is_x86_feature_detected!("avx2"),
            avx512: avx512_enabled(),
        }
    }

    /// 把当前线程（主线程）绑定到目标 vCPU 集合。
    /// `RV3D_CPU_PIN=off` 跳过；`RV3D_CPU_PIN=<掩码>` 精确覆盖；默认绑首簇。
    /// 返回实际生效的 vCPU 集合（未绑定时返回全集合）。
    pub fn pin_main_thread(&self) -> Vec<usize> {
        let target = match std::env::var("RV3D_CPU_PIN") {
            Ok(v) if v == "off" => return (0..self.threads).collect(),
            Ok(v) => match parse_cpu_list(&v) {
                Some(cpus) => cpus,
                None => {
                    log::warn!("cpu: RV3D_CPU_PIN 格式无效（期望如 0-7,16-23），使用默认首簇");
                    self.primary_set.clone()
                }
            },
            Err(_) => self.primary_set.clone(),
        };
        let mut mask = [0u64; 16];
        for &c in &target {
            if c < self.threads {
                mask[c / 64] |= 1u64 << (c % 64);
            }
        }
        let ok = unsafe {
            sched_setaffinity(0, std::mem::size_of::<[u64; 16]>(), mask.as_ptr()) == 0
        };
        if ok {
            log::info!(
                "cpu: 主线程已绑定 vCPU {:?}（{} 核 {} 线程，{}）",
                target,
                self.threads / 2,
                self.threads,
                match self.vendor {
                    CpuVendor::Amd => "AMD 双簇：主=CCD0，次=CCD1",
                    CpuVendor::Intel => "Intel 混合：主=P-core，次=E-core",
                    CpuVendor::Other => "未知厂商",
                }
            );
        } else {
            log::warn!("cpu: sched_setaffinity 失败（环境不支持），保持默认调度");
        }
        target
    }

    /// 启动日志摘要（厂商/簇/E-core/指令集）
    pub fn log_summary(&self) {
        log::info!(
            "cpu: vendor={:?} threads={} primary={:?} secondary={:?} e_cores={} avx2={} avx512={}",
            self.vendor,
            self.threads,
            self.primary_set,
            self.secondary_set,
            self.e_cores,
            self.avx2,
            self.avx512
        );
    }
}
