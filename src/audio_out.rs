//! 真实声音输出后端（零第三方依赖：Windows = winmm waveOut 直接 FFI，
//! Linux = 运行时 dlopen `libasound.so.2` 的 ALSA FFI）。
//!
//! AudioPlayer（audio.rs）一直在混音合成（枪声/爆炸/脚步/环境音乐），
//! 但游戏默认挂了 SilentSink（静默占位）——2026-08-22 用户反馈"进游戏一点声音都没有"。
//!
//! # WaveOutSink（Windows）
//!
//! 16-bit PCM 交错样本 → waveOut 环形缓冲队列 → 声卡。
//! 结构：4 块 2048 帧双声道缓冲（48kHz 下 4×2048/48000 = **170ms** 队列 —— 旧注释写"~85ms"是
//! 按 2 块算的，已更正）。主线程每帧 tick 写入小块样本
//! （350FPS 时 ~137 帧/帧），回调线程完成 buffer 后归还空闲槽；free 列表用
//! Arc<Mutex<Vec<usize>>> 保护（回调与主线程竞争）。
//!
//! # AlsaSink（Linux，2026-09-28 加）
//!
//! 混音器给的**交错 f32 直接进 PCM**（`SND_PCM_FORMAT_FLOAT_LE`）—— 不像 waveOut 那样自己
//! 转 16-bit（那条路是非转不可：waveOut 只吃整数）。设备侧的环形缓冲由 alsa-lib 管，
//! 本模块**不自己排缓冲**（waveOut 那边不得不自己管，因为它的队列语义就是"若干块缓冲轮转"）。
//! 队列目标与 waveOut 对齐（`BUFFER_COUNT × FRAMES_PER_BUFFER` = 8192 帧 ≈ 170ms @48kHz），
//! 用 `snd_pcm_set_params` 的 `latency` 参数表达 —— 它的单位是**微秒**，所以按采样率现算
//! （见 `alsa::queue_latency_us`，写死 48000 会在 44.1kHz 的机器上排成 185ms）。
//!
//! ## 为什么用 dlopen/dlsym，而不是 `#[link(name = "asound")]`
//!
//! 硬链接会把"这台机器有没有 ALSA"变成**加载期**的硬依赖：没装 libasound 的机器（容器、
//! 最小化发行版、只用来编译的构建机）连进程都起不来 —— 而本仓的原则是 `waveOutOpen` 失败就
//! 降级为静默：**没有声音可以接受，起不来不可以**。dlopen 把"缺库"从"加载失败"降级成一条
//! `log::warn!`。`snd_*` 符号也逐个 `dlsym`（缺任一个 ⇒ 整套后端不可用 ⇒ 静默降级），
//! 于是也不会出现"库在、但比我们假设的旧/被裁剪过"时半死不活的状态。
//! glibc ≥ 2.34 已把 dlopen/dlsym 并进 libc；更老的 glibc 上它们在 libdl，而 Rust std 的
//! linux-gnu 目标默认就链了 `-ldl` ⇒ 声明 `extern "C" { fn dlopen(..); fn dlsym(..); }`
//! 不需要任何额外 link 参数，`Cargo.toml` 一个字都不用改。
//!
//! **刻意不 `dlclose`**：`snd_pcm_t` 是 alsa-lib 的堆对象，库里还有配置树/插件单例/内部线程等
//! 全局状态；卸载一个可能仍持有活对象的库，等于让之后任何一次调用跳进已卸载的代码。
//! dlopen 的引用计数只增不减，库留在地址空间里到进程结束 —— 这与 waveOut 那边
//! "`waveOutClose` 失败时宁可刻意泄漏也不留 use-after-free"是同一个取舍。
//!
//! ## 失败如何降级（两条后端同一套语义）
//!
//! 打开失败（没装库 / 缺符号 / `snd_pcm_open` 失败 / `snd_pcm_set_params` 失败 / `waveOutOpen`
//! 失败）⇒ `silenced: true` 的内部静默模式：`write` 变成空操作，游戏照常跑，只在日志里留一条
//! 含 alsa 错误码原文的 warn。**绝不 panic、绝不阻止游戏启动**（降级判定抽成纯函数
//! `alsa::sink_from_open`，这样在没有声卡的机器上也能测）。
//! 运行中写不进去（队列满 = `-EAGAIN` / waveOut 无空闲块）⇒ 丢弃本帧样本（实时音频不补播旧数据），
//! 但**丢弃不静默**：一次性告警，见 `first_starved_drop`。

// 🔴 2026-09-26：这两个导入只有 Windows 那条路（`mod win` 的回调上下文）用得到，
// 非 Windows 下 `SilentSink` 不需要它们 ⇒ 原来在 aarch64-linux 上是一条 unused-import 警告
// （而本仓要求 0 警告，交叉验证那条命令以前根本跑不起来，见 engine/mod.rs 的同类修复）。
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};

/// waveOut 的块大小；ALSA 侧只用它算队列目标（`BUFFER_COUNT × 这个数` 帧）。
const FRAMES_PER_BUFFER: usize = 2048;
/// waveOut 的块数；ALSA 侧只用它算队列目标（见 `alsa::queue_latency_us`）。
const BUFFER_COUNT: usize = 4;

#[cfg(target_os = "windows")]
mod win {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WaveFormatEx {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WaveHdr {
        pub lp_data: *mut u8,
        pub dw_buffer_length: u32,
        pub dw_bytes_recorded: u32,
        pub dw_user: usize,
        pub dw_flags: u32,
        pub dw_loops: u32,
        pub lp_next: *mut u8,
        pub reserved: usize,
    }

    const WAVE_FORMAT_PCM: u16 = 0x0001;
    const WAVE_MAPPER: u32 = 0xFFFF_FFFF;
    const CALLBACK_FUNCTION: u32 = 0x0003_0000;
    const WOM_DONE: u32 = 0x0003;

    #[link(name = "winmm")]
    extern "system" {
        pub fn waveOutOpen(
            phwo: *mut usize,
            u_device_id: u32,
            pwfx: *const WaveFormatEx,
            dw_callback: usize,
            dw_instance: usize,
            fdw_open: u32,
        ) -> u32;
        pub fn waveOutPrepareHeader(hwo: usize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        pub fn waveOutWrite(hwo: usize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        pub fn waveOutUnprepareHeader(hwo: usize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        pub fn waveOutClose(hwo: usize) -> u32;
        pub fn waveOutReset(hwo: usize) -> u32;
    }

    pub struct CallbackCtx {
        pub free: Mutex<Vec<usize>>,
    }

    extern "system" fn wave_callback(
        _hwo: usize,
        msg: u32,
        dw_user: usize,
        dw1: usize,
        _dw2: usize,
    ) {
        if msg != WOM_DONE {
            return;
        }
        let ctx = unsafe { &*(dw_user as *const CallbackCtx) };
        // dw1 = 完成的 WAVEHDR 指针（非缓冲索引！）：从 hdr.dw_user 取回真实索引
        // （2026-08-23 修复：此前直接 push dw1 → 首块播完（~85ms）即索引越界 panic）
        let idx = unsafe { (*(dw1 as *const WaveHdr)).dw_user };
        if let Ok(mut free) = ctx.free.lock() {
            free.push(idx);
        }
    }

    pub struct WaveBuffer {
        pub hdr: WaveHdr,
        pub data: Vec<u8>,
    }

    impl WaveBuffer {
        fn new() -> Self {
            let mut data = vec![0u8; FRAMES_PER_BUFFER * 2 * 2];
            let hdr = WaveHdr {
                lp_data: data.as_mut_ptr(),
                dw_buffer_length: (data.len() as u32).min(0x7FFF_FF00),
                dw_bytes_recorded: 0,
                dw_user: 0,
                dw_flags: 0,
                dw_loops: 0,
                lp_next: std::ptr::null_mut(),
                reserved: 0,
            };
            WaveBuffer { hdr, data }
        }
    }

    pub fn open(
        sample_rate: u32,
        channels: u16,
    ) -> Result<(usize, Arc<CallbackCtx>, Vec<WaveBuffer>), String> {
        let fmt = WaveFormatEx {
            w_format_tag: WAVE_FORMAT_PCM,
            n_channels: channels,
            n_samples_per_sec: sample_rate,
            n_avg_bytes_per_sec: sample_rate * channels as u32 * 2,
            n_block_align: channels * 2,
            w_bits_per_sample: 16,
            cb_size: 0,
        };
        let ctx = Arc::new(CallbackCtx {
            free: Mutex::new((0..BUFFER_COUNT).collect()),
        });
        let ctx_ptr = Arc::as_ptr(&ctx) as usize;
        let mut handle: usize = 0;
        let rc = unsafe {
            waveOutOpen(
                &mut handle,
                WAVE_MAPPER,
                &fmt,
                wave_callback as *const () as usize,
                ctx_ptr,
                CALLBACK_FUNCTION,
            )
        };
        if rc != 0 {
            return Err(format!("waveOutOpen 失败 rc={}", rc));
        }
        let mut buffers = Vec::with_capacity(BUFFER_COUNT);
        for _ in 0..BUFFER_COUNT {
            buffers.push(WaveBuffer::new());
        }
        // 🔴 2026-09-23 复查：**prepare 必须在最终地址上做**。
        // 驱动会在 prepare 时把 WAVEHDR 的地址记进它自己的表（`waveOutWrite` 收的是这个地址，
        // 回调的 `dwParam1` 也是它），而 WAVEHDR 里还有 `reserved` 是"驱动内部使用、应用不得改"的。
        // 旧写法先在**栈上临时量**prepare、再 `buffers.push(b)` 搬家 ⇒ 准备的地址 ≠ 使用的地址。
        // 现在先收齐（`Vec` 容量一次给足，此后堆区不再变动）再逐个 prepare。
        for i in 0..buffers.len() {
            let rc = unsafe {
                waveOutPrepareHeader(
                    handle,
                    &mut buffers[i].hdr,
                    std::mem::size_of::<WaveHdr>() as u32,
                )
            };
            if rc != 0 {
                // 失败路径收尾：已 prepare 的头要 unprepare，设备句柄要 close。
                // （旧写法直接 `return Err` ⇒ 句柄与已准备的缓冲全部泄漏，且设备一直开着。）
                unsafe {
                    for b in buffers.iter_mut().take(i) {
                        waveOutUnprepareHeader(
                            handle,
                            &mut b.hdr as *mut _,
                            std::mem::size_of::<WaveHdr>() as u32,
                        );
                    }
                    waveOutClose(handle);
                }
                return Err(format!("waveOutPrepareHeader 失败 rc={}", rc));
            }
        }
        Ok((handle, ctx, buffers))
    }
}

/// Linux 的 ALSA 绑定：运行时 `dlopen` + 逐个 `dlsym`（理由见文件头），外加这一层需要的
/// **纯函数**（参数换算/帧换算/降级映射/越界处理）—— 它们不碰声卡，所以能直接在单测里钉死。
#[cfg(target_os = "linux")]
mod alsa {
    use super::{BUFFER_COUNT, FRAMES_PER_BUFFER};
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    use std::sync::OnceLock;

    // --------------------------------------------------------------- ABI 常量
    //
    // 🔴 FFI 里写错枚举值是**静默**的：往一个未定义的格式写不会报错，只表现为"没声音"
    // 或者"沙沙声"。所以下面每个值都注明它在头文件里的定义位置，并且都用一个 C 探针
    // （`gcc` 编一段 `printf("%d", SND_PCM_FORMAT_FLOAT_LE)`）**逐个核对过** ——
    // 2026-09-28，本机 alsa-lib 1.2.16.1。单测里另有字面量断言（`pcm_params_match_the_verified_abi_values`），
    // 改这里必须同时改那里，改不动就说明你在改一个不报错的错。

    /// `pcm.h:107`：`SND_PCM_STREAM_PLAYBACK = 0`（枚举首项，无显式值）
    const SND_PCM_STREAM_PLAYBACK: c_int = 0;
    /// `pcm.h:122`：`SND_PCM_ACCESS_RW_INTERLEAVED = 3`（`MMAP_INTERLEAVED=0` 之后的第 4 项）
    const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
    /// `pcm.h:161`：`SND_PCM_FORMAT_FLOAT_LE = 14`
    /// （从 `UNKNOWN=-1` 起算：S8=0 … S32_LE=10、S32_BE=11、U32_LE=12、U32_BE=13 ⇒ FLOAT_LE=14。
    /// 猜成 13 或 10 都会"能打开、能写、就是没声音/出噪声"。）
    const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;
    /// `pcm.h:405`：`#define SND_PCM_NONBLOCK 0x00000001`（`snd_pcm_open` 的 mode 标志）
    const SND_PCM_NONBLOCK: c_int = 0x0000_0001;
    /// `bits/dlfcn.h:25`：`#define RTLD_NOW 0x2`。用 NOW 而不是 LAZY：缺符号要在 dlopen 当场
    /// 暴露，而不是拖到第一次 `write` 时在游戏线程里炸。
    const RTLD_NOW: c_int = 0x2;
    /// `asm-generic/errno-base.h:15`：`EAGAIN = 11`（Linux 上 == `EWOULDBLOCK`）。
    /// 这些值**不能**用 `std::io::Error::last_os_error()` 反推：`snd_pcm_writei` 的负返回值是
    /// 函数自己返回的错误码，**没有**经过 `errno`（alsa-lib 内部不保证把它留在那里）。
    /// alpha/mips/parisc 另有一套 errno 编号，本仓的 Linux 目标只有 x86_64/aarch64（都用这套）。
    const EAGAIN: c_int = 11;
    /// `asm-generic/errno-base.h:36`：`EPIPE = 32` —— 播放流的 underrun（队列被播空了）
    const EPIPE: c_int = 32;
    /// `asm-generic/errno.h:70`：`ESTRPIPE = 86` —— 流被挂起（整机挂起、音频服务重启）
    const ESTRPIPE: c_int = 86;

    // --------------------------------------------------------------- dlopen 绑定

    extern "C" {
        /// `dlfcn.h:56`：`void *dlopen(const char *file, int mode)`
        fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
        /// `dlfcn.h:64`：`void *dlsym(void *handle, const char *name)`
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// 动态库的 SONAME。**必须带 `.so.2`**：`libasound.so` 那个名字只有装了 `-dev`/`-devel`
    /// 包才有（它是指向 `.so.2` 的符号链接），运行时机器上通常只有 `.so.2`。名字写错的代价是
    /// "永远静默"，而且没有任何编译期提示。
    const LIBASOUND: &[u8] = b"libasound.so.2\0";

    /// 依次尝试的 PCM 名字（NUL 结尾的静态字节串：不过 `CString` ⇒ 没有分配，
    /// 也没有"名字里有内嵌 NUL"这条不可能发生的错误分支）。
    ///
    /// - `default`：alsa.conf 里那条**用户可覆盖**的缺省路由，PipeWire/PulseAudio/dmix 都靠它接管，
    ///   桌面机上**通常**存在 ⇒ 排第一，用户自己配的重定向优先。
    /// - `pipewire`：PipeWire 的 ALSA 插件（`/usr/share/alsa/alsa.conf.d/50-pipewire.conf`
    ///   定义的 `pcm.pipewire`）。
    ///   🔴 **2026-09-28 真机跑出来才补的这一项**：本机（Arch + PipeWire）`default` 解析到
    ///   **dmix** 并报 `snd_pcm_dmix_open: unable to open slave` ⇒ `snd_pcm_open` 返回 `-ENOENT`。
    ///   根因是**系统侧**缺 `99-pipewire-default.conf`（那一条才会把 `!default` 指到 pipewire），
    ///   实测 `aplay -D pipewire` 退出码 0 能出声、`aplay -D default` 失败。
    ///   ⇒ 引擎**不能要求每个用户的 ALSA 配置都完整**，这个坑得自己兜住。
    /// - `sysdefault`：前两个都没被定义时的兜底（最小化发行版/容器）。
    ///
    /// 三个都失败才降级 —— 多试一个名字不会让任何机器变差（降级本身就是静默的），
    /// 而少试一个就会让"配置差一点的机器永远没声音"。成功时**把用的哪个名字写进日志**。
    const PCM_NAMES: [&[u8]; 3] = [b"default\0", b"pipewire\0", b"sysdefault\0"];

    /// dlopen 出来的函数指针表。
    ///
    /// **存函数指针而不是每次 `dlsym`**：`dlsym` 要在动态链接器的全局符号表里按名字查（还加锁），
    /// 而 `writei` 是每帧都要调的。表本身活在 `API` 那个静态里、库从不卸载 ⇒ 这里的
    /// `'static` 引用不是"假装"，是真的活到进程结束。
    pub struct Api {
        /// `pcm.h:528`：`int snd_pcm_open(snd_pcm_t **pcm, const char *name, snd_pcm_stream_t stream, int mode)`
        pub open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int,
        /// `pcm.h:537`：`int snd_pcm_close(snd_pcm_t *pcm)`
        pub close: unsafe extern "C" fn(*mut c_void) -> c_int,
        /// `pcm.h:681`：`int snd_pcm_set_params(snd_pcm_t*, snd_pcm_format_t, snd_pcm_access_t,`
        /// `unsigned int channels, unsigned int rate, int soft_resample, unsigned int latency)`
        /// —— 最后一个参数的单位是**微秒**，不是帧（见 `queue_latency_us`）。
        pub set_params: unsafe extern "C" fn(
            *mut c_void,
            c_int,
            c_int,
            c_uint,
            c_uint,
            c_int,
            c_uint,
        ) -> c_int,
        /// `pcm.h:574`：`snd_pcm_sframes_t snd_pcm_writei(snd_pcm_t*, const void *buffer, snd_pcm_uframes_t size)`
        /// —— 长度单位是**帧**而不是样本（见 `frames_from_samples`），返回实际写入的帧数或负错误码。
        /// 长度这里用 `c_ulong`，对应头文件的 `typedef unsigned long snd_pcm_uframes_t`（`pcm.h:400`）：
        /// 32 位 Linux 上 `unsigned long` 是 32 位，写死 `u64` 就是**参数错位**（返回值同理，
        /// `typedef long snd_pcm_sframes_t`，`pcm.h:402`）。
        pub writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long,
        /// `pcm.h:680`：`int snd_pcm_recover(snd_pcm_t *pcm, int err, int silent)`
        pub recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
        /// `pcm.h:560`：`int snd_pcm_drain(snd_pcm_t *pcm)`
        pub drain: unsafe extern "C" fn(*mut c_void) -> c_int,
        /// `error.h:50`：`const char *snd_strerror(int errnum)` —— 只用来把错误码翻成人话写进日志
        /// （用户报"没声音"时，日志里 `rc=-2` 和 `rc=-2 (No such file or directory)` 的价值差很多）。
        pub strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    }

    /// 进程级单例。用 `OnceLock`（std）而不是手写 `static mut`：多线程下"只初始化一次"是有保证的。
    /// `Option` 也要缓存 —— 否则"本机没有 ALSA"这件事会变成每次 open 都重新 dlopen 一遍。
    static API: OnceLock<Option<Api>> = OnceLock::new();

    /// 拿函数指针表；`None` = 本机没有可用的 ALSA（调用方一律降级为静默，**不 panic**）。
    pub fn api() -> Option<&'static Api> {
        API.get_or_init(load).as_ref()
    }

    /// `dlopen` + 7 个 `dlsym`。任何一个符号缺失都返回 `None`（= 整套后端不可用）。
    fn load() -> Option<Api> {
        // SAFETY: 参数是 NUL 结尾的静态字节串；`RTLD_NOW` 只影响绑定时机。
        // dlopen 失败返回空指针（详细原因要用 `dlerror` 取，我们**刻意不调它**：
        // "哪台机器没装 ALSA"从返回值就能判定，缺库的细节交给上层那句 warn）。
        let handle = unsafe { dlopen(LIBASOUND.as_ptr() as *const c_char, RTLD_NOW) };
        if handle.is_null() {
            return None;
        }
        // 🔴 这里**刻意不 dlclose**（连失败路径也不）：`handle` 之后的每一次 `dlsym`、以及
        // 由它创建的所有 `snd_pcm_t`，都要靠这个 handle 活着；而 `snd_pcm_t` 的寿命跟着
        // `AlsaSink`（可能到进程结束）。见文件头"刻意不 dlclose"。
        // SAFETY: `handle` 刚由成功的 dlopen 取得、且永不 dlclose；每个符号名都是 NUL 结尾的
        // 字面量；`sym` 的返回类型由字段类型（函数指针）决定，尺寸由 `sym` 内部断言兜住。
        unsafe {
            Some(Api {
                open: sym(handle, b"snd_pcm_open\0")?,
                close: sym(handle, b"snd_pcm_close\0")?,
                set_params: sym(handle, b"snd_pcm_set_params\0")?,
                writei: sym(handle, b"snd_pcm_writei\0")?,
                recover: sym(handle, b"snd_pcm_recover\0")?,
                drain: sym(handle, b"snd_pcm_drain\0")?,
                strerror: sym(handle, b"snd_strerror\0")?,
            })
        }
    }

    /// 取一个符号并转成函数指针；`None` = 这个符号不存在（老版本/被裁剪的 alsa-lib）。
    ///
    /// # Safety
    /// 调用方必须保证 `handle` 来自成功的 `dlopen` 且**从未 `dlclose`**（见文件头）。
    /// `name` 必须是 NUL 结尾的 C 字符串（下面有 debug 断言），`T` 必须是函数指针类型。
    unsafe fn sym<T: Copy>(handle: *mut c_void, name: &[u8]) -> Option<T> {
        debug_assert_eq!(name.last(), Some(&0), "符号名必须是 NUL 结尾的 C 字符串");
        debug_assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*mut c_void>(),
            "只允许把 dlsym 的返回值转成函数指针"
        );
        let p = dlsym(handle, name.as_ptr() as *const c_char);
        if p.is_null() {
            return None;
        }
        // `transmute_copy` 而不是 `transmute`：`T` 是泛型，`transmute` 要求两端尺寸在编译期已知
        // 相等（泛型下直接 E0512 编译不过）。上面那条 debug_assert 就是它的兜底。
        Some(std::mem::transmute_copy::<*mut c_void, T>(&p))
    }

    /// 把 alsa 的负错误码翻成人话（拿不到就退回数字）。
    /// `snd_strerror` 返回库内静态字符串，这里**立刻**拷成 `String`：既不用管它的生命周期，
    /// 也不受"库会不会被卸载"影响。
    pub fn err_text(api: &Api, rc: c_int) -> String {
        if rc >= 0 {
            return format!("rc={}", rc);
        }
        let p = unsafe { (api.strerror)(rc) };
        if p.is_null() {
            return format!("rc={}", rc);
        }
        // SAFETY: `snd_strerror` 保证返回以 NUL 结尾的字符串（error.h:50）；
        // 库从不卸载 ⇒ 这个指针在拷贝完成前一直有效。
        let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
        format!("rc={} ({})", rc, s)
    }

    /// `snd_pcm_set_params` 的全部实参（句柄之外）。
    ///
    /// 抽成结构体 + 纯函数是为了**能测**：其中两个是枚举常量，写错它们不报错、只表现为
    /// "没声音"或"杂音"（见常量区的警告）。有了这个纯函数，"我们到底要求了 FLOAT_LE 还是 13"
    /// 就能在一台没有声卡的机器上被测试钉死。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PcmParams {
        /// `SND_PCM_FORMAT_FLOAT_LE`（`pcm.h:161` = 14）：混音器产出的就是交错 f32，
        /// 直接交给 alsa-lib 的插件链去转设备原生格式（通常是 S16）。**不自己转 16-bit**：
        /// waveOut 那条路必须转是因为 API 只吃整数，这里多一次全缓冲扫描换不来任何东西。
        pub format: c_int,
        /// `SND_PCM_ACCESS_RW_INTERLEAVED`（`pcm.h:122` = 3）：`snd_pcm_writei` 对应的访问方式，
        /// 与我们的交错缓冲一一对应。用 MMAP 那一套（0）就得自己维护环形指针 —— 没必要。
        pub access: c_int,
        /// 声道数。0 是无效请求，但**不在这里替调用方改成 1**：交给 `snd_pcm_set_params` 报错
        /// ⇒ 走静默降级。悄悄改成单声道会把"上层配置错了"变成一个听不出来、也查不出来的状态。
        pub channels: c_uint,
        /// 采样率（Hz），原样传给 alsa-lib（不在这里偷偷重采样成 48k）。
        pub rate: c_uint,
        /// 1 = 允许软件重采样（`snd_pcm_set_params` 的第 6 个参数）。传 0 时，遇到只支持
        /// 44.1kHz 的老设备会直接配置失败 ⇒ **整机静音**；传 1 由 alsa-lib 插一层重采样，
        /// 代价是一点点音质/CPU。换「游戏必须能出声」，这个交换明显划算。
        pub soft_resample: c_int,
        /// 队列目标延迟，单位**微秒**（`snd_pcm_set_params` 最后一个参数就是微秒）。
        /// 写成帧数（8192）会得到一个 8192 秒的队列 —— 然后 open 直接失败。
        pub latency_us: c_uint,
    }

    /// 队列目标微秒：与 waveOut 的 `BUFFER_COUNT × FRAMES_PER_BUFFER` 帧**同一口径**
    /// （48kHz 下 8192 帧 = 170.6ms），但必须**按采样率现算** —— 单位是微秒，写死 170666 会让
    /// 44.1kHz 的机器实际排 185ms（多 9%：枪声反馈更慢），96kHz 的机器只剩 85ms（更容易 underrun）。
    /// 判据：`queue_latency_tracks_sample_rate`。
    ///
    /// 采样率为 0 时**不许除**（会 panic）：返回 `u32::MAX`，让 `snd_pcm_set_params` 去拒绝。
    /// "纯函数对任何输入都不 panic"是本文件所有辅助函数的底线 —— 音频出问题绝不该把游戏带走。
    pub fn queue_latency_us(sample_rate: u32) -> c_uint {
        let frames = (BUFFER_COUNT * FRAMES_PER_BUFFER) as u64;
        let rate = sample_rate.max(1) as u64;
        // 先升到 u64：8192 × 1e6 = 8.192e9 已经超过 u32::MAX（4.29e9），在 u32 里乘会**静默回绕**
        // 成一个小延迟（然后听感上就是反复 underrun，而日志里什么都没写）。
        u32::try_from(frames * 1_000_000 / rate).unwrap_or(u32::MAX)
    }

    /// (采样率, 声道数) → `snd_pcm_set_params` 的 6 个实参（顺序与头文件一致）。
    pub fn pcm_params(sample_rate: u32, channels: u16) -> PcmParams {
        PcmParams {
            format: SND_PCM_FORMAT_FLOAT_LE,
            access: SND_PCM_ACCESS_RW_INTERLEAVED,
            channels: channels as c_uint,
            rate: sample_rate,
            soft_resample: 1,
            latency_us: queue_latency_us(sample_rate),
        }
    }

    /// 打开并配置 PCM。成功返回句柄，失败返回给人看的错误串（调用方只负责 warn + 静默）。
    pub fn open_pcm(sample_rate: u32, channels: u16) -> Result<*mut c_void, String> {
        let Some(api) = api() else {
            return Err(format!(
                "dlopen {} 失败或缺 snd_pcm_* 符号（本机没装 ALSA？）",
                String::from_utf8_lossy(LIBASOUND.strip_suffix(b"\0").unwrap_or(LIBASOUND))
            ));
        };
        let p = pcm_params(sample_rate, channels);
        let mut last = String::from("没有可用的 PCM 名字");
        for name in PCM_NAMES {
            let mut pcm: *mut c_void = std::ptr::null_mut();
            // SAFETY: `name` 是 NUL 结尾的静态字节串；`pcm` 是本地变量、生命周期足够；
            // 另两个参数是常量整数（stream=PLAYBACK、mode=NONBLOCK，见常量区）。
            let rc = unsafe {
                (api.open)(
                    &mut pcm,
                    name.as_ptr() as *const c_char,
                    SND_PCM_STREAM_PLAYBACK,
                    SND_PCM_NONBLOCK,
                )
            };
            if rc < 0 {
                last = format!(
                    "snd_pcm_open({}) 失败 {}",
                    String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name)),
                    err_text(api, rc)
                );
                continue;
            }
            // SAFETY: rc ≥ 0 ⇒ alsa-lib 已把 `pcm` 填成有效句柄（pcm.h:528 的 out 参数），
            // 其余是上一步算好的普通整数。
            let rc = unsafe {
                (api.set_params)(
                    pcm,
                    p.format,
                    p.access,
                    p.channels,
                    p.rate,
                    p.soft_resample,
                    p.latency_us,
                )
            };
            if rc < 0 {
                // 🔴 配置失败必须**当场关掉**再换名字重试：不关的话这块 PCM 已经把设备占住，
                // 下一次 open 可能拿到 -EBUSY，把"参数不支持"伪装成"设备被占用"。
                // SAFETY: `pcm` 是上面 open 成功给出的句柄，且只在这里关一次。
                unsafe {
                    (api.close)(pcm);
                }
                last = format!("snd_pcm_set_params 失败 {}", err_text(api, rc));
                continue;
            }
            // 成功即记录**用的哪个名字**：`default` / `pipewire` / `sysdefault` 三者在不同
            // 发行版上哪个能用是不一样的（本机是 `pipewire`），不记下来下次"没声音"又要重推。
            log::info!(
                "audio: ALSA PCM 设备 = {}",
                String::from_utf8_lossy(name.strip_suffix(b"\0").unwrap_or(name))
            );
            return Ok(pcm);
        }
        Err(last)
    }

    /// 交错样本数 → **帧**数。`snd_pcm_writei` 的长度参数是帧（一帧 = `channels` 个样本），
    /// 传样本数会让驱动去读 `channels` 倍的数据 —— **读越界**：不崩、不报错，读到的不是我们的
    /// 音频，听感就是爆音/杂音（正是本仓最贵的那类"静默"事故）。
    /// 声道数为 0 时返回 0 **而不是除零 panic**（0 帧的写入本来就是空操作）。
    /// 除不尽的尾样本丢弃（最多 `channels-1` 个，且绝不会把"半帧"交给驱动）。
    pub fn frames_from_samples(samples: usize, channels: u16) -> usize {
        if channels == 0 {
            return 0;
        }
        samples / channels as usize
    }

    /// 「打开结果 → (PCM 句柄, 是否静默)」：降级判定的**唯一**出口。
    ///
    /// 抽成纯函数是为了能在没有声卡、也不用 root 的机器上测"打开失败"这条路径 ——
    /// 真去 open 硬件是测不了的（CI/容器里必然失败，而失败恰恰是这里要覆盖的分支）。
    /// 语义只有一条：**任何失败都不许往上传**，一律映射成 `(None, true)`。
    pub fn sink_from_open(opened: Result<*mut c_void, String>) -> (Option<*mut c_void>, bool) {
        match opened {
            Ok(pcm) => (Some(pcm), false),
            Err(_) => (None, true),
        }
    }

    /// 真正要交给 `snd_pcm_writei` 的样本：全部在 [-1,1] 内且有限时**直接借用**调用方的切片
    /// （零拷贝，常态）；否则把处理过的副本写进 `scratch` 并借用它。
    ///
    /// 为什么必须处理（而不是"混音器会夹好"）：
    /// - 混音总线是多个声部**直接相加**（枪声叠爆炸、再叠音乐），越界是常态不是异常；而
    ///   float→整数 的转换在越界时是**未定义/实现相关**的（x86 上 `cvttss2si` 给 0x8000
    ///   ⇒ 满量程反向爆音）。走哪条插件链（plug/dmix/PipeWire）决定了谁来做这次转换，
    ///   也就决定了它饱和还是截断 —— 不能把"越界没事"当前提。
    /// - NaN/Inf 同理：waveOut 那条路 `(s * 32767.0) as i16` 会把 NaN 变成 0（Rust 的
    ///   float→int `as` 是饱和转换），这里显式映射成 0.0，保持两条后端同语义。
    ///
    /// 夹紧**不改变长度** ⇒ 调用方按原长度算出的帧数依然对得上。
    pub fn write_slice<'a>(samples: &'a [f32], scratch: &'a mut Vec<f32>) -> &'a [f32] {
        if samples.iter().all(|s| s.is_finite() && *s >= -1.0 && *s <= 1.0) {
            return samples;
        }
        scratch.clear();
        scratch.extend(
            samples
                .iter()
                .map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }),
        );
        scratch.as_slice()
    }

    // ------------------------------------------------- 给上层的语义化接口（FFI 只在这里出现）

    /// 不透明的 `snd_pcm_t *`：我们从不解引用它，只原样交回 alsa-lib。
    /// 起个别名是为了让上层（`AlsaSink`）**一个 C 类型、一次指针转换都不用写** ——
    /// unsafe 与类型转换全部关在这个模块里，复核的时候只看这一处。
    pub type Pcm = *mut c_void;

    /// 写 `frames` 帧交错样本。返回值语义与 `snd_pcm_writei` 完全一致：
    /// 实际写进去的帧数，或负错误码（`-EAGAIN`/`-EPIPE`/…）。
    pub fn write_frames(api: &Api, pcm: Pcm, data: &[f32], frames: usize) -> c_long {
        // SAFETY: `pcm` 必须是本进程用 `snd_pcm_open` 成功取得、尚未 `snd_pcm_close` 的句柄
        // （调用方 `AlsaSink` 保证：句柄是它的字段，只在 `drop` 里关一次）；
        // `data` 至少有 `frames × channels` 个 f32（帧数正是按 `data.len()` 与声道数算出来的），
        // 驱动按 `frames` 帧读取的正是这块内存，不会读过尾端。
        unsafe { (api.writei)(pcm, data.as_ptr() as *const c_void, frames as c_ulong) }
    }

    /// 这个负返回值是不是"流已经停了、需要 recover 后重试"：
    /// `-EPIPE` = underrun（设备把队列播空了）、`-ESTRPIPE` = 流被挂起（挂起/音频服务重启）。
    pub fn needs_recover(n: c_long) -> bool {
        n == -(EPIPE as c_long) || n == -(ESTRPIPE as c_long)
    }

    /// 这个负返回值是不是"队列一点空间都没有"（本帧该丢）。
    pub fn queue_full(n: c_long) -> bool {
        n == -(EAGAIN as c_long)
    }

    /// `snd_pcm_recover`：把上一步的错误码交给 alsa-lib，让它 prepare（必要时 resume）后把流
    /// 恢复到可写。返回 0 = 可以重试一次，< 0 = 救不回来。
    /// `silent=1` 抑制 alsa-lib 自己往 stderr 打的那行 —— 成功恢复不是错误路径，而真正救不回来的
    /// 情况由 `warn_write_error` 留痕（一份日志里不需要两处说同一件事）。
    pub fn recover(api: &Api, pcm: Pcm, err: c_long) -> c_long {
        // SAFETY: 同 `write_frames`（句柄有效）；`err` 就是刚从前一次 writei 拿到的负返回值。
        unsafe { (api.recover)(pcm, err as c_int, 1) as c_long }
    }

    /// 收尾：drain（给队列一个播完的机会，理由见 `AlsaSink` 的 `drop`）+ close。
    /// 返回 close 的结果：0 = 关干净了，< 0 = 没关干净（调用方只记日志，**绝不因此 dlclose**）。
    pub fn finish(api: &Api, pcm: Pcm) -> c_long {
        // SAFETY: `pcm` 是有效句柄，且调用方保证这是**最后一次**使用它（`AlsaSink::drop` 里已把
        // 它从字段取走、之后不会再传给 submit）⇒ drain/close 各调一次，之后不再触碰。
        unsafe {
            (api.drain)(pcm);
            (api.close)(pcm) as c_long
        }
    }

    /// 「`writei`/`recover` 失败」的一次性告警闩（不刷屏）。
    ///
    /// 与 `first_starved_drop` **分开**：那个闩报的是队列满（已知、可预期），这里报的是意外错误
    /// （recover 都救不回来，或没见过的错误码）。两者混用一个闩的话，一次队列满就会把后面真正的
    /// 故障一起吞掉 —— 那正是"告警闩反而让人以为天下太平"的老问题。
    pub fn warn_write_error(err: c_long) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            // 错误码翻成人话。走到这一步说明 open 成功过 ⇒ 表必然在；`None` 只是防御分支。
            let text = match api() {
                Some(a) => err_text(a, err as c_int),
                None => format!("rc={}", err),
            };
            log::warn!(
                "audio: ALSA 写入失败 {}（recover 也失败，或属未知错误码），本帧丢弃；\
                 设备可能已经消失（USB 声卡被拿掉？），后续同样情况不再提示",
                text
            );
        }
    }

    /// ALSA 侧的判据：**全是纯函数**，不碰声卡、不需要 root、任何机器上都能跑
    /// （真去 open 硬件是测不了的：CI/容器里必然失败，而"失败"恰恰是要覆盖的分支之一）。
    ///
    /// 仓库铁律「凡声称支持，都要补一条会红的测试」在这里针对的是一类特别的缺陷：
    /// **写错不报错**的东西 —— 枚举常量（写到未定义格式 = 没声音/噪声）、微秒换算、
    /// 帧/样本换算（写错 = 驱动读越界）、越界样本（不夹 = 反向爆音）、降级映射。
    /// 每条断言都写明"改错了会怎样红"。
    #[cfg(test)]
    mod tests {
        use super::*;

        /// 🔴 队列长度必须**跟着采样率走**。
        /// 反例（这条测试就是为它写的）：把 48kHz 算出来的 170666µs 写死 —— 44.1kHz 的机器上
        /// 真队列会变成 185ms（多 9%，枪声反馈更慢），96kHz 上只剩 85ms（一半，更容易 underrun）。
        #[test]
        fn queue_latency_tracks_sample_rate() {
            // 4×2048 = 8192 帧：8192/48000 s = 170.666…ms
            assert_eq!(queue_latency_us(48_000), 170_666);
            // 同一批帧在 44.1kHz 下是 185.759…ms（写死 170666 的话这一条就红）
            assert_eq!(queue_latency_us(44_100), 185_759);
            // 采样率翻倍 ⇒ 时间减半（允许 ±1µs 的取整误差）
            let (a, b) = (
                queue_latency_us(48_000) as i64,
                queue_latency_us(96_000) as i64,
            );
            assert!((a - 2 * b).abs() <= 1, "48k={} 96k={} 应当成 2:1", a, b);
            // 0Hz 是无效输入：不许 panic（那是除零），也不许在 u32 里回绕成一个小数字。
            assert_eq!(queue_latency_us(0), u32::MAX);
        }

        /// 🔴 `snd_pcm_set_params` 的实参（特别是两个**写错不报错**的枚举常量）。
        ///
        /// 期望值刻意写成**字面量**而不是引用上面的常量：引用常量等于自己证明自己（恒真断言，
        /// 教训 14）。这些数字是照头文件逐个核对过的（见常量区的行号），改常量必须同时改这里。
        #[test]
        fn pcm_params_match_the_verified_abi_values() {
            let p = pcm_params(48_000, 2);
            assert_eq!(p.format, 14, "SND_PCM_FORMAT_FLOAT_LE（pcm.h:161）");
            assert_eq!(p.access, 3, "SND_PCM_ACCESS_RW_INTERLEAVED（pcm.h:122）");
            assert_eq!(p.channels, 2);
            assert_eq!(p.rate, 48_000);
            assert_eq!(p.soft_resample, 1, "不软重采样 ⇒ 只支持 44.1kHz 的设备直接静音");
            assert_eq!(p.latency_us, 170_666);
            // 声道数原样透传（包括 0：那是无效请求，交给 alsa-lib 报错 ⇒ 静默降级，
            // **不许**在这里偷偷改成 1 —— 那会把"配置错了"变成一个听不出来的状态）
            assert_eq!(pcm_params(48_000, 6).channels, 6);
            assert_eq!(pcm_params(48_000, 0).channels, 0);
            // 采样率也原样透传（不在库里偷偷重采样成 48k）
            assert_eq!(pcm_params(44_100, 2).rate, 44_100);
        }

        /// 🔴 样本数 → 帧数：`snd_pcm_writei` 只要**帧**。
        /// 传样本数 = 让驱动多读 `channels` 倍的数据（读越界不崩、不报错，只是爆音/杂音）；
        /// 声道数为 0 时必须**返回 0 而不是除零 panic**（panic 会顺着 `AudioPlayer::tick`
        /// 把整局游戏带走 —— 音频出问题绝不该是致命的）。
        #[test]
        fn frames_from_samples_never_divides_by_zero() {
            assert_eq!(frames_from_samples(1_600, 2), 800);
            assert_eq!(frames_from_samples(1_600, 1), 1_600);
            assert_eq!(frames_from_samples(0, 2), 0);
            assert_eq!(frames_from_samples(1_600, 0), 0, "0 声道：返回 0，不许 panic");
            // 除不尽的尾样本丢弃（绝不能把"半帧"交给驱动）
            assert_eq!(frames_from_samples(1_601, 2), 800);
            // 边界：`game.rs` 每帧上限 `frames.min(8192)` ⇒ 双声道最多 16384 个样本
            assert_eq!(frames_from_samples(16_384, 2), 8_192);
        }

        /// 🔴 错误码 → 反应 的映射：`EAGAIN`/`EPIPE`/`ESTRPIPE` 三个常量写错**不会报错**，
        /// 只会让"该重试的没重试"（underrun 之后音频永久哑掉）或者"该丢的没丢"。
        /// 期望值写成字面量（errno 是 Linux ABI，头文件行号见常量区）。
        #[test]
        fn error_codes_map_to_the_right_reaction() {
            assert!(needs_recover(-32), "-EPIPE（underrun）必须 recover 后重试一次");
            assert!(needs_recover(-86), "-ESTRPIPE（流被挂起）同理");
            assert!(!needs_recover(-11), "-EAGAIN 是队列满，不是「流停了」");
            assert!(queue_full(-11), "-EAGAIN 必须判成队列满（丢本帧 + 一次性告警）");
            assert!(
                !queue_full(-32),
                "-EPIPE 不能当队列满（那样 underrun 永远等不到 recover）"
            );
            // 正数 = 真正写进去的帧数：既不需要 recover，也不是队列满
            assert!(!needs_recover(0) && !queue_full(0));
            assert!(!needs_recover(8_192) && !queue_full(8_192));
            // 其它负错误码两边都不命中 ⇒ 落到「未知错误」那条一次性告警
            assert!(!needs_recover(-5) && !queue_full(-5));
        }

        /// 🔴 打开失败 ⇒ 降级（判定被抽成纯函数，所以在没有声卡的机器上也测得了）。
        #[test]
        fn open_failure_degrades_to_silence() {
            // 失败：没有句柄 + silenced=true ⇒ `write` 变空操作，游戏照跑
            let (pcm, silenced) =
                sink_from_open(Err("snd_pcm_open(default) 失败 rc=-2".to_string()));
            assert!(pcm.is_none());
            assert!(silenced, "打开失败必须置 silenced，否则 write 会去用空句柄");
            // 成功：句柄留着、silenced=false（用悬挂指针，**不解引用** —— 这里只测映射）
            let ok: *mut c_void = std::ptr::NonNull::<c_void>::dangling().as_ptr();
            let (pcm, silenced) = sink_from_open(Ok(ok));
            assert_eq!(pcm, Some(ok));
            assert!(!silenced, "打开成功却置 silenced = 明明有声卡却永远静音");
        }

        /// 🔴 越界/NaN 必须在**我们这一侧**处理掉。
        ///
        /// 这条测试会红的方式很具体：把 `write_slice` 改成"永远直接返回 samples"（省掉那次扫描），
        /// 越界样本就会原样进驱动 ⇒ float→int 转换越界（x86 上给 0x8000）= 满量程反向爆音。
        #[test]
        fn write_slice_clamps_only_out_of_range_samples() {
            let mut scratch: Vec<f32> = Vec::new();
            // 常态：全部合规 ⇒ **零拷贝借用**（不是"新分配一个内容一样的 Vec"）
            let ok = [0.0f32, 1.0, -1.0, 0.5];
            {
                let out = write_slice(&ok, &mut scratch);
                assert!(
                    std::ptr::eq(out.as_ptr(), ok.as_ptr()),
                    "全部合规时必须原样借用，不做拷贝"
                );
                assert_eq!(out.len(), ok.len(), "夹紧不许改变长度（帧数是按它算的）");
            }
            assert!(scratch.is_empty(), "合规输入不该往暂存缓冲里写任何东西");
            // 越界 + NaN/±Inf ⇒ 夹到 [-1,1] / 归零，长度不变
            let bad = [1.5f32, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25];
            let out = write_slice(&bad, &mut scratch);
            assert_eq!(out, &[1.0, -1.0, 0.0, 0.0, 0.0, 0.25]);
            assert_eq!(out.len(), bad.len());
        }
    }
}

pub struct WaveOutSink {
    sample_rate: u32,
    channels: u16,
    #[cfg(target_os = "windows")]
    handle: usize,
    #[cfg(target_os = "windows")]
    ctx: Option<Arc<win::CallbackCtx>>,
    #[cfg(target_os = "windows")]
    buffers: Vec<win::WaveBuffer>,
    silenced: bool,
}

impl WaveOutSink {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        #[cfg(target_os = "windows")]
        {
            match win::open(sample_rate, channels) {
                Ok((handle, ctx, buffers)) => {
                    log::info!(
                        "audio: waveOut 打开成功 {}Hz/{}ch（{} 块 x {} 帧缓冲）",
                        sample_rate,
                        channels,
                        BUFFER_COUNT,
                        FRAMES_PER_BUFFER
                    );
                    return WaveOutSink {
                        sample_rate,
                        channels,
                        handle,
                        ctx: Some(ctx),
                        buffers,
                        silenced: false,
                    };
                }
                Err(e) => {
                    log::warn!("audio: waveOut 打开失败（{}），静默降级", e);
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            // 这条分支在 Linux 上**不会**被走到：Linux 的 `DefaultSink` 是 `AlsaSink`（见文件尾），
            // 只有别的平台（macOS 等）或将来把 waveOut 做成可选后端时才会经过这里。
            // 文案保持"只说 waveOut"：它描述的是本类型自己，不是"这台机器没声音"。
            log::warn!("audio: 非 Windows 平台无 waveOut，静默降级");
        }
        WaveOutSink {
            sample_rate,
            channels,
            #[cfg(target_os = "windows")]
            handle: 0,
            #[cfg(target_os = "windows")]
            ctx: None,
            #[cfg(target_os = "windows")]
            buffers: Vec::new(),
            silenced: true,
        }
    }

    #[cfg(target_os = "windows")]
    fn submit(&mut self, samples: &[f32]) {
        let (Some(ctx), false) = (&self.ctx, self.silenced) else {
            return;
        };
        let idx = (|| ctx.free.lock().ok().and_then(|mut f| f.pop()))();
        let Some(idx) = idx else {
            // 全部在播：丢弃。队列是 4×2048 帧 ≈ 170ms，正常帧率下够用。
            // 🔴 2026-09-26：这条路径以前**完全静默** —— 帧率塌到 23fps 以下（或一次长卡顿）
            // 时用户听到的是断音，而日志里一个字都没有，与上面 truncation 那条同源。
            // 丢弃本身是有意的（卡顿之后不该补播旧音频），**不能静默**才是要修的。
            // （同名的 `queued` 字段已删：它只写不读，且回调线程拿不到它 ⇒ 天生是陈旧数据。）
            if first_starved_drop() {
                log::warn!(
                    "audio: 4 个缓冲全在播（≈170ms 队列已满），本帧 {} 个样本被丢弃 —— \
                     之后若多次出现，听感就是断音",
                    samples.len()
                );
            }
            return;
        };
        let b = &mut self.buffers[idx];
        let (n, truncated) = submit_plan(
            samples.len(),
            FRAMES_PER_BUFFER * self.channels as usize,
        );
        if truncated {
            warn_submit_truncation_once(samples.len(), FRAMES_PER_BUFFER * self.channels as usize);
        }
        let data8 = b.data.as_mut_ptr();
        for i in 0..n {
            let s = samples[i].clamp(-1.0, 1.0);
            let v = (s * 32767.0) as i16;
            unsafe {
                *data8.add(i * 2) = v as u8;
                *data8.add(i * 2 + 1) = (v >> 8) as u8;
            }
        }
        b.hdr.dw_buffer_length = (n * 2) as u32;
        b.hdr.dw_user = idx;
        let rc = unsafe {
            win::waveOutWrite(
                self.handle,
                &mut b.hdr as *mut _,
                std::mem::size_of::<win::WaveHdr>() as u32,
            )
        };
        if rc != 0 {
            // 提交失败：把缓冲还回空闲表 —— 否则 4 块用完之后音频**永久静音**
            // （回调只在真正播完时才归还，失败的这块永远不会有 WOM_DONE）。
            if let Ok(mut free) = ctx.free.lock() {
                free.push(idx);
            }
        }
    }
}

impl crate::audio::AudioSink for WaveOutSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn write(&mut self, samples: &[f32]) -> usize {
        #[cfg(target_os = "windows")]
        self.submit(samples);
        samples.len()
    }
}

impl Drop for WaveOutSink {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        {
            if self.handle != 0 {
                unsafe {
                    win::waveOutReset(self.handle);
                    for b in self.buffers.iter_mut() {
                        win::waveOutUnprepareHeader(
                            self.handle,
                            &mut b.hdr as *mut _,
                            std::mem::size_of::<win::WaveHdr>() as u32,
                        );
                    }
                    // 🔴 2026-09-23 复查：`waveOutClose` 的返回值原来被丢掉。
                    // 它**可能失败**（`WAVERR_STILLPLAYING`：还有缓冲没播完/没 unprepare 成功），
                    // 而失败 ⇒ 设备仍然开着、**回调线程随时可能再进来一次**：
                    // 回调做两件事 —— 解引用 `Arc::as_ptr` 给出去的裸指针（**不增加引用计数**）、
                    // 再 `lock` 那个 Mutex。而 Drop 一结束，`ctx` 与 `buffers` 两个字段就会被释放
                    // ⇒ **use-after-free**（"关声/退出时偶发崩溃"的典型形态；缓冲的 `lpData`
                    // 同理可能还握在驱动手里）。
                    // ⇒ 关不掉时把这两份一次性分配**刻意泄漏**（进程正在退出，量级几十 KB），
                    // 换掉 UAF 窗口。泄漏是**有意的**，不是忘了释放。
                    let rc = win::waveOutClose(self.handle);
                    if rc != 0 {
                        if let Some(ctx) = self.ctx.take() {
                            std::mem::forget(ctx);
                        }
                        std::mem::forget(std::mem::take(&mut self.buffers));
                        log::warn!(
                            "audio: waveOutClose 失败 rc={}，回调上下文与缓冲已刻意泄漏以避免 use-after-free",
                            rc
                        );
                    }
                }
            }
        }
    }
}

/// Linux 的 ALSA 播放后端。字段布局刻意与 `WaveOutSink` 对齐（采样率/声道/静默标志同名同义），
/// 这样 `AudioPlayer<DefaultSink>` 与 `AudioSink` 的实现两边一眼能对上。
#[cfg(target_os = "linux")]
pub struct AlsaSink {
    sample_rate: u32,
    channels: u16,
    /// PCM 句柄。**裸指针**（`alsa::Pcm`）：对象的所有权在 alsa-lib（我们只借），`Drop` 里用
    /// `snd_pcm_close` 归还；`None` = 没打开（静默模式）。不做 `Arc` 包装：本类型只在主线程用。
    pcm: Option<alsa::Pcm>,
    /// 夹紧用的复用暂存（只在样本真的越界时才用，见 `alsa::write_slice`）。
    /// **必须是字段、不能每帧 `vec![]`**：按 8192 帧算一次就是 64KB 的分配+清零，
    /// 与 `audio.rs::AudioPlayer::synth_bus` 同一个理由。
    scratch: Vec<f32>,
    silenced: bool,
}

#[cfg(target_os = "linux")]
impl AlsaSink {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        let opened = alsa::open_pcm(sample_rate, channels);
        // 错误详情必须在映射成 (句柄, 静默) 之前取出来：`sink_from_open` 只留两态，而
        // "到底哪一步失败"（没装库 / 缺符号 / 设备被占 / 参数不支持）是用户报"没声音"时
        // 唯一的线索，压缩掉就再也查不出来了。
        if let Err(e) = &opened {
            log::warn!("audio: ALSA 打开失败（{}），静默降级为无声（游戏照常跑）", e);
        }
        let (pcm, silenced) = alsa::sink_from_open(opened);
        if !silenced {
            log::info!(
                "audio: ALSA 打开成功 {}Hz/{}ch（队列目标 {}µs ≈ {}×{} 帧）",
                sample_rate,
                channels,
                alsa::queue_latency_us(sample_rate),
                BUFFER_COUNT,
                FRAMES_PER_BUFFER
            );
        }
        AlsaSink {
            sample_rate,
            channels,
            pcm,
            scratch: Vec::new(),
            silenced,
        }
    }

    /// 把一帧交错 f32 写进设备。策略与 `WaveOutSink::submit` 一致：**写不进去就丢这一帧**
    /// （音频是实时的，补播旧数据只会让延迟越积越多），但丢弃不许静默（见 `first_starved_drop`）。
    ///
    /// 这里看不到任何 FFI 细节（指针转换/错误码比较都在 `alsa` 模块里）：本函数只表达策略。
    fn submit(&mut self, samples: &[f32]) {
        let (Some(pcm), false) = (self.pcm, self.silenced) else {
            return;
        };
        // 打开成功过 ⇒ 这张表一定在（同一个 OnceLock 初始化）。`None` 只是防御性分支，
        // **绝不 unwrap**：没声音可以接受，音频路径上 panic 不可以。
        let Some(api) = alsa::api() else {
            return;
        };
        // 🔴 长度单位是**帧**：`snd_pcm_writei` 收帧数而不是样本数（见 `alsa::frames_from_samples`）。
        let frames = alsa::frames_from_samples(samples.len(), self.channels);
        if frames == 0 {
            return;
        }
        // 越界/NaN 先处理掉（见 `alsa::write_slice`）；夹紧不改变长度 ⇒ `frames` 依然对得上。
        let data = alsa::write_slice(samples, &mut self.scratch);
        let mut n = alsa::write_frames(api, pcm, data, frames);
        if alsa::needs_recover(n) {
            // EPIPE = underrun（设备把队列播空了）；ESTRPIPE = 流被挂起（整机挂起、音频服务重启）。
            // 两种都是"流已经停了"，`snd_pcm_recover` 会 prepare（必要时 resume）后让它重新可写 ⇒
            // **重试一次**。只重试一次：recover 之后再失败说明设备真的没了（USB 声卡被拿掉），
            // 每帧多试 N 次只会把一次故障放大成持续的额外系统调用。
            let rc = alsa::recover(api, pcm, n);
            if rc < 0 {
                alsa::warn_write_error(rc);
                return;
            }
            n = alsa::write_frames(api, pcm, data, frames);
        }
        if alsa::queue_full(n) {
            // 队列一点空间都没有（≈170ms 全在播）⇒ 丢这一帧。这与 waveOut 的"4 块全在播"是
            // 同一处境，所以**复用同一个一次性告警闩**：两条后端共用一个闩，就不会分叉成
            // 「一条后端提示、另一条没动静」（丢弃本身是有意的，静默才是不该有的）。
            if first_starved_drop() {
                log::warn!(
                    "audio: ALSA 缓冲已满（≈170ms 队列全在播），本帧 {} 个样本（{} 帧）被丢弃 —— \
                     之后若多次出现，听感就是断音",
                    samples.len(),
                    frames
                );
            }
            return;
        }
        if n < 0 {
            alsa::warn_write_error(n);
        }
        // n ≥ 0：写进去 n 帧，**剩下的直接丢**（非阻塞 `writei` 允许部分写：只写进装得下的
        // 帧数就返回，不报错 —— 这不是故障，是本设计的稳态）。这里不循环补写：非阻塞下再写一次
        // 多半立刻又 -EAGAIN，循环只会把每帧的代价从 1 次系统调用抬到 N 次，也违背"绝不阻塞
        // 游戏线程"的本意。
    }
}

#[cfg(target_os = "linux")]
impl crate::audio::AudioSink for AlsaSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn write(&mut self, samples: &[f32]) -> usize {
        self.submit(samples);
        // 与 waveOut 那条路同一口径：**返回 `samples.len()`（视为已消费）**。丢弃（EAGAIN/部分写）
        // 是实时音频的既定策略；返回"真实写入数"会诱导调用方去重试剩下的半帧
        // （`AudioPlayer::tick` 现在不看返回值，但语义别留给下一个人猜）。
        samples.len()
    }
}

/// 「`writei` 失败」的一次性告警闩在 `alsa::warn_write_error`（那边才有错误码类型）。
#[cfg(target_os = "linux")]
impl Drop for AlsaSink {
    fn drop(&mut self) {
        let Some(pcm) = self.pcm.take() else {
            return;
        };
        // `pcm` 非空 ⇒ 打开时 `api()` 一定成功过（同一个 OnceLock 只初始化一次）。
        // 仍然写成 `let Some(..) else return`：没声音可以接受，退出路径上 panic 不可以。
        let Some(api) = alsa::api() else {
            return;
        };
        // `alsa::finish` = drain + close。选 drain 而不是 `snd_pcm_drop` 的理由：
        // - `close` 本身就会释放流（ALSA 没有 waveOut 那种 reset→unprepare→close 的强制次序），
        //   但 close 是**立刻**丢队列 ⇒ 最后那 ~170ms 的枪声会被切掉；
        // - drain 给队列一个播完的机会，而且**不会把退出路径挂住**：本 PCM 是以
        //   `SND_PCM_NONBLOCK` 打开的，非阻塞句柄上 drain 不做等待（2026-09-28 实测：用 `null`
        //   插件打开非阻塞句柄、灌进远超缓冲的帧之后，drain 仍在 0.001ms 内返回，状态转 SETUP）
        //   ⇒ 满足本仓「所有等待必须有上界」。设备已经消失时它直接返回错误，忽略即可。
        // - **刻意不用 drop**：那是"立刻丢"。waveOut 那边必须先 reset 是因为 unprepare 要求流已停
        //   （否则 STILLPLAYING ⇒ 关不掉的句柄 + 回调线程 UAF），ALSA 没有这个约束，所以这里可以选
        //   「体面收尾」而不是「立刻丢掉」。
        let rc = alsa::finish(api, pcm);
        if rc < 0 {
            // close 失败 = 这块 PCM 的内存回不去（进程也快退出了，代价可忽略）。
            // 🔴 **绝不能因为 close 失败就去 dlclose**：库里可能仍持有它的内部引用，卸载等于让
            // 之后任何一次调用跳进已卸载的代码（见文件头"刻意不 dlclose"）。
            log::warn!(
                "audio: snd_pcm_close 失败 rc={}（PCM 交由 alsa-lib 自行回收，库刻意不卸载）",
                rc
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub type DefaultSink = WaveOutSink;
/// Linux 走 ALSA。**这是本次改动的全部"接线"**：`game.rs:1833` 的
/// `open_default_sink(48_000, 2)` 从此在 Linux 上真的会出声（打不开就内部静默，不 panic）。
#[cfg(target_os = "linux")]
pub type DefaultSink = AlsaSink;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub type DefaultSink = crate::audio::SilentSink;

/// 单块最多能装多少**样本**（交错后的 f32 个数）：`FRAMES_PER_BUFFER × 声道数`。
///
/// 🔴 2026-09-23 复查补的判据：本帧要写的样本数可能超过这个容量（帧率骤降到
/// `48000/2048 ≈ 23fps` 以下，或加载/卡顿让某一帧的 dt 覆盖 170ms 以上）。
/// 超出的部分**丢弃是有意的**（卡顿之后不需要补播旧音频），但**不能静默** —— 见 `submit`。
fn submit_plan(available: usize, capacity: usize) -> (usize, bool) {
    (available.min(capacity), available > capacity)
}

/// 一次性告警（不刷屏）：单块装不下的样本被丢弃
fn warn_submit_truncation_once(available: usize, capacity: usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "audio: 单块只能装 {} 个样本，本帧要写 {} 个 —— 超出部分已丢弃（帧率骤降时会有杂音；后续同样情况不再提示）",
            capacity,
            available
        );
    }
}

/// 「一个空闲缓冲都没有」的丢弃是否**第一次**发生（true = 调用方该打那条 warn）。
///
/// 🔴 2026-09-26：这条丢弃路径以前是完全静默的 —— 而它与 `submit_plan` 的截断是同一种
/// 处境（缓冲/时间不够 ⇒ 丢样本），那边有一次性告警，这边什么都没有：用户听到断音，
/// 日志里查不到任何线索。抽成纯闩函数是为了能直接测（`starved_drop_warns_only_once`）。
///
/// 🔴 2026-09-28：ALSA 侧**复用同一个闩**（`AlsaSink::submit` 收到 `-EAGAIN` 时）。两条后端的
/// 处境逐字相同（"队列满 ⇒ 丢掉这一帧"），共用一个闩才不会分叉成「一条后端提示、另一条没动静」。
/// 同一进程里两条后端互斥（Windows 只走 waveOut，Linux 只走 ALSA），不会互相吃掉对方的告警。
fn first_starved_drop() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    !WARNED.swap(true, Ordering::Relaxed)
}

pub fn open_default_sink(sample_rate: u32, channels: u16) -> DefaultSink {
    #[cfg(target_os = "windows")]
    {
        WaveOutSink::new(sample_rate, channels)
    }
    // Linux：ALSA（`AlsaSink::new` 自己负责"打不开就静默"）。注意**不要**在这里加 `unwrap`/
    // `expect` —— 这个函数是在建关卡/进游戏那条路上被调的，音频出问题绝不该拦下游戏启动。
    #[cfg(target_os = "linux")]
    {
        AlsaSink::new(sample_rate, channels)
    }
    // macOS 等：仍然只有静默占位（本仓不做原生 CoreAudio，铁律 E：非 Windows/Linux 走
    // MoltenVK 那条"只编译不运行"的交叉验证路径）。
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        log::warn!("audio: 本平台无输出后端（windows=waveOut / linux=ALSA），静默降级");
        crate::audio::SilentSink::new(sample_rate, channels)
    }
}

/// 单块容量/截断判定的判据（纯函数，跨平台可测 —— 不依赖声卡）。
///
/// ALSA 侧的判据在 `alsa::tests` 里：那边的常量与纯函数是 linux-only 的，混进这个模块会让
/// Windows 构建凭空多出一堆 `cfg`。
#[cfg(test)]
mod tests {
    use super::{first_starved_drop, submit_plan, FRAMES_PER_BUFFER};

    #[test]
    fn submit_plan_never_exceeds_source_or_capacity() {
        let cap = FRAMES_PER_BUFFER * 2; // 双声道
        // 正常帧（48kHz / 60fps = 800 帧 → 1600 样本）：全写，不截断
        assert_eq!(submit_plan(1600, cap), (1600, false));
        // 边界：刚好装满
        assert_eq!(submit_plan(cap, cap), (cap, false));
        // 帧率骤降（48kHz / 20fps = 2400 帧 → 4800 样本）：截到容量，并报告截断
        assert_eq!(submit_plan(4800, cap), (cap, true));
        // 空输入：写 0，不算截断
        assert_eq!(submit_plan(0, cap), (0, false));
        // 🔴 这一条是"改错了会红"的关键：绝不能返回超过 available 的长度 ——
        // `submit` 会按这个数去索引 `samples[i]`（越界读 = panic）。
        let (n, _) = submit_plan(3, cap);
        assert!(n <= 3, "返回的写入长度不得超过源切片长度");
    }

    /// 🔴 判据：「缓冲全满 ⇒ 丢样本」只在**第一次**告警（否则 23fps 以下会每帧刷屏）。
    ///
    /// 这条路径原来是完全静默的（同文件里 `submit_plan` 的截断有一次性告警，
    /// 这边没有）—— 用户听到断音、日志里查不到线索。
    #[test]
    fn starved_drop_warns_only_once() {
        assert!(first_starved_drop(), "第一次必须返回 true（要打那条 warn）");
        assert!(!first_starved_drop(), "第二次起必须为 false（不刷屏）");
        assert!(!first_starved_drop(), "一直保持 false");
    }
}
