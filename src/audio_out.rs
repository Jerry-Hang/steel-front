//! Windows 真实声音输出后端（零第三方依赖：winmm waveOut 直接 FFI）。
//!
//! AudioPlayer（audio.rs）一直在混音合成（枪声/爆炸/脚步/环境音乐），
//! 但游戏默认挂了 SilentSink（静默占位）——2026-08-22 用户反馈"进游戏一点声音都没有"。
//! 本模块提供 WaveOutSink：16-bit PCM 交错样本 → waveOut 环形缓冲队列 → 声卡。
//!
//! 结构：4 块 2048 帧双声道缓冲（~85ms 队列）。主线程每帧 tick 写入小块样本
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
        if let Ok(mut free) = ctx.free.lock() {
            free.push(dw1);
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
            let mut b = WaveBuffer::new();
            let rc = unsafe {
                waveOutPrepareHeader(handle, &mut b.hdr, std::mem::size_of::<WaveHdr>() as u32)
            };
            if rc != 0 {
                return Err(format!("waveOutPrepareHeader 失败 rc={}", rc));
            }
            buffers.push(b);
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
            return; // 全部在播：丢弃（85ms 队列在 350FPS 下足够）
        };
        let b = &mut self.buffers[idx];
        let n = samples.len().min(FRAMES_PER_BUFFER * self.channels as usize);
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
                    win::waveOutClose(self.handle);
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub type DefaultSink = WaveOutSink;
#[cfg(not(target_os = "windows"))]
pub type DefaultSink = crate::audio::SilentSink;

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
