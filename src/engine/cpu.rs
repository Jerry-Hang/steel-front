//! CPU 拓扑检测与线程亲和性（零第三方依赖：sysfs + CPUID + `sched_setaffinity` FFI）
//!
//! 目标（用户策略）：
//! - AMD 双 CCD：主线程（游戏逻辑/物理/渲染提交）绑首簇（vCPU 前半 = CCD0，默认频率偏高），
//!   次簇（后半 = CCD1）专供 AI/地图生成等后台线程（`ai_pool`），杜绝跨 CCD 访问。
//! - Intel 混合架构：主线程绑 P-core 组；渲染侧场景计算（剔除/上传/地形 morph）只用
//!   P-core（`scene_pool`，绝不丢到 E-core）；E-core ≤8 只接音频等轻任务，>8 时
//!   E-core 组承担 AI 判定/部分地图生成（`ai_set` 决策）。
//! - 渲染线程不固定到 1-2 个核：主线程与 `scene_pool` 绑定的是「整簇集合」，
//!   由 OS 调度器把渲染工作分给集合内空闲率最高的核（多核共同承担线程渲染）。
//! - AVX-512：Zen4/Zen5（7000/9000 系）原生支持，运行时检测后由 renderer 选路启用。
//!
//! 注意：WSL2 虚拟化会抹平 L3/NUMA 分组（sysfs L3 `shared_cpu_list` 全 0-31、仅 node0），
//! 因此双簇推断采用「vCPU 枚举顺序 = 物理枚举顺序」：前半 = 首簇、后半 = 次簇，
//! 可用环境变量 `RV3D_CPU_PIN` 覆盖精确亲和性掩码（如 `RV3D_CPU_PIN=0-7,16-23`）。

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
    /// Intel 能效核数量（CPUID leaf 0x1A hybrid；AMD 恒 0）
    pub e_cores: usize,
    pub avx2: bool,
    pub avx512: bool,
}

/// 全局拓扑缓存：`topology()` 首次调用检测一次，后续零成本取用（避免 Game/Renderer 重复探测）。
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

impl CpuTopology {
    /// 场景计算集合（视锥剔除/实例上传/地形 morph 等渲染侧 CPU 工作）：
    /// AMD = 首簇 CCD0（与渲染主线程同簇，避免跨 CCD 访问）；
    /// Intel = 仅 P-core（杜绝渲染工作被调度到 E-core）。
    pub fn scene_compute_set(&self) -> &[usize] {
        &self.primary_set
    }

    /// AI/地图生成集合（用户策略）：
    /// - AMD = 次簇 CCD1（与主线程所在 CCD0 分离，AI 并行不吃主线程带宽）；
    /// - Intel = E-core ≥8 时交 E-core 组，否则回退 P-core（E-core 少时 AI 太重，
    ///   P-core 保证 AI 判定延迟，且绝不把渲染侧工作挤到 E-core）。
    pub fn ai_set(&self) -> &[usize] {
        if self.vendor == CpuVendor::Intel && self.e_cores >= 8 {
            &self.secondary_set
        } else if self.vendor == CpuVendor::Amd && !self.secondary_set.is_empty() {
            &self.secondary_set
        } else {
            &self.primary_set
        }
    }

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
                    // 非 x86_64/Linux（Apple Silicon、macOS、ARM 服务器）：
                    // 无 CPUID leaf 0x1A 与 sched_setaffinity，无 E-core 概念，全部归 primary 集合
                    ((0..threads).collect(), Vec::new(), 0)
                }
            }
            CpuVendor::Other => ((0..threads).collect(), Vec::new(), 0),
        };
        CpuTopology {
            vendor,
            threads,
            primary_set,
            secondary_set,
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
        }
        #[cfg(not(target_os = "linux"))]
        {
            // 非 Linux（如未来 macOS/iOS Apple Silicon 构建）：无 sched_setaffinity，
            // 不手工绑核，线程调度交给系统 QoS/调度器
            log::info!("cpu: 非 Linux 平台，跳过线程手工绑定（交给系统调度）");
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
        #[cfg(not(target_os = "linux"))]
        {
            let _ = set;
            false
        }
    }

    /// 启动日志摘要（厂商/簇/E-core/指令集）
    pub fn log_summary(&self) {
        log::info!(
            "cpu: vendor={:?} threads={} primary={:?} secondary={:?} e_cores={} avx2={} avx512={} scene_set={:?} ai_set={:?}",
            self.vendor,
            self.threads,
            self.primary_set,
            self.secondary_set,
            self.e_cores,
            self.avx2,
            self.avx512,
            self.scene_compute_set(),
            self.ai_set()
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
        for i in 0..threads {
            let (tx, rx) = std::sync::mpsc::channel::<PoolMsg>();
            senders.push(tx);
            let cpus = set_v.clone();
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

/// 全局 AI 池（NPC 状态机/A* 等后台判定）：
/// AMD = 次簇 CCD1；Intel = E-core ≥8 时 E-core 组，否则 P-core。
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
