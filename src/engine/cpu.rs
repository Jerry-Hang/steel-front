//! CPU 拓扑检测与线程亲和性（零第三方依赖：sysfs + CPUID + `sched_setaffinity` FFI）
//!
//! 目标（用户策略）：
//! - AMD 双 CCD：主线程（游戏逻辑/物理/渲染提交）绑首簇（vCPU 前半 = CCD0，默认频率偏高），
//!   次簇（后半 = CCD1）专供 AI/地图生成等后台线程（`ai_pool`），杜绝跨 CCD 访问。
//! - Intel 混合架构：主线程绑 P-core 组；渲染侧场景计算（剔除/上传/地形 morph）只用
//!   P-core（`scene_pool`，绝不丢到 E-core）；AI 分层负载（2026-08-11）：近组/与玩家
//!   交互的 NPC 走 `scene_pool`（P-core），远组/延迟不敏感 AI 走 E-core 组
//!   （`ai_pool`，有 E-core 即绑定，无论数量；无 E-core 平台回退 P-core）。
//! - 渲染线程不固定到 1-2 个核：主线程与 `scene_pool` 绑定的是「整簇集合」，
//!   由 OS 调度器把渲染工作分给集合内空闲率最高的核（多核共同承担线程渲染）。
//! - AVX-512：Zen4/Zen5（7000/9000 系）原生支持，运行时检测后由 renderer 选路启用。
//!
//! 注意：WSL2 虚拟化会抹平 L3/NUMA 分组（sysfs L3 `shared_cpu_list` 全 0-31、仅 node0），
//! 因此双簇推断采用「vCPU 枚举顺序 = 物理枚举顺序」：前半 = 首簇、后半 = 次簇，
//! 可用环境变量 `RV3D_CPU_PIN` 覆盖精确亲和性掩码（如 `RV3D_CPU_PIN=0-7,16-23`）。
//!
//! 物理核/超线程识别（2026-08-12，优化点 1）：sysfs `thread_siblings_list` 在 WSL2 下
//! 保留真实 SMT 配对（实测 8940HX：0-1/2-3/…，偶数 vCPU = 物理主线程），
//! 高性能线程（主线程/渲染/scene 池）严格绑定物理核集合，超线程仅在池线程数超过
//! 物理核数时作为溢出辅助（用户策略：延迟敏感任务避开超线程）。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::__cpuid;

/// CPU 厂商（CPUID leaf 0x0 的 12 字节 vendor 串）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuVendor {
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    Amd,
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
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
    /// 首簇内物理核心主线程 vCPU（每个 SMT 对的最小 vCPU；sysfs 不可读时 = primary_set）
    pub primary_physical: Vec<usize>,
    /// 次簇内物理核心主线程 vCPU（AMD = CCD1 物理核；Intel E-core 无 SMT 时 = secondary_set）
    pub secondary_physical: Vec<usize>,
    /// 全部超线程 vCPU（SMT 对的最大 vCPU；无 SMT/sysfs 不可读时为空）
    pub smt_set: Vec<usize>,
    /// Intel 能效核数量（CPUID leaf 0x1A hybrid；AMD 恒 0）
    pub e_cores: usize,
    pub avx2: bool,
    pub avx512: bool,
    /// CCX/L3 缓存组（每组的逻辑处理器集合；Windows 按 L3 缓存精确分组，
    /// Zen1/Zen2 为 4 核 CCX、Zen3+ 为 8 核 CCD——旧“半半”拆分在早期 Zen 上错误）
    pub ccx_groups: Vec<Vec<usize>>,
    /// 每逻辑处理器能效等级（0=P 大核, 1=E 能效核, 2=LE 低功耗核；未知恒 0）
    pub efficiency: Vec<u8>,
    /// 低功耗能效核（Intel LE/LP-E；AMD 无 = 空）
    pub le_set: Vec<usize>,
    /// 音频等简单低延迟任务集合（LE > E > 第三 CCX > 次簇 > 主簇）
    pub audio_set: Vec<usize>,
}

/// 全局拓扑缓存：`topology()` 首次调用检测一次，后续零成本取用（避免 Game/Renderer 重复探测）。
/// 平台精确拓扑中间结构（Windows 由 GetLogicalProcessorInformationEx 解析）
struct PlatformTopo {
    /// 物理核列表：每核 = 该核的逻辑处理器集合（SMT 配对，E-core 单成员）
    cores: Vec<Vec<usize>>,
    /// L3 缓存组（CCX）：每组的逻辑处理器集合
    ccx_groups: Vec<Vec<usize>>,
    /// 每逻辑处理器能效等级（0=P 1=E 2=LE）
    efficiency: Vec<u8>,
}

/// Windows：GetLogicalProcessorInformationEx 解析物理核/CCX(L3)/能效等级。
/// 零第三方依赖（kernel32 直接 FFI）；失败返回 None 回退旧逻辑。
#[cfg(target_os = "windows")]
mod win_topology {
    use super::PlatformTopo;

    const REL_PROCESSOR_CORE: u32 = 0;
    const REL_CACHE: u32 = 2;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct GroupAffinity {
        mask: usize,
        group: u16,
        reserved: [u16; 3],
    }

    #[repr(C)]
    struct ProcessorRel {
        flags: u8,
        efficiency: u8,
        reserved: [u8; 20],
        group_count: u16,
        group_mask: [GroupAffinity; 1],
    }

    #[repr(C)]
    struct CacheRel {
        level: u8,
        associativity: u8,
        line_size: u16,
        cache_size: u32,
        cache_type: u32,
        reserved: [u8; 20],
        group_mask: GroupAffinity,
    }

    #[repr(C)]
    struct InfoEx {
        relationship: u32,
        // 40 字节联合体（实测 sizeof = 48：缓存/核心关系条目均 48B 步长，2026-08-22 实测）
        data: [u64; 5],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalProcessorInformationEx(
            relationship: u32,
            buffer: *mut InfoEx,
            returned_length: *mut u32,
        ) -> i32;
    }

    fn query(rel: u32) -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        unsafe {
            GetLogicalProcessorInformationEx(rel, std::ptr::null_mut(), &mut len);
        }
        if len == 0 {
            return None;
        }
        let mut buf: Vec<u8> = vec![0u8; len as usize];
        let ok = unsafe {
            GetLogicalProcessorInformationEx(
                rel,
                buf.as_mut_ptr() as *mut InfoEx,
                &mut len,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(buf)
    }

    fn mask_members(mask: usize) -> Vec<usize> {
        let mut v = Vec::new();
        for b in 0..usize::BITS {
            if mask & (1usize << b) != 0 {
                v.push(b as usize);
            }
        }
        v
    }

    pub fn detect() -> Option<PlatformTopo> {
        let mut cores: Vec<Vec<usize>> = Vec::new();
        let mut efficiency: Vec<u8> = Vec::new();
        let mut ccx: Vec<Vec<usize>> = Vec::new();

        if let Some(buf) = query(REL_PROCESSOR_CORE) {
            let stride = std::mem::size_of::<InfoEx>();
            for i in 0..buf.len() / stride {
                let e = unsafe { &*(buf.as_ptr().add(i * stride) as *const InfoEx) };
                if e.relationship != REL_PROCESSOR_CORE {
                    continue;
                }
                let pr = unsafe { &*(e.data.as_ptr() as *const ProcessorRel) };
                if pr.group_count < 1 {
                    continue;
                }
                let members = mask_members(pr.group_mask[0].mask);
                if !members.is_empty() {
                    for _ in 0..members.len() {
                        efficiency.push(pr.efficiency);
                    }
                    cores.push(members);
                }
            }
        }
        if let Some(buf) = query(REL_CACHE) {
            let stride = std::mem::size_of::<InfoEx>();
            for i in 0..buf.len() / stride {
                let e = unsafe { &*(buf.as_ptr().add(i * stride) as *const InfoEx) };
                if e.relationship != REL_CACHE {
                    continue;
                }
                let cr = unsafe { &*(e.data.as_ptr() as *const CacheRel) };
                if cr.level == 3 {
                    let members = mask_members(cr.group_mask.mask);
                    if !members.is_empty() {
                        ccx.push(members);
                    }
                }
            }
        }
        if cores.is_empty() {
            return None;
        }
        Some(PlatformTopo {
            cores,
            ccx_groups: ccx,
            efficiency,
        })
    }
}

static TOPOLOGY: OnceLock<CpuTopology> = OnceLock::new();

/// 取全局 CPU 拓扑（首次调用触发 `detect()`，幂等）
pub fn topology() -> &'static CpuTopology {
    TOPOLOGY.get_or_init(CpuTopology::detect)
}

// Linux `sched_setaffinity`（cpu_set_t = 1024 位，x86_64 下 16×u64）；
// 仅 Linux 提供该系统调用（macOS/iOS 无，Apple Silicon 构建时线程调度交由系统 QoS）
#[cfg(target_os = "linux")]
extern "C" {
    fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u64) -> i32;
}

// Windows 线程亲和（Zero-dep：GetCurrentThread/SetThreadAffinityMask 直接 FFI）。
// 仅 x64 单处理器组（<64 逻辑线程）适用；>64 线程时 mask 截断并告警。
#[cfg(target_os = "windows")]
extern "system" {
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn SetThreadAffinityMask(hThread: *mut std::ffi::c_void, dwThreadAffinityMask: usize) -> usize;
}

#[cfg(target_arch = "x86_64")]
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

#[cfg(not(target_arch = "x86_64"))]
fn cpu_vendor() -> CpuVendor {
    // 非 x86_64（如 Apple Silicon AArch64）：无 CPUID，按"未知厂商"处理
    CpuVendor::Other
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
        #[cfg(target_arch = "x86_64")]
        {
            avx512_allowed_x86()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            // 非 x86_64 无 AVX-512
            false
        }
    })
}

/// x86_64 专用判定：硬件支持 + 环境变量 + Intel 型号过滤
#[cfg(target_arch = "x86_64")]
fn avx512_allowed_x86() -> bool {
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
}

/// 基准/调试用强制选路：`RV3D_FORCE_SIMD=avx512|avx2|avx|sse4.2|scalar` 锁定指令集路径，
/// 供 SIMD 指令级 A/B 对比（shockwave 压力场 / 视锥剔除 / 地形 morph 共用）。
/// 默认空 = 自动选路；非法值告警一次并回退自动选路；强制档位仍要求硬件支持
/// （如强制 avx512 但 CPU 无 avx512f 则回退自动，防 SIGILL）。
pub fn forced_simd_path() -> Option<&'static str> {
    static WARNED: OnceLock<bool> = OnceLock::new();
    let Some(v) = std::env::var("RV3D_FORCE_SIMD").ok() else {
        return None;
    };
    let v = v.trim();
    let known = matches!(v, "avx512" | "avx2" | "avx" | "sse4.2" | "scalar");
    if !known {
        WARNED.get_or_init(|| {
            log::warn!("cpu: RV3D_FORCE_SIMD 值无效（{v}），忽略并回退自动选路");
            true
        });
        return None;
    }
    match v {
        "avx512" => Some("avx512"),
        "avx2" => Some("avx2"),
        "avx" => Some("avx"),
        "sse4.2" => Some("sse4.2"),
        _ => Some("scalar"),
    }
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
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
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

/// 从 SMT 兄弟列表划分物理主线程与超线程：
/// 每组 `siblings`（同一物理核心的 vCPU）取编号最小者为物理主线程，其余为超线程。
/// 单成员组（如 Intel E-core，无 SMT）= 自身即物理核。
fn split_smt_pairs(siblings: &[Vec<usize>]) -> (Vec<usize>, Vec<usize>) {
    let mut physical = Vec::new();
    let mut smt = Vec::new();
    for group in siblings {
        if group.is_empty() {
            continue;
        }
        let min = *group.iter().min().expect("非空组必有最小值");
        physical.push(min);
        for &c in group {
            if c != min {
                smt.push(c);
            }
        }
    }
    physical.sort_unstable();
    smt.sort_unstable();
    (physical, smt)
}

/// 读 sysfs 收集每个物理核心的 SMT 兄弟组，返回 `Vec<group>`。
/// 任意 CPU 的 `thread_siblings_list` 不可读时返回 None（调用方回退不区分物理/超线程）。
fn read_smt_siblings(threads: usize) -> Option<Vec<Vec<usize>>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for cpu in 0..threads {
        let path = format!("/sys/devices/system/cpu/cpu{}/topology/thread_siblings_list", cpu);
        let s = std::fs::read_to_string(&path).ok()?;
        let members = parse_cpu_list(s.trim())?;
        // 只保留编号最小者代表该组（同组在后续 CPU 重复出现，去重）
        let min = *members.iter().min()?;
        if min == cpu {
            out.push(members);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

impl CpuTopology {
    /// 场景计算集合（视锥剔除/实例上传/地形 morph 等渲染侧 CPU 工作）：
    /// AMD = 首簇 CCD0 物理核（与渲染主线程同簇，避免跨 CCD 访问，且避开超线程）；
    /// Intel = 仅 P-core 物理核（杜绝渲染工作被调度到 E-core/超线程）。
    pub fn scene_compute_set(&self) -> &[usize] {
        if self.primary_physical.is_empty() {
            &self.primary_set
        } else {
            &self.primary_physical
        }
    }

    /// AI/地图生成集合（用户策略，2026-08-11 修正）：
    /// - AMD = 次簇 CCD1（与主线程所在 CCD0 分离，AI 并行不吃主线程带宽）；
    /// - Intel = 有 E-core 即交 E-core 组（无论数量，接"远 AI"分层负载；E-core 少如
    ///   12600K/13400F/12700K 的 4E 也接远组），近组/交互 AI 走 scene_pool（仅 P-core）；
    ///   无 E-core（全 P-core 平台）回退 primary_set。
    pub fn ai_set(&self) -> &[usize] {
        if self.vendor == CpuVendor::Intel && !self.secondary_set.is_empty() {
            // E-core 无超线程，集合即物理核，直接使用
            &self.secondary_set
        } else if self.vendor == CpuVendor::Amd && !self.secondary_set.is_empty() {
            if self.secondary_physical.is_empty() {
                &self.secondary_set
            } else {
                &self.secondary_physical
            }
        } else {
            &self.primary_set
        }
    }

    /// 启动时检测（CPUID + sysfs），一次调用、毫秒级
    pub fn detect() -> CpuTopology {
        let vendor = cpu_vendor();
        let threads = cpu_thread_count();
        // 平台精确拓扑：Windows 用 GetLogicalProcessorInformationEx（物理核/CCX/能效等级），
        // 其它平台 None → 回退旧逻辑（sysfs + CPUID + 半半）。
        #[cfg(target_os = "windows")]
        let plat = win_topology::detect();
        #[cfg(not(target_os = "windows"))]
        let plat: Option<PlatformTopo> = None;

        let (primary_set, secondary_set, le_set, ccx_groups, efficiency, e_cores) =
            if let Some(pt) = &plat {
                if pt.efficiency.iter().any(|&e| e != 0) {
                    // 混合架构（Intel P/E/LE；AMD Zen4C/5C 被系统标记时同样生效）：
                    // 按能效等级分组：0=P 主簇（渲染/逻辑），1=E（AI/后台），2=LE（音频/低延迟）
                    let n = pt.efficiency.len();
                    let p: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 0).collect();
                    let e: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 1).collect();
                    let le: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 2).collect();
                    // E 核物理数：每物理核（cores 条目）只要有一成员为 E 即计 1
                    let mut ec = 0usize;
                    for core in &pt.cores {
                        if core.iter().any(|&c| c < pt.efficiency.len() && pt.efficiency[c] == 1) {
                            ec += 1;
                        }
                    }
                    (p, e, le, pt.ccx_groups.clone(), pt.efficiency.clone(), ec)
                } else if pt.ccx_groups.len() >= 2 {
                    // AMD 双簇（CCX/L3 分组）：主簇 = 组 0（逻辑/渲染同簇，共享 L3），
                    // 次簇 = 组 1（AI/地图生成）。Zen1/Zen2 为 4 核 CCX 时精确成立。
                    (
                        pt.ccx_groups[0].clone(),
                        pt.ccx_groups[1].clone(),
                        Vec::new(),
                        pt.ccx_groups.clone(),
                        pt.efficiency.clone(),
                        0,
                    )
                } else {
                    // 无小核且无 L3 分组：回退半半
                    let half = threads / 2;
                    (
                        (0..half).collect(),
                        (half..threads).collect(),
                        Vec::new(),
                        pt.ccx_groups.clone(),
                        pt.efficiency.clone(),
                        0,
                    )
                }
            } else {
                // 旧逻辑（Linux sysfs / 其它平台）——语义不变
                let (primary_set, secondary_set, e_cores) = match vendor {
                    CpuVendor::Amd => {
                        let half = threads / 2;
                        ((0..half).collect::<Vec<usize>>(), (half..threads).collect::<Vec<usize>>(), 0)
                    }
                    CpuVendor::Intel => {
                        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                        {
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
                        #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
                        {
                            ((0..threads).collect(), Vec::new(), 0)
                        }
                    }
                    CpuVendor::Other => ((0..threads).collect(), Vec::new(), 0),
                };
                (primary_set, secondary_set, Vec::new(), Vec::new(), Vec::new(), e_cores)
            };
        // 物理核/超线程识别：Windows 从核心关系直接获得（SMT 配对 = 每核逻辑集合）；
        // Linux sysfs；不可读时回退不区分
        let (primary_physical, secondary_physical, smt_set) = if let Some(pt) = &plat {
            let mut physical_all = Vec::new();
            let mut smt_all = Vec::new();
            for core in &pt.cores {
                let min = *core.iter().min().unwrap_or(&0);
                physical_all.push(min);
                for &c in core {
                    if c != min {
                        smt_all.push(c);
                    }
                }
            }
            let pp: Vec<usize> = primary_set
                .iter()
                .filter(|c| physical_all.contains(c))
                .copied()
                .collect();
            let sp: Vec<usize> = secondary_set
                .iter()
                .filter(|c| physical_all.contains(c))
                .copied()
                .collect();
            let pp = if pp.is_empty() { primary_set.clone() } else { pp };
            let sp = if sp.is_empty() { secondary_set.clone() } else { sp };
            (pp, sp, smt_all)
        } else {
            match read_smt_siblings(threads) {
                Some(groups) => {
                    let (physical_all, smt_all) = split_smt_pairs(&groups);
                    let pp: Vec<usize> = primary_set
                        .iter()
                        .filter(|c| physical_all.contains(c))
                        .copied()
                        .collect();
                    let sp: Vec<usize> = secondary_set
                        .iter()
                        .filter(|c| physical_all.contains(c))
                        .copied()
                        .collect();
                    let pp = if pp.is_empty() { primary_set.clone() } else { pp };
                    let sp = if sp.is_empty() { secondary_set.clone() } else { sp };
                    (pp, sp, smt_all)
                }
                None => (primary_set.clone(), secondary_set.clone(), Vec::new()),
            }
        };
        // 音频集合（简单/低延迟任务）：LE 核 > E 核 > 第三 CCX > 次簇 > 主簇
        let audio_set = if !le_set.is_empty() {
            le_set.clone()
        } else if !secondary_set.is_empty() && vendor == CpuVendor::Intel {
            secondary_set.clone()
        } else if ccx_groups.len() >= 3 {
            ccx_groups[2].clone()
        } else if !secondary_set.is_empty() {
            secondary_set.clone()
        } else {
            primary_set.clone()
        };
        CpuTopology {
            vendor,
            threads,
            primary_set,
            secondary_set,
            primary_physical,
            secondary_physical,
            smt_set,
            e_cores,
            avx2: {
                #[cfg(target_arch = "x86_64")]
                {
                    std::is_x86_feature_detected!("avx2")
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    false
                }
            },
            avx512: avx512_enabled(),
            ccx_groups,
            efficiency,
            le_set,
            audio_set,
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
            Err(_) => {
                if self.primary_physical.is_empty() {
                    self.primary_set.clone()
                } else {
                    self.primary_physical.clone()
                }
            }
        };
        #[cfg(target_os = "linux")]
        {
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
                    "cpu: 主线程已绑定物理核心 vCPU {:?}（{} 物理核 {} 逻辑线程，{}；SMT {}）",
                    target,
                    self.threads / 2,
                    self.threads,
                    match self.vendor {
                        CpuVendor::Amd => "AMD 双簇：主=CCD0，次=CCD1",
                        CpuVendor::Intel => "Intel 混合：主=P-core，次=E-core",
                        CpuVendor::Other => "未知厂商",
                    },
                    self.smt_set.len()
                );
            } else {
                log::warn!("cpu: sched_setaffinity 失败（环境不支持），保持默认调度");
            }
        }
        #[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
        {
            // 非 Linux/Windows（如未来 macOS/iOS Apple Silicon 构建）：无 sched_setaffinity，
            // 不手工绑核，线程调度交给系统 QoS/调度器
            log::info!("cpu: 非 Linux 平台，跳过线程手工绑定（交给系统调度）");
        }
        #[cfg(target_os = "windows")]
        {
            // Windows 线程亲和：GetCurrentThread/SetThreadAffinityMask 直接 FFI。
            // 仅 x64 单处理器组（<64 逻辑线程）适用；>64 线程时 mask 截断并告警。
            let mut mask: usize = 0;
            let mut truncated = false;
            for &c in &target {
                if c > 63 {
                    truncated = true;
                    continue; // 跳过编号 >63 的 vCPU（超出单 DWORD_PTR 掩码）
                }
                mask |= 1usize << c;
            }
            if truncated {
                log::warn!("cpu: Windows 线程亲和掩码截断（检测到 vCPU 编号 >63，仅单处理器组可用）");
            }
            let h = unsafe { GetCurrentThread() };
            let prev = unsafe { SetThreadAffinityMask(h, mask) };
            if prev == 0 {
                log::warn!("cpu: SetThreadAffinityMask 失败，保持默认调度");
            } else {
                log::info!(
                    "cpu: 主线程已绑定 vCPU {:?}（{} 逻辑线程，{}；SMT {}）",
                    target,
                    self.threads,
                    match self.vendor {
                        CpuVendor::Amd => "AMD 双簇：主=CCD0，次=CCD1",
                        CpuVendor::Intel => "Intel 混合：主=P-core，次=E-core",
                        CpuVendor::Other => "未知厂商",
                    },
                    self.smt_set.len()
                );
            }
        }
        target
    }

    /// 把「当前线程」绑定到目标 vCPU 集合（供池内工作线程/作用域线程启动时自绑）。
    /// 非 Linux 平台（如 macOS/iOS）无 `sched_setaffinity`，恒返回 false，调度交给系统。
    pub fn pin_current_thread(set: &[usize]) -> bool {
        #[cfg(target_os = "linux")]
        {
            let threads = topology().threads;
            let mut mask = [0u64; 16];
            for &c in set {
                if c < threads {
                    mask[c / 64] |= 1u64 << (c % 64);
                }
            }
            unsafe {
                sched_setaffinity(0, std::mem::size_of::<[u64; 16]>(), mask.as_ptr()) == 0
            }
        }
        #[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
        {
            let _ = set;
            false
        }
        #[cfg(target_os = "windows")]
        {
            // Windows 线程亲和：GetCurrentThread/SetThreadAffinityMask 直接 FFI。
            let mut mask: usize = 0;
            for &c in set {
                if c <= 63 {
                    mask |= 1usize << c;
                } else {
                    log::warn!("cpu: pin_current_thread 跳过 vCPU 编号 >63（{}）", c);
                }
            }
            let h = unsafe { GetCurrentThread() };
            unsafe { SetThreadAffinityMask(h, mask) != 0 }
        }
    }

    /// 启动日志摘要（厂商/簇/E-core/指令集）
    pub fn log_summary(&self) {
        log::info!(
            "cpu: vendor={:?} threads={} primary={:?} secondary={:?} e_cores={} avx2={} avx512={} scene_set={:?} ai_set={:?} physical_primary={:?} physical_secondary={:?} smt={:?} ccx_groups={:?} efficiency={:?} le_set={:?} audio_set={:?}",
            self.vendor,
            self.threads,
            self.primary_set,
            self.secondary_set,
            self.e_cores,
            self.avx2,
            self.avx512,
            self.scene_compute_set(),
            self.ai_set(),
            self.primary_physical,
            self.secondary_physical,
            self.smt_set,
            self.ccx_groups,
            self.efficiency,
            self.le_set,
            self.audio_set
        );
    }
}

/// 原始指针的 Send/Sync 包装（供线程池任务捕获）。
/// SAFETY 契约：调用方保证池任务 join 前指针指向的缓冲区有效，且各段写入区间互不相交。
pub struct SendPtr<T: ?Sized>(pub *mut T);
unsafe impl<T: ?Sized> Send for SendPtr<T> {}
unsafe impl<T: ?Sized> Sync for SendPtr<T> {}
impl<T: ?Sized> SendPtr<T> {
    /// 取回裸指针。经方法调用整体捕获 `SendPtr`（Rust 2021 闭包会按 `ptr.0` 字段
    /// 粒度捕获裸指针本身导致 Send/Sync 检查失败，方法调用强制捕获整个包装）。
    pub fn get(self) -> *mut T {
        self.0
    }
}
// 手工实现 Copy/Clone：派生会引入 `T: Copy` 隐式约束（AtomicUsize 等不可 Copy 会炸）
impl<T: ?Sized> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for SendPtr<T> {}

/// 亲和线程池：创建时把每个工作线程绑定到目标 vCPU 集合
/// （AMD = CCD1 / Intel = P-core 或 E-core，见 `ai_set`/`scene_compute_set`），
/// 避免 OS 把后台工作随意调度到主簇/能效核，也避免每帧临时 spawn 线程的开销。
pub struct ThreadPool {
    senders: Vec<std::sync::mpsc::Sender<PoolMsg>>,
    worker_count: usize,
}

enum PoolMsg {
    Run(Box<dyn FnOnce() + Send + 'static>),
    Stop,
}

impl ThreadPool {
    /// 创建线程池：`threads` 个工作线程全部绑定到 `set`（轮转分配 vCPU）。
    /// spawn 失败（资源耗尽）时静默降级：对应 sender 无接收端，`par_for_each_mut`
    /// 会检测 send 失败并自行补计完成数，不会死等。
    pub fn new(set: &[usize], threads: usize, name: &str) -> ThreadPool {
        let threads = threads.max(1);
        let mut senders = Vec::with_capacity(threads);
        let set_v = set.to_vec();
        // 超线程辅助（2026-08-12）：池线程数 ≤ 物理核集合大小时全部绑物理核；
        // 超出部分（如 RV3D_*_WORKERS 调大）绑定 物理核∪超线程 全集，
        // 由 OS 调度器平衡——物理核满载时超线程承接溢出任务。
        let topo = topology();
        let mut overflow_v: Vec<usize> = set.to_vec();
        for &s in &topo.smt_set {
            if !overflow_v.contains(&s) {
                overflow_v.push(s);
            }
        }
        for i in 0..threads {
            let (tx, rx) = std::sync::mpsc::channel::<PoolMsg>();
            senders.push(tx);
            let cpus = if i < set_v.len() { set_v.clone() } else { overflow_v.clone() };
            let tname = format!("{}-{}", name, i);
            std::thread::Builder::new()
                .name(tname)
                .spawn(move || {
                    if !cpus.is_empty() {
                        CpuTopology::pin_current_thread(&cpus);
                    }
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            PoolMsg::Run(job) => job(),
                            PoolMsg::Stop => break,
                        }
                    }
                })
                .ok();
        }
        ThreadPool {
            senders,
            worker_count: threads,
        }
    }

    /// 池内工作线程数（不含调用线程；`par_for_each_mut` 总并发 = worker_count + 1）
    pub fn workers(&self) -> usize {
        self.worker_count
    }

    /// 池析构：通知各工作线程退出（全局静态池由 OnceLock 持有，进程结束才回收；
    /// 非静态场景下用于正常关停）
    fn shutdown(&self) {
        for tx in &self.senders {
            let _ = tx.send(PoolMsg::Stop);
        }
    }

    /// 同步并行遍历 `data`：调用线程处理首段，池内线程处理其余段，返回时全部段已执行完
    /// （join 语义）。`f(seg_idx, global_start, seg_slice)` 只允许访问 `seg_slice` 指向的
    /// 不相交段；`global_start` 为段在 `data` 中的全局起始下标（AI 步进等按全局 id 寻址用）。
    ///
    /// 段数与每次调用的 `worker_count+1` 相同；段过小（如 NPC<64）时并行收益有限，
    /// 调用方应按阈值自行选择串行路径（见 game.rs `PARALLEL_AI_MIN`）。
    pub fn par_for_each_mut<T, F>(&self, data: &mut [T], f: F)
    where
        T: Send,
        F: Fn(usize, usize, &mut [T]) + Sync,
    {
        let n = data.len();
        if n == 0 {
            return;
        }
        let nw = self.worker_count + 1;
        // 均匀段边界：bounds[0]=0 .. bounds[nw]=n（首段归调用线程）
        let mut bounds = Vec::with_capacity(nw + 1);
        for i in 0..=nw {
            bounds.push((n * i) / nw);
        }
        let (head, tail) = data.split_at_mut(bounds[1]);
        f(0, 0, head);
        if tail.is_empty() {
            return;
        }
        // 池作业闭包要求 'static：调用方数据/闭包/计数器都以裸指针传入（无生命周期），
        // 安全性由「join 后才返回」保证——与 thread::scope 同款生命周期论证。
        let ptr = SendPtr(tail.as_mut_ptr() as *mut u8);
        let off0 = bounds[1];
        let done = AtomicUsize::new(nw - 1);
        let done_ptr = SendPtr(&done as *const AtomicUsize as *mut AtomicUsize);
        // F/T 一律抹成 *mut u8 捕获（u8 恒 'static），作业体内再 cast 回具体类型，
        // 从而不要求调用方闭包/元素类型带 'static 约束（调用方闭包可借局部上下文）。
        let fptr: SendPtr<u8> = SendPtr(&f as *const F as *const u8 as *mut u8);
        for w in 1..nw {
            let (start, end) = (bounds[w], bounds[w + 1]);
            let send_result = self.senders[w - 1].send(PoolMsg::Run(Box::new(move || {
                // SAFETY:
                // - ptr 指向 tail 的可变缓冲区，[start-off0, end-off0) 各段互不相交；
                // - fptr 指向调用方闭包 f，done 计数器归零（AcqRel 同步）后调用方才返回，
                //   因此任务执行期间 f 必然存活（同 thread::scope 生命周期论证）；
                // - done_ptr 指向调用方栈上的计数器，join 后才被重新读取。
                let seg = unsafe {
                    std::slice::from_raw_parts_mut(
                        ptr.get().add((start - off0) * std::mem::size_of::<T>()) as *mut T,
                        end - start,
                    )
                };
                let fref: &F = unsafe { &*(fptr.get() as *const F) };
                fref(w, start, seg);
                unsafe {
                    (*done_ptr.get()).fetch_sub(1, Ordering::AcqRel);
                }
            })));
            if send_result.is_err() {
                // 工作线程未存活（spawn 失败降级）：自行补计，避免死等
                unsafe {
                    (*done_ptr.get()).fetch_sub(1, Ordering::AcqRel);
                }
            }
        }
        while done.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
    }

    /// 在池的亲和核上同步执行单个任务并取回结果（线程优化第 3 步：低频重计算换核，
    /// 如地图/地形生成，避免占用主线程所在簇）。`f` 被 move 进工作线程执行，返回前
    /// 已 join（join 语义）。`f` 需 'static（捕获值语义，如地图种子）；spawn 失败降级
    /// 时回退调用线程执行，不 panic。
    pub fn run_sync<R: Send + 'static>(&self, f: impl FnOnce() -> R + Send + 'static) -> R {
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(f)));
        let (tx, rx) = std::sync::mpsc::channel::<R>();
        let job = PoolMsg::Run(Box::new({
            let slot = std::sync::Arc::clone(&slot);
            move || {
                let f = slot.lock().unwrap().take().expect("run_sync 任务只能执行一次");
                let _ = tx.send(f());
            }
        }));
        if self.senders[0].send(job).is_err() {
            // 工作线程未存活（spawn 失败降级）：调用线程直接执行
            let f = slot.lock().unwrap().take().expect("run_sync 任务只能执行一次");
            return f();
        }
        rx.recv().expect("run_sync 执行失败")
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 环境变量解析：工作线程数（非法/缺省回退 default，下限 1）
fn env_workers(key: &str, default: usize) -> usize {
    match std::env::var(key) {
        Ok(v) => v.parse::<usize>().unwrap_or(default).max(1),
        Err(_) => default,
    }
}

/// 全局场景计算池（渲染侧 CPU 工作：视锥剔除/实例上传/地形 morph）：
/// AMD = 首簇 CCD0（与主线程同簇）；Intel = 仅 P-core。
/// 线程数：`RV3D_SCENE_WORKERS` 覆盖，默认 min(8, 集合大小)。
static SCENE_POOL: OnceLock<ThreadPool> = OnceLock::new();

pub fn scene_pool() -> &'static ThreadPool {
    SCENE_POOL.get_or_init(|| {
        let topo = topology();
        let set = topo.scene_compute_set();
        let default = set.len().min(8).max(1);
        let n = env_workers("RV3D_SCENE_WORKERS", default);
        log::info!("cpu: scene_pool 创建（{} 工作线程，绑定 vCPU {:?}）", n, set);
        ThreadPool::new(set, n, "scene")
    })
}

/// 全局 AI 池（NPC 状态机/A* 等后台判定，远组/延迟不敏感负载）：
/// AMD = 次簇 CCD1；Intel = 有 E-core 即 E-core 组（远 AI 分层负载，见 `ai_set`）。
/// 线程数：`RV3D_AI_WORKERS` 覆盖，默认 min(8, 集合大小)。
static AI_POOL: OnceLock<ThreadPool> = OnceLock::new();

pub fn ai_pool() -> &'static ThreadPool {
    AI_POOL.get_or_init(|| {
        let topo = topology();
        let set = topo.ai_set();
        let default = set.len().min(8).max(1);
        let n = env_workers("RV3D_AI_WORKERS", default);
        log::info!("cpu: ai_pool 创建（{} 工作线程，绑定 vCPU {:?}）", n, set);
        ThreadPool::new(set, n, "ai")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_list_supports_ranges_and_lists() {
        assert_eq!(parse_cpu_list("0-7").unwrap(), (0..=7).collect::<Vec<_>>());
        assert_eq!(parse_cpu_list("0,2,4").unwrap(), vec![0, 2, 4]);
        assert_eq!(
            parse_cpu_list("0-3,16-19").unwrap(),
            vec![0, 1, 2, 3, 16, 17, 18, 19]
        );
        assert_eq!(parse_cpu_list("0").unwrap(), vec![0]);
        assert!(parse_cpu_list("x").is_none());
        assert!(parse_cpu_list("5-1").is_none());
    }

    #[test]
    fn split_smt_pairs_assigns_min_vcpu_as_physical() {
        // 模拟 8940HX WSL2：SMT 配对 0-1/2-3/…，偶数 vCPU 是物理主线程
        let groups = vec![
            vec![0, 1],
            vec![2, 3],
            vec![4],      // 无 SMT 的核心（如 Intel E-core）：自身即物理核
            vec![6, 7],
        ];
        let (physical, smt) = split_smt_pairs(&groups);
        assert_eq!(physical, vec![0, 2, 4, 6]);
        assert_eq!(smt, vec![1, 3, 7]);
    }

    #[test]
    fn split_smt_pairs_handles_empty_and_single() {
        let (physical, smt) = split_smt_pairs(&[]);
        assert!(physical.is_empty() && smt.is_empty());
        let (physical, smt) = split_smt_pairs(&[vec![5], vec![8, 9, 10]]);
        assert_eq!(physical, vec![5, 8]);
        assert_eq!(smt, vec![9, 10]);
    }

    #[test]
    fn thread_siblings_spec_parses_like_cpu_list() {
        // thread_siblings_list 常见格式 "0-1" / "0,1"，应能被 parse_cpu_list 解析
        assert_eq!(parse_cpu_list("0-1").unwrap(), vec![0, 1]);
        assert_eq!(parse_cpu_list("16-17").unwrap(), vec![16, 17]);
    }
}
