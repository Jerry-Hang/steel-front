//! 音频系统模块（std-only）
//!
//! - WAV 解析：手写 RIFF/WAV(PCM) 解析器，chunk 遍历 + fmt/data 解析，样本统一转 f32
//! - OGG 接口：std 无法直接解码 OGG(Vorbis)，提供 `OggDecoder` trait + DEP 注释，集成阶段补 lewton
//! - 播放后端：`AudioSink` trait 抽象，无平台依赖时用 `SilentSink` / `CollectingSink` 测试
//! - 3D 空间音频：`AudioSource` 带 3D 位置，按声源-听者距离做 `1/(1+k·d)` 衰减
//! - 音量混音：`MasterVolume` × 分通道音量（Music/Sfx）× 距离衰减，混音时相乘
//!
//! 混音输出为交错立体声（L/R 成对）。播放时需保证 clip 采样率与后端一致（重采样不在本模块范围）。

// DEP: lewton = "0.10"  // OGG(Vorbis) 解码，集成阶段补充依赖
// DEP: rodio = "0.19"   // 平台播放后端，集成阶段补充依赖

use std::fmt;
use std::sync::Arc;

use glam::Vec3;

/// 默认距离衰减系数 k（每单位距离，`1/(1+k·d)`）
const DEFAULT_ROLLOFF: f32 = 0.02;

/// WAV 解析错误
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // WAV 解析错误类型：随 parse_wav 管线预留（rodio 集成后启用）
pub enum WavError {
    /// 不是 RIFF 容器
    NotRiff,
    /// 不是 WAVE 格式
    NotWave,
    /// 缺少 fmt chunk
    MissingFmt,
    /// 缺少 data chunk
    MissingData,
    /// 不支持的音频编码格式（仅支持 PCM / IEEE float）
    UnsupportedFormat(u16),
    /// 不支持的位深
    UnsupportedBits(u16),
    /// 声道数非法
    InvalidChannels(u16),
    /// 采样率非法
    InvalidSampleRate(u32),
    /// 数据截断 / 长度不足
    Truncated,
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::NotRiff => write!(f, "not a RIFF container"),
            WavError::NotWave => write!(f, "not a WAVE file"),
            WavError::MissingFmt => write!(f, "missing fmt chunk"),
            WavError::MissingData => write!(f, "missing data chunk"),
            WavError::UnsupportedFormat(v) => write!(f, "unsupported audio format {v}"),
            WavError::UnsupportedBits(v) => write!(f, "unsupported bit depth {v}"),
            WavError::InvalidChannels(v) => write!(f, "invalid channel count {v}"),
            WavError::InvalidSampleRate(v) => write!(f, "invalid sample rate {v}"),
            WavError::Truncated => write!(f, "truncated or malformed data"),
        }
    }
}

impl std::error::Error for WavError {}

/// 音频片段：持有 f32 样本（帧交错）与采样率
#[derive(Debug, Clone)]
#[allow(dead_code)] // 字段/访问器随 WAV 管线预留；new 已用于程序化测试音
pub struct AudioClip {
    /// 样本数据，按帧交错：[f0_ch0, f0_ch1, f1_ch0, ...]
    samples: Vec<f32>,
    /// 采样率（Hz）
    sample_rate: u32,
    /// 声道数
    channels: u16,
}

#[allow(dead_code)] // 访问器随 WAV 管线预留（new/sample_mono 已用）
impl AudioClip {
    /// 创建片段；声道数或采样率为 0、样本数与声道不匹配时返回 None
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Option<Self> {
        if sample_rate == 0 || channels == 0 || samples.len() % channels as usize != 0 {
            return None;
        }
        Some(Self {
            samples,
            sample_rate,
            channels,
        })
    }

    /// 样本数据（帧交错）
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// 采样率（Hz）
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 声道数
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 总帧数
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// 时长（秒）
    pub fn duration_secs(&self) -> f32 {
        self.frame_count() as f32 / self.sample_rate as f32
    }

    /// 取某一帧、某一声道的样本
    pub fn sample_frame(&self, frame: usize, channel: usize) -> f32 {
        self.samples[frame * self.channels as usize + channel]
    }

    /// 取某一帧的单声道样本（多声道取平均）
    pub fn sample_mono(&self, frame: usize) -> f32 {
        let n = self.channels as usize;
        let start = frame * n;
        let mut sum = 0.0;
        for c in 0..n {
            sum += self.samples[start + c];
        }
        sum / n as f32
    }
}

#[allow(dead_code)] // parse_wav 辅助，随 WAV 管线预留
fn read_u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

#[allow(dead_code)] // parse_wav 辅助，随 WAV 管线预留
fn read_u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[allow(dead_code)] // parse_wav 辅助，随 WAV 管线预留
fn read_i16_le(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes([b[off], b[off + 1]])
}

#[allow(dead_code)] // parse_wav 辅助，随 WAV 管线预留
fn read_i32_le(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// PCM 音频格式：format 1 = 整数 PCM，format 3 = IEEE float
#[allow(dead_code)] // parse_wav 常量，随 WAV 管线预留
const FORMAT_PCM: u16 = 1;
#[allow(dead_code)] // parse_wav 常量，随 WAV 管线预留
const FORMAT_FLOAT: u16 = 3;

/// 解析 WAV 字节流为 `AudioClip`（支持 PCM 8/16/24/32 位整数与 32 位 float）
#[allow(dead_code)] // WAV 文件加载预留（rodio 未装，当前用程序化测试音）
pub fn parse_wav(bytes: &[u8]) -> Result<AudioClip, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" {
        return Err(WavError::NotRiff);
    }
    if &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWave);
    }

    // chunk 遍历：跳过未知 chunk，记录 fmt / data
    let mut offset = 12;
    let mut format: Option<u16> = None;
    let mut channels: Option<u16> = None;
    let mut sample_rate: Option<u32> = None;
    let mut bits: Option<u16> = None;
    let mut data: Option<&[u8]> = None;

    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = read_u32_le(bytes, offset + 4) as usize;
        let body = offset + 8;
        if body + size > bytes.len() {
            return Err(WavError::Truncated);
        }
        let chunk = &bytes[body..body + size];
        match id {
            b"fmt " => {
                if chunk.len() < 16 {
                    return Err(WavError::Truncated);
                }
                format = Some(read_u16_le(chunk, 0));
                channels = Some(read_u16_le(chunk, 2));
                sample_rate = Some(read_u32_le(chunk, 4));
                bits = Some(read_u16_le(chunk, 14));
            }
            b"data" => data = Some(chunk),
            _ => {} // LIST / fact / cue / smpl 等未知 chunk 直接跳过
        }
        // chunk 按 2 字节对齐
        offset = body + size + (size & 1);
    }

    let channels = channels.ok_or(WavError::MissingFmt)?;
    let sample_rate = sample_rate.ok_or(WavError::MissingFmt)?;
    let format = format.ok_or(WavError::MissingFmt)?;
    let bits = bits.ok_or(WavError::MissingFmt)?;
    let data = data.ok_or(WavError::MissingData)?;
    if channels == 0 {
        return Err(WavError::InvalidChannels(channels));
    }
    if sample_rate == 0 {
        return Err(WavError::InvalidSampleRate(sample_rate));
    }

    let samples = match format {
        FORMAT_PCM => decode_pcm_int(data, bits),
        FORMAT_FLOAT => {
            if bits != 32 {
                return Err(WavError::UnsupportedBits(bits));
            }
            let n = data.len() / 4;
            Ok((0..n)
                .map(|i| f32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]))
                .collect())
        }
        other => return Err(WavError::UnsupportedFormat(other)),
    }?;

    AudioClip::new(samples, sample_rate, channels).ok_or(WavError::InvalidChannels(channels))
}

/// 整数 PCM 解码：8 位无符号，16/24/32 位有符号小端，统一转 f32（[-1, 1]）
#[allow(dead_code)] // parse_wav 内部辅助，随 WAV 管线预留
fn decode_pcm_int(data: &[u8], bits: u16) -> Result<Vec<f32>, WavError> {
    Ok(match bits {
        8 => data.iter().map(|&b| (b as f32 / 128.0) - 1.0).collect(),
        16 => {
            let n = data.len() / 2;
            (0..n).map(|i| read_i16_le(data, i * 2) as f32 / 32768.0).collect()
        }
        24 => {
            let n = data.len() / 3;
            (0..n)
                .map(|i| {
                    let raw = (data[i * 3] as i32)
                        | ((data[i * 3 + 1] as i32) << 8)
                        | ((data[i * 3 + 2] as i32) << 16);
                    let v = (raw << 8) >> 8; // 符号扩展第 24 位
                    v as f32 / 8388608.0
                })
                .collect()
        }
        32 => {
            let n = data.len() / 4;
            (0..n).map(|i| read_i32_le(data, i * 4) as f32 / 2147483648.0).collect()
        }
        other => return Err(WavError::UnsupportedBits(other)),
    })
}

/// OGG(Vorbis) 解码器接口：std 无法直接解码，集成阶段用 lewton 实现
#[allow(dead_code)] // OGG 解码接口预留（lewton 集成阶段实现）
pub trait OggDecoder {
    /// 解码 OGG 字节流；失败返回 None
    fn decode_ogg(&self, data: &[u8]) -> Option<AudioClip>;
}

/// 无依赖时的空实现：始终返回 None（集成阶段替换为 lewton 实现）
#[allow(dead_code)] // lewton 未装时的占位实现，预留
pub struct NullOggDecoder;

impl OggDecoder for NullOggDecoder {
    fn decode_ogg(&self, _data: &[u8]) -> Option<AudioClip> {
        None
    }
}

/// 播放后端 trait：接收混音器输出的交错样本，送往平台音频设备
#[allow(dead_code)] // sample_rate/channels 查询预留；write 已用于 SilentSink
pub trait AudioSink {
    /// 设备采样率（Hz）
    fn sample_rate(&self) -> u32;
    /// 设备声道数
    fn channels(&self) -> u16;
    /// 写入交错样本，返回实际写入的样本数
    fn write(&mut self, samples: &[f32]) -> usize;
}

/// 静默后端：丢弃所有样本（无音频设备时的默认实现）
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // 字段仅供 trait 查询实现，预留
pub struct SilentSink {
    sample_rate: u32,
    channels: u16,
}

impl SilentSink {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }
}

impl AudioSink for SilentSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn write(&mut self, samples: &[f32]) -> usize {
        samples.len()
    }
}

/// 收集后端：保存写入的样本，供测试/调试检查输出
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // 测试收集后端，仅测试构造
pub struct CollectingSink {
    /// 已写入的交错样本
    pub samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
}

impl CollectingSink {
    #[allow(dead_code)] // 仅供测试构造
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            samples: Vec::new(),
            sample_rate,
            channels,
        }
    }
}

impl AudioSink for CollectingSink {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn write(&mut self, samples: &[f32]) -> usize {
        self.samples.extend_from_slice(samples);
        samples.len()
    }
}

/// 分通道类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// 音乐
    #[allow(dead_code)] // Music 通道预留，当前仅用 Sfx
    Music,
    /// 音效
    Sfx,
}

/// 分通道音量（线性增益 0..=1，混音时与主音量相乘）
#[derive(Debug, Clone, Copy)]
pub struct ChannelVolume {
    music: f32,
    sfx: f32,
}

impl Default for ChannelVolume {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelVolume {
    /// 所有通道默认 1.0（满音量）
    pub fn new() -> Self {
        Self { music: 1.0, sfx: 1.0 }
    }

    /// 设置指定通道音量（clamp 到 [0, 1]）
    pub fn set(&mut self, channel: Channel, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        match channel {
            Channel::Music => self.music = v,
            Channel::Sfx => self.sfx = v,
        }
    }

    /// 读取指定通道音量
    pub fn get(&self, channel: Channel) -> f32 {
        match channel {
            Channel::Music => self.music,
            Channel::Sfx => self.sfx,
        }
    }
}

/// 主音量（线性增益 0..=1，混音时与分通道音量相乘）
#[derive(Debug, Clone, Copy)]
pub struct MasterVolume(f32);

impl Default for MasterVolume {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl MasterVolume {
    /// 创建主音量（clamp 到 [0, 1]）
    pub fn new(volume: f32) -> Self {
        Self(volume.clamp(0.0, 1.0))
    }

    /// 设置主音量（clamp 到 [0, 1]）
    pub fn set(&mut self, volume: f32) {
        self.0 = volume.clamp(0.0, 1.0);
    }

    /// 当前主音量
    #[allow(dead_code)] // 查询 getter 预留（set/gain 已用）
    pub fn get(&self) -> f32 {
        self.0
    }

    /// 混音用的增益系数
    pub fn gain(&self) -> f32 {
        self.0
    }
}

/// 距离衰减系数：`1 / (1 + k·d)`，d=0 时为 1，随距离单调递减趋近 0
pub fn distance_attenuation(distance: f32, rolloff: f32) -> f32 {
    if !distance.is_finite() || distance <= 0.0 {
        return 1.0;
    }
    1.0 / (1.0 + rolloff.max(0.0) * distance)
}

/// 听者：空间音频的参考位置
#[derive(Debug, Clone, Copy)]
pub struct AudioListener {
    /// 听者位置（世界坐标）
    pub position: Vec3,
}

impl AudioListener {
    pub fn new(position: Vec3) -> Self {
        Self { position }
    }
}

/// 空间声源：3D 位置 + 自带音量
#[derive(Debug, Clone, Copy)]
pub struct AudioSource {
    /// 声源位置（世界坐标）
    pub position: Vec3,
    /// 声源基础音量（0..=1）
    pub volume: f32,
}

impl AudioSource {
    pub fn new(position: Vec3, volume: f32) -> Self {
        Self {
            position,
            volume: volume.clamp(0.0, 1.0),
        }
    }
}

/// 声音实例 ID（用于停止指定声音）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceId(usize);

/// 正在播放的声音实例
#[derive(Debug, Clone)]
struct Voice {
    #[allow(dead_code)] // 供 stop/stop_all 匹配使用；stop 系列当前未接线
    id: usize,
    clip: Arc<AudioClip>,
    /// 播放游标（帧）
    cursor: f64,
    source: AudioSource,
    channel: Channel,
    looping: bool,
    finished: bool,
}

/// 混音器：管理声音实例，按 主音量 × 通道音量 × 距离衰减 混音到交错立体声
#[derive(Debug, Clone)]
pub struct Mixer {
    voices: Vec<Voice>,
    next_id: usize,
    master: MasterVolume,
    channels: ChannelVolume,
    rolloff: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // Mixer 公共控制/查询 API 预留（主循环当前仅 play/set_*/tick 链路）
impl Mixer {
    pub fn new() -> Self {
        Self {
            voices: Vec::new(),
            next_id: 0,
            master: MasterVolume::default(),
            channels: ChannelVolume::default(),
            rolloff: DEFAULT_ROLLOFF,
        }
    }

    /// 设置主音量
    pub fn set_master(&mut self, volume: f32) {
        self.master.set(volume);
    }

    /// 主音量
    pub fn master(&self) -> f32 {
        self.master.get()
    }

    /// 设置分通道音量
    pub fn set_channel_volume(&mut self, channel: Channel, volume: f32) {
        self.channels.set(channel, volume);
    }

    /// 分通道音量
    pub fn channel_volume(&self, channel: Channel) -> f32 {
        self.channels.get(channel)
    }

    /// 设置距离衰减系数 k
    pub fn set_rolloff(&mut self, rolloff: f32) {
        self.rolloff = rolloff.max(0.0);
    }

    /// 距离衰减系数 k
    pub fn rolloff(&self) -> f32 {
        self.rolloff
    }

    /// 开始播放一个声音实例
    pub fn play(&mut self, clip: Arc<AudioClip>, source: AudioSource, channel: Channel, looping: bool) -> VoiceId {
        let id = self.next_id;
        self.next_id += 1;
        self.voices.push(Voice {
            id,
            clip,
            cursor: 0.0,
            source,
            channel,
            looping,
            finished: false,
        });
        VoiceId(id)
    }

    /// 停止指定声音
    pub fn stop(&mut self, id: VoiceId) {
        self.voices.retain(|v| v.id != id.0);
    }

    /// 停止所有声音
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    /// 停止指定通道的所有声音
    pub fn stop_channel(&mut self, channel: Channel) {
        self.voices.retain(|v| v.channel != channel);
    }

    /// 当前活跃声音数
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// 混音一帧到输出缓冲（交错立体声，长度需为偶数；自动推进游标并清理结束的声音）
    pub fn mix(&mut self, listener: &AudioListener, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        let frames = out.len() / 2;
        for v in self.voices.iter_mut() {
            let gain = self.master.gain()
                * self.channels.get(v.channel)
                * v.source.volume
                * distance_attenuation(listener.position.distance(v.source.position), self.rolloff);
            if gain <= 0.0 {
                continue;
            }
            let total = v.clip.frame_count() as f64;
            for f in 0..frames {
                let mut fi = v.cursor.floor() as usize;
                if fi >= v.clip.frame_count() {
                    if !v.looping {
                        v.finished = true;
                        break;
                    }
                    v.cursor %= total;
                    fi = v.cursor.floor() as usize;
                }
                let s = v.clip.sample_mono(fi) * gain;
                out[f * 2] += s;
                out[f * 2 + 1] += s;
                v.cursor += 1.0;
            }
        }
        self.voices.retain(|v| !v.finished);
    }

    /// 混音并返回指定帧数的交错立体声缓冲
    pub fn mix_vec(&mut self, listener: &AudioListener, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        self.mix(listener, &mut out);
        out
    }
}

/// 播放器：Mixer + AudioSink 的组合，每帧 `tick` 渲染并写入后端
#[derive(Debug)]
pub struct AudioPlayer<S: AudioSink> {
    mixer: Mixer,
    sink: S,
}

#[allow(dead_code)] // 访问器预留（sink 直读/调试用；mixer_mut 已用）
impl<S: AudioSink> AudioPlayer<S> {
    pub fn new(sink: S) -> Self {
        Self {
            mixer: Mixer::new(),
            sink,
        }
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// 渲染 `frames` 帧混音样本并写入后端
    pub fn tick(&mut self, listener: &AudioListener, frames: usize) {
        let buf = self.mixer.mix_vec(listener, frames);
        self.sink.write(&buf);
    }
}

/// 游戏音效种类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxKind {
    /// 枪声
    Gunshot,
    /// 脚步声
    Footstep,
    /// 命中
    Hit,
    /// 换弹
    Reload,
    /// HUD 提示音
    UiBlip,
    /// 环境音（供循环播放）
    Ambient,
}

/// 游戏音效库：全部 CPU 程序化合成，确定性（无 rand 依赖），无外部音频文件
#[derive(Debug, Clone)]
pub struct SfxBank {
    /// 按 `SfxKind` 下标索引的预合成 clip（与枚举顺序一致，共 6 个）
    clips: [Arc<AudioClip>; 6],
}

impl SfxBank {
    /// 以指定采样率确定性合成 6 种音效（sample_rate 为 0 时回退 44100）
    pub fn new(sample_rate: u32) -> Self {
        let sr = if sample_rate == 0 { 44_100 } else { sample_rate };
        Self {
            clips: [
                synth_gunshot(sr),
                synth_footstep(sr),
                synth_hit(sr),
                synth_reload(sr),
                synth_ui_blip(sr),
                synth_ambient(sr),
            ],
        }
    }

    /// 取指定音效的 clip
    pub fn clip(&self, kind: SfxKind) -> &Arc<AudioClip> {
        &self.clips[self.kind_index(kind)]
    }

    /// 播放指定音效（内部复用 `Mixer::play`）
    pub fn play(
        &self,
        mixer: &mut Mixer,
        kind: SfxKind,
        source: AudioSource,
        channel: Channel,
        looping: bool,
    ) -> VoiceId {
        mixer.play(self.clip(kind).clone(), source, channel, looping)
    }

    /// 播放指定音效并叠加音量缩放：`volume_scale` 先 clamp 到 0.0..=1.0，
    /// 再乘到声源音量上（结果同样 clamp 到 0.0..=1.0），复用 `Mixer::play` 链路
    pub fn play_variant(
        &self,
        mixer: &mut Mixer,
        kind: SfxKind,
        source: AudioSource,
        channel: Channel,
        looping: bool,
        volume_scale: f32,
    ) -> VoiceId {
        let scale = volume_scale.clamp(0.0, 1.0);
        let mut src = source;
        src.volume = (src.volume * scale).clamp(0.0, 1.0);
        mixer.play(self.clip(kind).clone(), src, channel, looping)
    }

    /// `SfxKind` → clips 数组下标（内部/测试用）
    pub fn kind_index(&self, kind: SfxKind) -> usize {
        match kind {
            SfxKind::Gunshot => 0,
            SfxKind::Footstep => 1,
            SfxKind::Hit => 2,
            SfxKind::Reload => 3,
            SfxKind::UiBlip => 4,
            SfxKind::Ambient => 5,
        }
    }
}

/// 合成辅助：按帧数生成单声道 clip（采样率合法且帧数 > 0）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn build_clip(sample_rate: u32, frames: usize, f: impl FnMut(usize) -> f32) -> Arc<AudioClip> {
    let samples: Vec<f32> = (0..frames).map(f).collect();
    Arc::new(AudioClip::new(samples, sample_rate, 1).expect("SfxBank 合成参数合法"))
}

/// 确定性伪随机数（LCG，固定种子，std-only，无 rand 依赖）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn lcg_next(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// LCG 输出归一化为 [-1, 1) 的白噪声样本
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn noise_unit(state: &mut u32) -> f32 {
    (lcg_next(state) as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// 枪声：0.15s，白噪声 × 快指数衰减 + 80Hz 低频 thump（峰值约 0.55）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_gunshot(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (0.15 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    let mut state: u32 = 0x9E37_79B9; // 固定种子（确定性）
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        let noise = noise_unit(&mut state) * (-t * 28.0).exp();
        let thump = (std::f32::consts::TAU * 80.0 * t).sin() * (-t * 22.0).exp();
        noise * 0.35 + thump * 0.20
    })
}

/// 脚步声：0.08s，120Hz 低频衰减脉冲 + 轻微噪声（峰值约 0.48）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_footstep(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (0.08 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    let mut state: u32 = 0xABCD_EF01; // 固定种子（确定性）
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        let pulse = (std::f32::consts::TAU * 120.0 * t).sin() * (-t * 40.0).exp();
        let noise = noise_unit(&mut state) * (-t * 60.0).exp() * 0.06;
        pulse * 0.42 + noise
    })
}

/// 命中：0.06s，短促 1kHz tick（正弦 × 快衰减，峰值约 0.5）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_hit(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (0.06 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        (std::f32::consts::TAU * 1000.0 * t).sin() * (-t * 90.0).exp() * 0.5
    })
}

/// 换弹：0.25s，两个短噪声脉冲 click（间隔约 0.12s，峰值约 0.52）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_reload(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (0.25 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    let mut state: u32 = 0x1357_9BDF; // 固定种子（确定性）
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        let mut s = 0.0;
        for t0 in [0.02f32, 0.14] {
            let dt = t - t0;
            if dt >= 0.0 {
                s += noise_unit(&mut state) * (-dt * 300.0).exp();
            }
        }
        s * 0.26
    })
}

/// HUD 提示音：0.08s，880Hz 正弦 × 起落包络（峰值约 0.4）
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_ui_blip(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (0.08 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        // 起落包络：0.01s 快起，随后指数落
        let env = if t < 0.01 { t / 0.01 } else { (-(t - 0.01) * 45.0).exp() };
        (std::f32::consts::TAU * 880.0 * t).sin() * env * 0.4
    })
}

/// 环境音：2.0s，60Hz + 120Hz 低频 drone + 0.5Hz 缓动（峰值约 0.35）
///
/// 60Hz/120Hz/0.5Hz 在 2.0s 内均为整数周期，循环点无缝，供 `looping` 播放。
#[allow(dead_code)] // 预留：SfxBank 合成辅助
fn synth_ambient(sample_rate: u32) -> Arc<AudioClip> {
    let frames = (2.0 * sample_rate as f32) as usize;
    let sr = sample_rate as f32;
    build_clip(sample_rate, frames, |i| {
        let t = i as f32 / sr;
        let drone = (std::f32::consts::TAU * 60.0 * t).sin() * 0.5
            + (std::f32::consts::TAU * 120.0 * t).sin() * 0.25;
        let lfo = 1.0 + 0.15 * (std::f32::consts::TAU * 0.5 * t).sin();
        drone * lfo * 0.4
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    /// 构造最小 WAV 字节流（fmt 固定 16 字节，无扩展字段）
    fn build_wav(format: u16, channels: u16, sample_rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        let riff_size = 4 + 8 + 16 + 8 + data.len();
        bytes.extend_from_slice(&(riff_size as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&format.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        let block_align = channels * (bits / 8);
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn wav_parses_pcm16_mono() {
        // 样本：-32768, 0, 32767, 16384 → -1.0, 0.0, 32767/32768, 0.5
        let mut data = Vec::new();
        for v in [-32768i16, 0, 32767, 16384] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let wav = build_wav(1, 1, 44100, 16, &data);
        let clip = parse_wav(&wav).unwrap();
        assert_eq!(clip.sample_rate(), 44100);
        assert_eq!(clip.channels(), 1);
        assert_eq!(clip.frame_count(), 4);
        assert!((clip.sample_mono(0) - (-1.0)).abs() < EPS);
        assert!((clip.sample_mono(1) - 0.0).abs() < EPS);
        assert!((clip.sample_mono(2) - (32767.0 / 32768.0)).abs() < EPS);
        assert!((clip.sample_mono(3) - 0.5).abs() < EPS);
        assert!((clip.duration_secs() - (4.0 / 44100.0)).abs() < 1e-4);
    }

    #[test]
    fn wav_parses_pcm8_stereo() {
        // 帧0 = (128,128) → (0,0)；帧1 = (0,255) → (-1, 127/128)
        let data = [128u8, 128, 0, 255];
        let wav = build_wav(1, 2, 22050, 8, &data);
        let clip = parse_wav(&wav).unwrap();
        assert_eq!(clip.channels(), 2);
        assert_eq!(clip.frame_count(), 2);
        assert!((clip.sample_frame(0, 0) - 0.0).abs() < EPS);
        assert!((clip.sample_frame(0, 1) - 0.0).abs() < EPS);
        assert!((clip.sample_frame(1, 0) - (-1.0)).abs() < EPS);
        assert!((clip.sample_frame(1, 1) - (127.0 / 128.0)).abs() < EPS);
        assert!((clip.sample_mono(1) - ((-1.0 + 127.0 / 128.0) / 2.0)).abs() < EPS);
    }

    #[test]
    fn wav_parses_pcm24_and_float32() {
        // 24 位：0x000000=0.0, 0x800000=-1.0, 0x400000=0.5, 0x7FFFFF≈1.0
        let d24 = [0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x40, 0xFF, 0xFF, 0x7F];
        let wav24 = build_wav(1, 1, 48000, 24, &d24);
        let clip24 = parse_wav(&wav24).unwrap();
        assert!((clip24.sample_mono(0) - 0.0).abs() < EPS);
        assert!((clip24.sample_mono(1) - (-1.0)).abs() < EPS);
        assert!((clip24.sample_mono(2) - 0.5).abs() < EPS);
        assert!((clip24.sample_mono(3) - (8388607.0 / 8388608.0)).abs() < EPS);

        // 32 位 float
        let mut d32 = Vec::new();
        for v in [0.5f32, -1.0, 0.25] {
            d32.extend_from_slice(&v.to_le_bytes());
        }
        let wav32 = build_wav(3, 1, 48000, 32, &d32);
        let clip32 = parse_wav(&wav32).unwrap();
        assert!((clip32.sample_mono(0) - 0.5).abs() < EPS);
        assert!((clip32.sample_mono(1) - (-1.0)).abs() < EPS);
        assert!((clip32.sample_mono(2) - 0.25).abs() < EPS);
    }

    #[test]
    fn wav_skips_unknown_chunks_and_alignment_padding() {
        // 在 fmt 与 data 之间插入带填充字节的未知 chunk
        let data = 0i16.to_le_bytes();
        let wav = build_wav(1, 1, 44100, 16, &data);
        let mut padded = wav.clone();
        // 手动在 fmt 后插入 3 字节 chunk（奇数长度 → 需 1 字节填充对齐）
        let insert_at = 12 + 8 + 16;
        let mut extra = Vec::new();
        extra.extend_from_slice(b"junk");
        extra.extend_from_slice(&3u32.to_le_bytes());
        extra.extend_from_slice(&[1, 2, 3, 0]); // 3 字节 + 1 字节填充
        padded.splice(insert_at..insert_at, extra);
        let clip = parse_wav(&padded).unwrap();
        assert_eq!(clip.frame_count(), 1);
        assert_eq!(clip.sample_mono(0), 0.0);
        // 原文件应解析出相同的 0.0
        assert_eq!(parse_wav(&wav).unwrap().sample_mono(0), 0.0);
    }

    #[test]
    fn wav_rejects_invalid_inputs() {
        assert!(matches!(parse_wav(b"not a wav at all"), Err(WavError::NotRiff)));
        let not_wave = b"RIFF\x04\x00\x00\x00XXXX".to_vec();
        assert!(matches!(parse_wav(&not_wave), Err(WavError::NotWave)));

        // 空 data chunk 合法（0 帧片段）
        let empty = build_wav(1, 1, 44100, 16, &[]);
        assert_eq!(parse_wav(&empty).unwrap().frame_count(), 0);

        // 只有 fmt、没有 data chunk
        let mut no_data = Vec::new();
        no_data.extend_from_slice(b"RIFF");
        no_data.extend_from_slice(&(4 + 8 + 16u32).to_le_bytes()); // "WAVE" + fmt chunk
        no_data.extend_from_slice(b"WAVE");
        no_data.extend_from_slice(b"fmt ");
        no_data.extend_from_slice(&16u32.to_le_bytes());
        no_data.extend_from_slice(&1u16.to_le_bytes());
        no_data.extend_from_slice(&1u16.to_le_bytes());
        no_data.extend_from_slice(&44100u32.to_le_bytes());
        no_data.extend_from_slice(&(44100u32 * 2).to_le_bytes());
        no_data.extend_from_slice(&2u16.to_le_bytes());
        no_data.extend_from_slice(&16u16.to_le_bytes());
        assert!(matches!(parse_wav(&no_data), Err(WavError::MissingData)));

        // 非 PCM/float 编码（ADPCM format=2）
        let adpcm = build_wav(2, 1, 44100, 4, &[0u8; 4]);
        assert!(matches!(parse_wav(&adpcm), Err(WavError::UnsupportedFormat(2))));

        // 不支持的位深（PCM 12 位）
        let weird = build_wav(1, 1, 44100, 12, &[0u8; 6]);
        assert!(matches!(parse_wav(&weird), Err(WavError::UnsupportedBits(12))));

        // 截断：声明 data 长度超过实际
        let mut trunc = build_wav(1, 1, 44100, 16, &[0u8; 2]);
        let data_size_off = trunc.len() - 2 - 4;
        trunc[data_size_off..data_size_off + 4].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(parse_wav(&trunc), Err(WavError::Truncated)));
    }

    #[test]
    fn attenuation_near_is_one_far_decays() {
        assert!((distance_attenuation(0.0, 1.0) - 1.0).abs() < EPS);
        assert!((distance_attenuation(-3.0, 1.0) - 1.0).abs() < EPS);
        assert!((distance_attenuation(f32::NAN, 1.0) - 1.0).abs() < EPS);
        // 远处 → 0
        assert!(distance_attenuation(1e6, 1.0) < 1e-5);
        // k=0 → 恒为 1
        assert!((distance_attenuation(500.0, 0.0) - 1.0).abs() < EPS);
        // 公式 1/(1+k·d)
        assert!((distance_attenuation(10.0, 2.0) - (1.0 / 21.0)).abs() < EPS);
    }

    #[test]
    fn attenuation_is_monotonic_decreasing() {
        let k = 0.05;
        let mut prev = distance_attenuation(0.0, k);
        for d in 1..=1000 {
            let cur = distance_attenuation(d as f32, k);
            assert!(cur < prev, "d={d}: {cur} >= {prev}");
            prev = cur;
        }
    }

    #[test]
    fn volume_controls_clamp() {
        let mut master = MasterVolume::new(2.0);
        assert_eq!(master.get(), 1.0);
        master.set(-1.0);
        assert_eq!(master.get(), 0.0);
        master.set(0.4);
        assert_eq!(master.gain(), 0.4);

        let mut ch = ChannelVolume::new();
        assert_eq!(ch.get(Channel::Music), 1.0);
        ch.set(Channel::Sfx, 1.7);
        assert_eq!(ch.get(Channel::Sfx), 1.0);
        ch.set(Channel::Music, 0.3);
        assert_eq!(ch.get(Channel::Music), 0.3);
        assert_eq!(ch.get(Channel::Sfx), 1.0);
    }

    fn const_clip(v: f32, frames: usize) -> Arc<AudioClip> {
        Arc::new(AudioClip::new(vec![v; frames], 44100, 1).unwrap())
    }

    #[test]
    fn mixer_multiplies_master_and_channel_volume() {
        let clip = const_clip(0.8, 8);
        let mut mixer = Mixer::new();
        mixer.set_master(0.5);
        mixer.set_channel_volume(Channel::Sfx, 0.25);
        mixer.set_channel_volume(Channel::Music, 0.9);
        mixer.play(clip, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        let mut out = vec![0.0; 16];
        mixer.mix(&AudioListener::new(Vec3::ZERO), &mut out);
        // 每帧 = 0.8 × 0.5(主) × 0.25(sfx) × 1.0(距离) = 0.1
        for v in out {
            assert!((v - 0.1).abs() < EPS, "sample {v}");
        }
        // music 通道音量不影响 sfx 声音
        assert_eq!(mixer.channel_volume(Channel::Music), 0.9);
    }

    #[test]
    fn mixer_mixes_two_sources_and_respects_distance() {
        let clip = const_clip(1.0, 8);
        let mut mixer = Mixer::new();
        mixer.set_rolloff(0.1);
        // 声源 A 在听者处（衰减 1），声源 B 距离 10（衰减 1/2）
        mixer.play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        mixer.play(clip, AudioSource::new(Vec3::new(10.0, 0.0, 0.0), 1.0), Channel::Sfx, false);
        let mut out = vec![0.0; 16];
        mixer.mix(&AudioListener::new(Vec3::ZERO), &mut out);
        let expected = 1.0 + distance_attenuation(10.0, 0.1);
        for v in out {
            assert!((v - expected).abs() < 1e-5, "sample {v} vs {expected}");
        }
    }

    #[test]
    fn mixer_channel_volume_is_per_channel() {
        let clip = const_clip(1.0, 4);
        let mut mixer = Mixer::new();
        mixer.set_channel_volume(Channel::Music, 0.2);
        mixer.set_channel_volume(Channel::Sfx, 0.6);
        mixer.play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Music, false);
        mixer.play(clip, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        let out = mixer.mix_vec(&AudioListener::new(Vec3::ZERO), 4);
        for v in out {
            assert!((v - 0.8).abs() < EPS, "sample {v}");
        }
    }

    #[test]
    fn mixer_stops_and_cleans_finished_voices() {
        let clip = const_clip(0.5, 2);
        let mut mixer = Mixer::new();
        let a = mixer.play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        mixer.play(clip, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        assert_eq!(mixer.voice_count(), 2);
        mixer.stop(a);
        assert_eq!(mixer.voice_count(), 1);
        // 播放完 2 帧后声音自动结束并清理
        mixer.mix_vec(&AudioListener::new(Vec3::ZERO), 4);
        assert_eq!(mixer.voice_count(), 0);
    }

    #[test]
    fn mixer_looping_wraps_cursor() {
        // 2 帧循环片段 [0.5, 0.25]，混 4 帧应得到 0.5,0.25,0.5,0.25
        let clip = Arc::new(AudioClip::new(vec![0.5, 0.25], 44100, 1).unwrap());
        let mut mixer = Mixer::new();
        mixer.play(clip, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, true);
        let out = mixer.mix_vec(&AudioListener::new(Vec3::ZERO), 4);
        let expected = [0.5, 0.25, 0.5, 0.25];
        for (i, v) in out.iter().enumerate() {
            assert!((v - expected[i / 2]).abs() < EPS, "frame {}: {}", i / 2, v);
        }
        assert_eq!(mixer.voice_count(), 1, "循环声音不结束");
    }

    #[test]
    fn sink_receives_mixed_samples() {
        let clip = const_clip(0.25, 2);
        let mut player = AudioPlayer::new(CollectingSink::new(44100, 2));
        player.mixer_mut().set_master(0.5);
        player
            .mixer_mut()
            .play(clip, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        player.tick(&AudioListener::new(Vec3::ZERO), 2);
        let got = &player.sink().samples;
        assert_eq!(got.len(), 4);
        for v in got {
            assert!((v - 0.125).abs() < EPS);
        }
        // 播放完毕后再 tick 输出静音
        player.tick(&AudioListener::new(Vec3::ZERO), 2);
        assert_eq!(player.sink().samples[4..], [0.0; 4]);
    }

    /// 全部 6 种音效（与 SfxKind 枚举顺序一致）
    const ALL_SFX_KINDS: [SfxKind; 6] = [
        SfxKind::Gunshot,
        SfxKind::Footstep,
        SfxKind::Hit,
        SfxKind::Reload,
        SfxKind::UiBlip,
        SfxKind::Ambient,
    ];

    #[test]
    fn sfx_bank_new_has_all_kinds_with_frames() {
        let bank = SfxBank::new(44100);
        for kind in ALL_SFX_KINDS {
            let clip = bank.clip(kind);
            assert!(clip.frame_count() > 0, "{kind:?} 帧数应为正数");
            assert_eq!(clip.sample_rate(), 44100);
            assert_eq!(clip.channels(), 1);
        }
    }

    #[test]
    fn sfx_bank_clips_have_nonzero_samples() {
        let bank = SfxBank::new(44100);
        for kind in ALL_SFX_KINDS {
            let clip = bank.clip(kind);
            assert!(
                clip.samples().iter().any(|&s| s != 0.0),
                "{kind:?} 应包含非零样本"
            );
        }
    }

    #[test]
    fn sfx_bank_play_adds_one_voice() {
        let bank = SfxBank::new(44100);
        let mut mixer = Mixer::new();
        for kind in ALL_SFX_KINDS {
            let before = mixer.voice_count();
            bank.play(&mut mixer, kind, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
            assert_eq!(mixer.voice_count(), before + 1, "{kind:?} 播放后应新增 1 个 voice");
        }
    }

    /// 同一设置下混 `frames` 帧，返回首帧样本值（便于与 play 输出对比）
    fn mix_first_sample(mixer: &mut Mixer, frames: usize) -> f32 {
        let out = mixer.mix_vec(&AudioListener::new(Vec3::ZERO), frames);
        out[0]
    }

    #[test]
    fn sfx_bank_play_variant_volume_scale_bounds() {
        let bank = SfxBank::new(44100);
        // 0.0 缩放 → 静音
        let mut mixer = Mixer::new();
        bank.play_variant(
            &mut mixer,
            SfxKind::Gunshot,
            AudioSource::new(Vec3::ZERO, 1.0),
            Channel::Sfx,
            false,
            0.0,
        );
        assert_eq!(mix_first_sample(&mut mixer, 4), 0.0);
        // 负值 clamp 到 0.0 → 同样静音
        let mut mixer = Mixer::new();
        bank.play_variant(
            &mut mixer,
            SfxKind::Gunshot,
            AudioSource::new(Vec3::ZERO, 1.0),
            Channel::Sfx,
            false,
            -1.0,
        );
        assert_eq!(mix_first_sample(&mut mixer, 4), 0.0);
        // 超 1.0 clamp 到 1.0 → 与原始声源音量一致（不放大）
        let mut mixer = Mixer::new();
        bank.play_variant(
            &mut mixer,
            SfxKind::Gunshot,
            AudioSource::new(Vec3::ZERO, 0.5),
            Channel::Sfx,
            false,
            2.0,
        );
        assert!((mix_first_sample(&mut mixer, 4) - bank.clip(SfxKind::Gunshot).sample_frame(0, 0) * 0.5).abs() < EPS);
    }

    #[test]
    fn sfx_bank_play_variant_scale_one_matches_play() {
        let bank = SfxBank::new(44100);
        for kind in ALL_SFX_KINDS {
            let mut a = Mixer::new();
            bank.play(&mut a, kind, AudioSource::new(Vec3::ZERO, 0.8), Channel::Sfx, false);
            let baseline = mix_first_sample(&mut a, 4);

            let mut b = Mixer::new();
            bank.play_variant(
                &mut b,
                kind,
                AudioSource::new(Vec3::ZERO, 0.8),
                Channel::Sfx,
                false,
                1.0,
            );
            let scaled = mix_first_sample(&mut b, 4);
            assert!((scaled - baseline).abs() < EPS, "{kind:?} scale=1.0 应与 play 等价");
        }
    }

    #[test]
    fn sfx_bank_play_variant_scales_output_amplitude() {
        let bank = SfxBank::new(44100);
        for kind in ALL_SFX_KINDS {
            let mut full = Mixer::new();
            bank.play(&mut full, kind, AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
            let baseline = mix_first_sample(&mut full, 4);

            let mut half = Mixer::new();
            bank.play_variant(
                &mut half,
                kind,
                AudioSource::new(Vec3::ZERO, 1.0),
                Channel::Sfx,
                false,
                0.5,
            );
            let scaled = mix_first_sample(&mut half, 4);
            assert!((scaled - baseline * 0.5).abs() < EPS, "{kind:?} 0.5 缩放应减半");
        }
    }

    #[test]
    fn sfx_bank_play_variant_silent_sink_no_panic() {
        let bank = SfxBank::new(44100);
        let mut player = AudioPlayer::new(SilentSink::new(44100, 2));
        for kind in ALL_SFX_KINDS {
            bank.play_variant(
                player.mixer_mut(),
                kind,
                AudioSource::new(Vec3::ZERO, 1.0),
                Channel::Sfx,
                false,
                0.5,
            );
        }
        // 静默后端下 tick 不应 panic；4 帧远短于 clip 时长，6 个 voice 应全部存活
        player.tick(&AudioListener::new(Vec3::ZERO), 4);
        assert_eq!(player.mixer().voice_count(), ALL_SFX_KINDS.len());
    }

    #[test]
    fn sfx_bank_ambient_looping_never_ends() {
        let bank = SfxBank::new(44100);
        let mut mixer = Mixer::new();
        bank.play(
            &mut mixer,
            SfxKind::Ambient,
            AudioSource::new(Vec3::ZERO, 1.0),
            Channel::Sfx,
            true,
        );
        // 混 3 倍 clip 时长，循环声音应始终存活且输出非零
        let frames = bank.clip(SfxKind::Ambient).frame_count() * 3;
        let out = mixer.mix_vec(&AudioListener::new(Vec3::ZERO), frames);
        assert_eq!(mixer.voice_count(), 1, "Ambient 循环播放不应结束");
        assert!(out.iter().any(|&s| s != 0.0), "Ambient 循环输出不应全零");
    }

    #[test]
    fn sfx_bank_synthesis_is_deterministic() {
        let a = SfxBank::new(48000);
        let b = SfxBank::new(48000);
        for kind in ALL_SFX_KINDS {
            assert_eq!(
                a.clip(kind).samples(),
                b.clip(kind).samples(),
                "{kind:?} 两次合成应逐样本一致"
            );
        }
    }

    #[test]
    fn sfx_bank_kind_index_maps_in_order() {
        let bank = SfxBank::new(44100);
        for (i, kind) in ALL_SFX_KINDS.iter().enumerate() {
            assert_eq!(bank.kind_index(*kind), i, "kind_index 应与枚举顺序一致");
        }
    }
}
