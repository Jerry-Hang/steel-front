//! Windows 真实声音输出后端（零第三方依赖：winmm waveOut 直接 FFI）。
//!
//! AudioPlayer（audio.rs）一直在混音合成（枪声/爆炸/脚步/环境音乐），
//! 但游戏默认挂了 SilentSink（静默占位）——2026-08-22 用户反馈"进游戏一点声音都没有"。
//! 本模块提供 WaveOutSink：16-bit PCM 交错样本 → waveOut 环形缓冲队列 → 声卡。
//!
//! 结构：4 块 2048 帧双声道缓冲（48kHz 下 4×2048/48000 = **170ms** 队列 —— 旧注释写"~85ms"是
//! 按 2 块算的，已更正）。主线程每帧 tick 写入小块样本
//! （350FPS 时 ~137 帧/帧），回调线程完成 buffer 后归还空闲槽；free 列表用
//! Arc<Mutex<Vec<usize>>> 保护（回调与主线程竞争）。
//!
//! 失败降级：waveOutOpen 失败 → 内部静默模式（不开声但绝不崩溃），日志告警。

use std::sync::{Arc, Mutex};

const FRAMES_PER_BUFFER: usize = 2048;
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

pub struct WaveOutSink {
    sample_rate: u32,
    channels: u16,
    #[cfg(target_os = "windows")]
    handle: usize,
    #[cfg(target_os = "windows")]
    ctx: Option<Arc<win::CallbackCtx>>,
    #[cfg(target_os = "windows")]
    buffers: Vec<win::WaveBuffer>,
    #[cfg(target_os = "windows")]
    queued: Vec<u32>,
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
                        queued: vec![0; BUFFER_COUNT],
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
            #[cfg(target_os = "windows")]
            queued: Vec::new(),
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
            return; // 全部在播：丢弃（48kHz 下 4×2048 帧 = 170ms 队列，正常帧率下够用）
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
        if rc == 0 {
            self.queued[idx] = b.hdr.dw_buffer_length;
        } else if let Ok(mut free) = ctx.free.lock() {
            free.push(idx);
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

#[cfg(target_os = "windows")]
pub type DefaultSink = WaveOutSink;
#[cfg(not(target_os = "windows"))]
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

pub fn open_default_sink(sample_rate: u32, channels: u16) -> DefaultSink {
    #[cfg(target_os = "windows")]
    {
        WaveOutSink::new(sample_rate, channels)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (sample_rate, channels);
        crate::audio::SilentSink::new(sample_rate, channels)
    }
}

/// 单块容量/截断判定的判据（纯函数，跨平台可测 —— 不依赖声卡）。
#[cfg(test)]
mod tests {
    use super::{submit_plan, FRAMES_PER_BUFFER};

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
}
