//! 音频系统模块（std-only）
//!
//! - WAV 解析：手写 RIFF/WAV(PCM) 解析器，chunk 遍历 + fmt/data 解析，样本统一转 f32
//! - OGG 接口：std 无法直接解码 OGG(Vorbis)，提供 `OggDecoder` trait + DEP 注释，集成阶段补 lewton
//! - 播放后端：`AudioSink` trait 抽象，无平台依赖时用 `SilentSink` / `CollectingSink` 测试
//! - 3D 空间音频：`AudioSource` 带 3D 位置，按声源-听者距离做 `1/(1+k·d)` 衰减
//! - 音量混音：`MasterVolume` × 分通道音量（Music/Sfx）× 距离衰减，混音时相乘
//! - 程序化 DSP：`DspSynth` 事件式合成（枪声/爆炸/脚步/环境风），ADSR 包络 + 一阶低通 + 多声部混音
//! - 程序化环境音乐：`MusicSynth` 确定性合成（低音 pad 铺底 + 行军节奏 + 五声旋律动机，二战氛围），
//!   混音总线按通道分层（Music/Sfx，`AudioPlayer::tick` 把音乐乘 Music 通道音量）+ 淡入淡出渐变
//!   （`set_music_target` 设置目标音量，默认 1.5s 线性逼近，供主会话菜单/战斗状态切换时调用）
//!
//! 混音输出为交错立体声（L/R 成对）。播放时需保证 clip 采样率与后端一致（重采样不在本模块范围）。

// DEP: lewton = "0.10"  // OGG(Vorbis) 解码，集成阶段补充依赖
// DEP: rodio = "0.19"   // 平台播放后端，集成阶段补充依赖

use std::fmt;
use std::sync::Arc;

/// 采样率兜底（3 处合成器共用；2026-08-24 常量统一）
const DEFAULT_RATE: u32 = 44_100;

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
    #[allow(dead_code)] // 平台无输出后端时/测试回退用
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
    /// 音乐（程序化环境音乐走此通道，混音总线按 Music 通道音量分层）
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

/// 播放器：Mixer + DspSynth + AudioSink 的组合，每帧 `tick` 渲染并写入后端
#[derive(Debug)]
pub struct AudioPlayer<S: AudioSink> {
    mixer: Mixer,
    synth: DspSynth,
    sink: S,
}

#[allow(dead_code)] // 访问器预留（sink 直读/调试用；mixer_mut/synth_mut 已用）
impl<S: AudioSink> AudioPlayer<S> {
    pub fn new(sink: S) -> Self {
        let sample_rate = sink.sample_rate();
        Self {
            mixer: Mixer::new(),
            synth: DspSynth::new(sample_rate),
            sink,
        }
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    pub fn mixer_mut(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    pub fn synth(&self) -> &DspSynth {
        &self.synth
    }

    pub fn synth_mut(&mut self) -> &mut DspSynth {
        &mut self.synth
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// 渲染 `frames` 帧混音样本并写入后端（clip 声部 + 程序化 DSP 声部叠加）
    pub fn tick(&mut self, listener: &AudioListener, frames: usize) {
        let mut buf = self.mixer.mix_vec(listener, frames);
        // 混音总线：音乐合成输出乘 Music 通道音量（Sfx 声部由 Mixer 内部按通道分层；淡入淡出在 MusicSynth 内）
        self.synth
            .set_music_channel_volume(self.mixer.channel_volume(Channel::Music));
        self.synth.render(listener, frames, &mut buf);
        self.sink.write(&buf);
    }

    /// 设置环境音乐淡入淡出目标音量（主会话在菜单/战斗状态切换时调用：战斗调大、菜单调小）
    pub fn set_music_target(&mut self, volume: f32) {
        self.synth.set_music_target(volume);
    }

    /// 当前环境音乐淡入淡出增益（0..=1）
    pub fn music_gain(&self) -> f32 {
        self.synth.music_gain()
    }
}

/// 枪声参数集：决定枪械音色（M1 步枪 vs Thompson 冲锋枪差异化）。
///
/// 全部字段为确定性合成参数，映射到声部发生器：
/// - `pitch`：低频爆鸣（thump）基频 Hz（步枪清脆 crack 偏高，冲锋枪低闷偏低；0 = 纯噪声无爆鸣）
/// - `thump_gain`：爆鸣分量幅度 0..=1（其余为宽带噪声；0 = 纯噪声音色）
/// - `noise_scale`：宽带噪声幅度 0..=1（整体响亮度）
/// - `cutoff`：一阶低通截止 Hz（越低越闷）
/// - `duration`：声部时长（秒，ADSR 衰减段；越长尾巴越长）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotParams {
    pub pitch: f32,
    pub thump_gain: f32,
    pub noise_scale: f32,
    pub cutoff: f32,
    pub duration: f32,
}

/// M1 加兰德步枪枪声：短促响亮、中低音、清脆 crack。
///
/// 与历史 `play_shot` 的合成参数**逐位一致**（纯噪声、cutoff 3200Hz、decay 0.12s），
/// 保证既有调用链路与全部旧单测零回归。
pub const M1_SHOT: ShotParams = ShotParams {
    pitch: 115.0,
    thump_gain: 0.0,
    noise_scale: 1.0,
    cutoff: 3200.0,
    duration: 0.12,
};

/// Thompson 冲锋枪枪声：较低音（78Hz 爆鸣）、略长尾巴（0.17s）、更闷（cutoff 2100Hz）。
/// 噪声幅度降到 0.7、爆鸣 0.45，组合峰值 ~1.15 由渲染钳位兜底，音色闷而厚。
pub const THOMPSON_SHOT: ShotParams = ShotParams {
    pitch: 78.0,
    thump_gain: 0.45,
    noise_scale: 0.70,
    cutoff: 2100.0,
    duration: 0.17,
};

/// 狙击/精确射手枪声：低沉爆鸣、长尾（0.32s）、闷厚（cutoff 2200Hz），强调远距重击感。
pub const SNIPER_SHOT: ShotParams = ShotParams {
    pitch: 62.0,
    thump_gain: 0.55,
    noise_scale: 0.85,
    cutoff: 2200.0,
    duration: 0.32,
};

/// 霰弹枪声：极低音爆鸣（55Hz）、宽厚噪声、稍长尾（0.30s），近距离震撼感。
pub const SHOTGUN_SHOT: ShotParams = ShotParams {
    pitch: 55.0,
    thump_gain: 0.65,
    noise_scale: 1.0,
    cutoff: 1700.0,
    duration: 0.30,
};

/// 机枪枪声：中低音、中等长尾（0.22s），持续压制节奏。
pub const LMG_SHOT: ShotParams = ShotParams {
    pitch: 88.0,
    thump_gain: 0.40,
    noise_scale: 0.85,
    cutoff: 2300.0,
    duration: 0.22,
};

/// 手枪枪声：高频清脆 crack、短促（0.09s），干净利落。
pub const PISTOL_SHOT: ShotParams = ShotParams {
    pitch: 150.0,
    thump_gain: 0.0,
    noise_scale: 0.85,
    cutoff: 3600.0,
    duration: 0.09,
};

/// 合成声部种类：决定发生器与音色参数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthKind {
    /// 枪声：噪声突发 + 指数衰减（ADSR；可选低频爆鸣分量由 ShotParams 驱动）
    Shot,
    /// 爆炸：低频轰鸣 + 次声成分
    Explosion,
    /// 脚步：短促宽带噪声
    Footstep,
    /// 手榴弹投掷哨声：高音正弦下滑（甩出瞬间的哨响）
    GrenadeWhistle,
    /// 手榴弹落地滚动：短促低音 thud
    GrenadeBounce,
}

/// ADSR 包络参数（时间单位秒；sustain 为 0..=1 的保持电平）
#[derive(Debug, Clone, Copy)]
pub struct Adsr {
    /// 起音时长（线性 0→1）
    pub attack: f32,
    /// 衰减时长（1 → sustain，指数）
    pub decay: f32,
    /// 保持电平（0..=1；0 表示衰减结束后声部直接结束）
    pub sustain: f32,
    /// 释音时长（指数落回 0）
    pub release: f32,
}

impl Adsr {
    pub const fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
        }
    }
}

/// 包络阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

/// ADSR 包络运行状态（每声部一份）
#[derive(Debug, Clone, Copy)]
struct AdsrEnv {
    stage: EnvStage,
    level: f32,
    elapsed: f32,
}

impl AdsrEnv {
    fn new(adsr: Adsr) -> Self {
        Self {
            stage: if adsr.attack > 0.0 {
                EnvStage::Attack
            } else {
                EnvStage::Decay
            },
            level: 0.0,
            elapsed: 0.0,
        }
    }

    /// 推进 dt 秒并返回当前包络增益（[0,1]；Done 后恒 0）
    fn advance(&mut self, adsr: &Adsr, dt: f32) -> f32 {
        if self.stage != EnvStage::Done {
            self.elapsed += dt;
            match self.stage {
                EnvStage::Attack => {
                    let frac = if adsr.attack > 0.0 {
                        (self.elapsed / adsr.attack).min(1.0)
                    } else {
                        1.0
                    };
                    self.level = frac;
                    if self.elapsed >= adsr.attack {
                        self.stage = EnvStage::Decay;
                        self.elapsed = 0.0;
                    }
                }
                EnvStage::Decay => {
                    let k = if adsr.decay > 0.0 { 1.0 / adsr.decay } else { 0.0 };
                    self.level =
                        adsr.sustain + (self.level - adsr.sustain) * (-k * self.elapsed).exp();
                    if self.elapsed >= adsr.decay {
                        if adsr.sustain <= 0.0 {
                            self.stage = EnvStage::Done;
                            self.level = 0.0;
                        } else {
                            self.stage = EnvStage::Sustain;
                            self.level = adsr.sustain;
                        }
                        self.elapsed = 0.0;
                    }
                }
                EnvStage::Sustain => {
                    self.level = adsr.sustain;
                }
                EnvStage::Release => {
                    let k = if adsr.release > 0.0 { 1.0 / adsr.release } else { 0.0 };
                    self.level = (self.level * (-k * self.elapsed).exp()).max(0.0);
                    if self.elapsed >= adsr.release || self.level <= 1e-5 {
                        self.stage = EnvStage::Done;
                        self.level = 0.0;
                    }
                }
                EnvStage::Done => {}
            }
        }
        self.level
    }

    /// 进入释音阶段（仅 Sustain 态可触发；单发音色 sustain=0 衰减结束即 Done，未接线）
    #[allow(dead_code)] // 释音阶段预留：单发音色不触发，测试覆盖
    fn release(&mut self) {
        if self.stage == EnvStage::Sustain {
            self.stage = EnvStage::Release;
            self.elapsed = 0.0;
        }
    }
}

/// 一阶低通（one-pole）系数：`a = 1 - exp(-2π·fc/sr)`；sr 或 fc 非正时返回 0（直通）
fn lowpass_alpha(sample_rate: f32, cutoff: f32) -> f32 {
    if sample_rate <= 0.0 || cutoff <= 0.0 {
        return 0.0;
    }
    1.0 - (-(std::f32::consts::TAU * cutoff / sample_rate)).exp()
}

/// 一次性合成声部：确定性发生器 + ADSR 包络 + 一阶低通
#[derive(Debug)]
struct SynthVoice {
    kind: SynthKind,
    position: Vec3,
    volume: f32,
    /// 低通截止频率（Hz）
    cutoff: f32,
    adsr: Adsr,
    /// 已发声时长（秒）
    time: f32,
    /// 噪声 LCG 状态（每声部独立种子）
    rng: u32,
    /// 一阶低通输出状态
    lp: f32,
    env: AdsrEnv,
    /// 低频爆鸣基频（Hz；仅 Shot 用，0 = 无爆鸣）
    thump_freq: f32,
    /// 低频爆鸣幅度（0..=1；仅 Shot 用，0 = 纯噪声）
    thump_gain: f32,
    /// 宽带噪声幅度（0..=1；仅 Shot 用）
    noise_scale: f32,
}

impl SynthVoice {
    /// 生成当前原始样本（未包络/未滤波；确定性）
    fn raw_sample(&mut self) -> f32 {
        let t = self.time;
        match self.kind {
            SynthKind::Shot => {
                let n = noise_unit(&mut self.rng) * self.noise_scale;
                if self.thump_gain > 0.0 {
                    // 宽带噪声 + 低频爆鸣（正弦）：步枪 crack 高脆、冲锋枪低闷
                    n + (std::f32::consts::TAU * self.thump_freq * t).sin() * self.thump_gain
                } else {
                    n
                }
            }
            SynthKind::Footstep => noise_unit(&mut self.rng),
            SynthKind::Explosion => {
                // 低频轰鸣 80→35Hz 下滑 + 次声 24Hz + 宽带噪声（进低通变“闷响”）
                let p = (t / 0.6).min(1.0);
                let rumble = (std::f32::consts::TAU * (80.0 - 45.0 * p) * t).sin() * 0.55;
                let sub = (std::f32::consts::TAU * 24.0 * t).sin() * 0.45;
                let boom = noise_unit(&mut self.rng) * (1.0 - 0.7 * p);
                rumble + sub + boom
            }
            SynthKind::GrenadeWhistle => {
                // 投掷哨声：1100→520Hz 高音下滑（0.2s 内），叠加轻微噪声质感
                let p = (t / 0.2).min(1.0);
                let freq = 1100.0 - 580.0 * p;
                (std::f32::consts::TAU * freq * t).sin() * 0.8 + noise_unit(&mut self.rng) * 0.2
            }
            SynthKind::GrenadeBounce => {
                // 落地滚动：60Hz 低音 thud × 快衰减 + 轻微噪声（滚动质感）
                let thud = (std::f32::consts::TAU * 60.0 * t).sin() * (-t * 30.0).exp();
                thud * 0.7 + noise_unit(&mut self.rng) * 0.3
            }
        }
    }
}

/// 环境风声部：持续噪声 + 慢速 LFO 调制（截止/增益），确定性
#[derive(Debug)]
struct AmbientWind {
    enabled: bool,
    position: Vec3,
    volume: f32,
    /// LFO 相位（秒）
    phase: f32,
    /// 噪声 LCG 状态
    rng: u32,
    /// 一阶低通输出状态
    lp: f32,
}

impl AmbientWind {
    fn new() -> Self {
        Self {
            enabled: false,
            position: Vec3::ZERO,
            volume: 0.0,
            phase: 0.0,
            rng: 0xA5A5_5A5A,
            lp: 0.0,
        }
    }

    /// 渲染一帧环境风样本（未钳位；距离衰减按听者位置）
    fn sample(&mut self, sample_rate: f32, listener: &AudioListener) -> f32 {
        let t = self.phase;
        // 0.13Hz 慢速 LFO：低通截止 250→650Hz 往复（风声“呜”感）
        let lfo = 0.5 + 0.5 * (std::f32::consts::TAU * 0.13 * t).sin();
        let alpha = lowpass_alpha(sample_rate, 250.0 + 400.0 * lfo);
        let n = noise_unit(&mut self.rng);
        self.lp += alpha * (n - self.lp);
        // 0.07Hz 增益 LFO（相位错开 1.7rad，避免与截止 LFO 同相）
        let gain_lfo = 1.0 + 0.35 * (std::f32::consts::TAU * 0.07 * t + 1.7).sin();
        self.phase += 1.0 / sample_rate;
        let att = distance_attenuation(listener.position.distance(self.position), DEFAULT_ROLLOFF);
        self.lp * self.volume * gain_lfo * att
    }
}

/// 一次性声部上限（防单帧大量事件打满 CPU；满时丢弃最旧声部）
const MAX_SYNTH_VOICES: usize = 32;

/// 程序化音效 DSP 合成层：事件式触发一次性声部 + 持续环境风声部。
///
/// 每帧按采样率推进 ADSR 包络与一阶低通，多声部累加后钳位到 [-1,1]；
/// 纯 std 确定性实现（LCG 噪声），无外部音频资源。
#[derive(Debug)]
pub struct DspSynth {
    sample_rate: f32,
    voices: Vec<SynthVoice>,
    next_id: usize,
    ambient: AmbientWind,
    /// 环境音乐合成器（默认静音，`set_music_target` 淡入淡出控制）
    music: MusicSynth,
}

impl DspSynth {
    /// 以指定采样率创建合成器（sample_rate 为 0 时回退 44100）
    pub fn new(sample_rate: u32) -> Self {
        let sr = if sample_rate == 0 { DEFAULT_RATE } else { sample_rate };
        Self {
            sample_rate: sr as f32,
            voices: Vec::new(),
            next_id: 1,
            ambient: AmbientWind::new(),
            music: MusicSynth::new(sr),
        }
    }

    /// 枪声：噪声突发 + 指数衰减（默认 M1 加兰德音色，委托 `play_shot_with`）。
    /// 与历史实现逐位一致（M1_SHOT 参数），既有调用链路零回归。
    #[allow(dead_code)] // 兼容入口：主会话已切换 play_shot_with 参数化调用，此签名保留供测试/第三方
    pub fn play_shot(&mut self, position: Vec3, volume: f32) -> VoiceId {
        self.play_shot_with(position, volume, M1_SHOT)
    }

    /// 参数化枪声：按 `ShotParams` 决定音色（M1_SHOT 清脆 crack / THOMPSON_SHOT 低闷长尾）。
    /// ADSR：attack 0.002s、decay = params.duration、sustain 0（单发）、短 release。
    pub fn play_shot_with(&mut self, position: Vec3, volume: f32, params: ShotParams) -> VoiceId {
        let dur = params.duration.max(0.01);
        self.spawn_full(
            SynthKind::Shot,
            position,
            volume,
            Adsr::new(0.002, dur, 0.0, (dur * 0.15).min(0.05)),
            params.cutoff,
            0x9E37_79B9,
            params.pitch,
            params.thump_gain,
            params.noise_scale,
        )
    }

    /// 手榴弹投掷哨声：高音正弦下滑（1100→520Hz，~0.23s），与枪声明显区分。
    /// 体积固定 0.7（事件自身响度已编码在合成器内），距离衰减照常。
    pub fn play_grenade_throw(&mut self, position: Vec3) -> VoiceId {
        self.spawn_full(
            SynthKind::GrenadeWhistle,
            position,
            0.7,
            Adsr::new(0.01, 0.2, 0.0, 0.05),
            4000.0,
            0x6D17_4A3B,
            0.0,
            0.0,
            1.0,
        )
    }

    /// 手榴弹落地滚动：短促低音 thud（60Hz 正弦 × 快衰减 + 噪声，~0.1s），体积固定 0.6
    #[allow(dead_code)] // 预留：主会话 game.rs 手榴弹落地事件接入（指令单 #4 阶段三，集成由主会话完成）
    pub fn play_grenade_bounce(&mut self, position: Vec3) -> VoiceId {
        self.spawn_full(
            SynthKind::GrenadeBounce,
            position,
            0.6,
            Adsr::new(0.002, 0.1, 0.0, 0.03),
            700.0,
            0x2B3C_4D5E,
            0.0,
            0.0,
            1.0,
        )
    }

    /// 爆炸：低频轰鸣（80→35Hz 下滑）+ 次声 24Hz + 宽带噪声（~200Hz 低通）
    pub fn play_explosion(&mut self, position: Vec3, volume: f32) -> VoiceId {
        self.spawn(
            SynthKind::Explosion,
            position,
            volume,
            Adsr::new(0.005, 0.8, 0.0, 0.15),
            200.0,
            0xC0FF_EE01,
        )
    }

    /// 脚步：短促宽带噪声（~850Hz 低通，~60ms 衰减）
    pub fn play_footstep(&mut self, position: Vec3, volume: f32) -> VoiceId {
        self.spawn(
            SynthKind::Footstep,
            position,
            volume,
            Adsr::new(0.001, 0.06, 0.0, 0.01),
            850.0,
            0xABCD_EF01,
        )
    }

    /// 设置环境风（慢速调制噪声）；`volume` 为 0 时关闭
    pub fn set_ambient(&mut self, position: Vec3, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        self.ambient.enabled = v > 0.0;
        self.ambient.position = position;
        self.ambient.volume = v;
    }

    /// 当前活跃一次性声部数
    #[allow(dead_code)] // 仅供测试断言声部生命周期
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// 环境风是否开启
    #[allow(dead_code)] // 仅供测试断言环境风状态
    pub fn ambient_active(&self) -> bool {
        self.ambient.enabled
    }

    /// 设置环境音乐淡入淡出目标音量（clamp [0,1]；默认 0 = 静音，主会话状态切换时调大/调小）
    pub fn set_music_target(&mut self, volume: f32) {
        self.music.set_target(volume);
    }

    /// 当前环境音乐淡入淡出增益（0..=1）
    pub fn music_gain(&self) -> f32 {
        self.music.level()
    }

    /// 设置音乐通道音量（混音总线：AudioPlayer 每帧从 Mixer 同步）
    pub fn set_music_channel_volume(&mut self, volume: f32) {
        self.music.set_channel_volume(volume);
    }

    /// 渲染 `frames` 帧并累加进 `out`（交错立体声，长度须为 2×frames）；叠加后钳位 [-1,1]
    pub fn render(&mut self, listener: &AudioListener, frames: usize, out: &mut [f32]) {
        debug_assert_eq!(out.len(), frames * 2);
        let dt = 1.0 / self.sample_rate;
        for f in 0..frames {
            let mut mono = 0.0f32;
            if self.ambient.enabled {
                mono += self.ambient.sample(self.sample_rate, listener);
            }
            for v in self.voices.iter_mut() {
                if v.env.stage == EnvStage::Done {
                    continue;
                }
                let env = v.env.advance(&v.adsr, dt);
                let raw = v.raw_sample();
                let alpha = lowpass_alpha(self.sample_rate, v.cutoff);
                v.lp += alpha * (raw - v.lp);
                let att = distance_attenuation(listener.position.distance(v.position), DEFAULT_ROLLOFF);
                mono += v.lp * env * v.volume * att;
                v.time += dt;
            }
            let s = mono.clamp(-1.0, 1.0);
            out[f * 2] += s;
            out[f * 2 + 1] += s;
        }
        // 环境音乐：确定性合成 × 淡入淡出 × 音乐通道音量（默认静音，主会话 set_music_target 控制）
        self.music.render(frames, out);
        self.voices.retain(|v| v.env.stage != EnvStage::Done);
    }

    /// 生成一个一次性声部（声部满时丢弃最旧）；非枪声默认无爆鸣（thump_gain=0、noise_scale=1）
    fn spawn(
        &mut self,
        kind: SynthKind,
        position: Vec3,
        volume: f32,
        adsr: Adsr,
        cutoff: f32,
        seed: u32,
    ) -> VoiceId {
        self.spawn_full(kind, position, volume, adsr, cutoff, seed, 0.0, 0.0, 1.0)
    }

    /// 生成一个一次性声部（含枪声爆鸣/噪声幅度参数；声部满时丢弃最旧）
    #[allow(clippy::too_many_arguments)] // 合成器内部参数直通，避免引入中间结构
    fn spawn_full(
        &mut self,
        kind: SynthKind,
        position: Vec3,
        volume: f32,
        adsr: Adsr,
        cutoff: f32,
        seed: u32,
        thump_freq: f32,
        thump_gain: f32,
        noise_scale: f32,
    ) -> VoiceId {
        if self.voices.len() >= MAX_SYNTH_VOICES {
            self.voices.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.voices.push(SynthVoice {
            kind,
            position,
            volume: volume.clamp(0.0, 1.0),
            cutoff,
            adsr,
            time: 0.0,
            rng: seed ^ (id as u32).wrapping_mul(0x0100_0193),
            lp: 0.0,
            env: AdsrEnv::new(adsr),
            thump_freq,
            thump_gain,
            noise_scale,
        });
        VoiceId(id)
    }
}

// ============================================================================
// 程序化环境音乐：低音 pad 铺底 + 行军节奏 + 五声旋律动机（二战氛围，纯 std 确定性合成）
// ============================================================================

/// 行军节奏速度（BPM，4/4 拍）
const MARCH_BPM: f32 = 112.0;

/// 单拍时长（秒）
const MARCH_BEAT: f32 = 60.0 / MARCH_BPM;

/// 音乐淡入淡出时长（秒）：主会话状态切换时音量渐变到目标
const MUSIC_FADE_SECS: f32 = 1.5;

/// 低音 pad 和弦（每小节一个，4 小节循环，频率 Hz）：Am → F → C → G（A 小调进行，二战军乐氛围）
const PAD_CHORDS: [[f32; 3]; 4] = [
    [55.0, 82.41, 110.0],  // Am: A1 E2 A2
    [43.65, 65.41, 87.31], // F:  F1 C2 F2
    [65.41, 98.0, 130.81], // C:  C2 G2 C3
    [49.0, 73.42, 98.0],   // G:  G1 D2 G2
];

/// 旋律动机：A 小调五声音阶（A C D E G，半音偏移 0/3/5/7/10；12 = 高八度 A）。
/// 16 音符 = 4 小节 × 4 拍，行军式起伏；`st >= 0` 为音高、其余休止。
const MELODY_MOTIF: [i8; 16] = [0, 0, 3, 5, 7, 7, 5, 3, 5, 5, 7, 10, 12, 10, 7, 5];

/// 淡入淡出插值（纯函数）：`level` 以线性速度向 `target` 逼近，最大步进 `dt / fade_secs`
/// （0→1 完整过渡恰需 `fade_secs` 秒）；永不越过 target。
/// `fade_secs <= 0` = 无淡变（直接跳变到 target）；`dt <= 0` = 时间未流逝（保持原值）。
fn fade_step(level: f32, target: f32, dt: f32, fade_secs: f32) -> f32 {
    let target = target.clamp(0.0, 1.0);
    if fade_secs <= 0.0 {
        return target;
    }
    if dt <= 0.0 {
        return level;
    }
    let delta = target - level;
    let max_step = dt / fade_secs;
    if delta.abs() <= max_step {
        return target;
    }
    level + delta.signum() * max_step
}

/// 确定性整数散列 → [-1,1)（无状态；军鼓噪声按绝对样本索引取种，纯函数可测）
fn hash_noise(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x as f32 / u64::MAX as f32) * 2.0 - 1.0
}

/// 生成某绝对时间点的音乐单声道样本（纯函数：仅由 `time_secs` / `sample_rate` 决定，确定性）。
///
/// 三声部叠加（各声部增益之和 ≤ ~0.8，输出天然在 [-1,1] 内）：
/// - 低音 pad：4 小节和弦循环 + 0.25s 起落包络（小节边界零振幅防爆音）+ 0.13Hz 慢 LFO；
/// - 行军节奏：底鼓（60→35Hz 下滑正弦）在 1/3 拍、军鼓（散列噪声突发）在 2/4 拍；
/// - 旋律动机：16 音符五声音阶循环（每拍一音），正弦 + 轻微二次谐波（簧管感）。
fn music_wave(time_secs: f32, sample_rate: f32) -> f32 {
    let bar_len = 4.0 * MARCH_BEAT; // 1 小节 4 拍
    let bar = (time_secs / bar_len).floor();
    let bar_pos = time_secs - bar * bar_len; // 小节内偏移（0..bar_len）
    let beat = bar_pos / MARCH_BEAT; // 小节内拍号（0..4）
    let beat_idx = beat.floor();
    let t_beat = (beat - beat_idx) * MARCH_BEAT; // 拍内偏移（秒）

    let mut s = 0.0;

    // 低音 pad：4 小节和弦循环，0.25s 起落包络 + 0.13Hz 慢 LFO
    let chord = PAD_CHORDS[bar.rem_euclid(4.0) as usize];
    let pad_env = ((bar_pos / 0.25).min(1.0)).min(((bar_len - bar_pos) / 0.25).min(1.0));
    let lfo = 0.75 + 0.25 * (std::f32::consts::TAU * 0.13 * time_secs + bar * 1.7).sin();
    for f in chord {
        s += (std::f32::consts::TAU * f * time_secs).sin() * pad_env * lfo * 0.09;
    }

    // 行军节奏：底鼓 0/2 拍、军鼓 1/3 拍（鼓点持续 0.2s）
    if t_beat < 0.2 {
        match beat_idx as i32 {
            0 | 2 => {
                let f = 60.0 - 25.0 * (t_beat / 0.2);
                s += (std::f32::consts::TAU * f * t_beat).sin() * (-t_beat * 18.0).exp() * 0.25;
            }
            1 | 3 => {
                let idx = (time_secs * sample_rate) as u64;
                s += hash_noise(idx) * (-t_beat * 30.0).exp() * 0.18;
            }
            _ => {}
        }
    }

    // 旋律动机：16 音符循环（每拍一音），正弦 + 二次谐波
    let total_beat = (time_secs / MARCH_BEAT).floor();
    let note_idx = total_beat.rem_euclid(16.0) as usize;
    let t_note = time_secs - total_beat * MARCH_BEAT;
    let st = MELODY_MOTIF[note_idx];
    if st >= 0 {
        let f = 110.0 * 2.0f32.powf(st as f32 / 12.0);
        let env = ((t_note / 0.02).min(1.0)) * (-t_note * 3.5).exp();
        s += ((std::f32::consts::TAU * f * time_secs).sin()
            + 0.3 * (std::f32::consts::TAU * 2.0 * f * time_secs).sin())
            * env
            * 0.08;
    }

    s
}

/// 纯函数：渲染 `frames = out.len()/2` 帧环境音乐并累加到 `out`（交错立体声，与混音输出格式一致）。
/// 起始相位由绝对时间 `time_secs` 决定 → 同参数必得同输出；每样本先乘 `gain` 再钳位 [-1,1]。
fn render_music_into(time_secs: f32, sample_rate: f32, gain: f32, out: &mut [f32]) {
    debug_assert_eq!(out.len() % 2, 0);
    for (i, pair) in out.chunks_exact_mut(2).enumerate() {
        let t = time_secs + i as f32 / sample_rate;
        let s = (music_wave(t, sample_rate) * gain).clamp(-1.0, 1.0);
        pair[0] += s;
        pair[1] += s;
    }
}

/// 环境音乐合成器：确定性音乐渲染 + 淡入淡出音量渐变。
///
/// - 波形由 `music_wave`（纯函数，time_secs 驱动相位）渲染，单声道复制到 L/R；
/// - `fade_level` 为淡入淡出增益（默认 0 = 静音），每 tick 以 `fade_secs`（默认 1.5s）
///   线性逼近 `fade_target`（`set_target` 设置，主会话在菜单/战斗状态切换时调用）；
/// - `channel_volume` 为混音总线 Music 通道音量（AudioPlayer 每帧从 Mixer 同步）。
#[derive(Debug)]
pub struct MusicSynth {
    sample_rate: f32,
    /// 累计时间相位（秒）：驱动确定性合成，淡入淡出期间持续推进（波形无缝）
    time_secs: f32,
    /// 淡入淡出目标音量（0..=1）
    fade_target: f32,
    /// 当前淡入淡出音量（0..=1）
    fade_level: f32,
    /// 淡入淡出时长（秒）
    fade_secs: f32,
    /// 音乐通道音量（混音总线，0..=1）
    channel_volume: f32,
}

impl MusicSynth {
    /// 以指定采样率创建合成器（默认静音：fade_target=0；sample_rate=0 回退 44100）
    pub fn new(sample_rate: u32) -> Self {
        let sr = if sample_rate == 0 { DEFAULT_RATE } else { sample_rate };
        Self {
            sample_rate: sr as f32,
            time_secs: 0.0,
            fade_target: 0.0,
            fade_level: 0.0,
            fade_secs: MUSIC_FADE_SECS,
            channel_volume: 1.0,
        }
    }

    /// 设置淡入淡出目标音量（clamp [0,1]；主会话在菜单/战斗状态切换时调用）
    pub fn set_target(&mut self, volume: f32) {
        self.fade_target = volume.clamp(0.0, 1.0);
    }

    /// 设置音乐通道音量（混音总线：AudioPlayer 每帧从 Mixer 同步）
    pub fn set_channel_volume(&mut self, volume: f32) {
        self.channel_volume = volume.clamp(0.0, 1.0);
    }

    /// 当前淡入淡出增益（0..=1）
    pub fn level(&self) -> f32 {
        self.fade_level
    }

    /// 推进淡入淡出并渲染 `frames` 帧到 `out`（交错立体声，累加进现有缓冲）
    pub fn render(&mut self, frames: usize, out: &mut [f32]) {
        debug_assert_eq!(out.len(), frames * 2);
        if frames == 0 {
            return;
        }
        let dt = 1.0 / self.sample_rate;
        self.fade_level = fade_step(
            self.fade_level,
            self.fade_target,
            frames as f32 * dt,
            self.fade_secs,
        );
        let gain = self.fade_level * self.channel_volume;
        if gain > 0.0 {
            render_music_into(self.time_secs, self.sample_rate, gain, out);
        }
        self.time_secs += frames as f32 * dt;
    }
}

/// 游戏音效种类
#[allow(dead_code)] // 预留：事件式合成已走 DspSynth，旧 SfxKind 仅保留预合成链路（Hit/Reload/UiBlip 在用）
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
        let sr = if sample_rate == 0 { DEFAULT_RATE } else { sample_rate };
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
    #[allow(dead_code)] // 预留：旧 SfxBank 链路，事件式合成已走 DspSynth
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

    #[test]
    fn dsp_synth_output_is_finite_and_clamped() {
        let mut synth = DspSynth::new(48_000);
        let listener = AudioListener::new(Vec3::ZERO);
        synth.play_shot(Vec3::ZERO, 1.0);
        synth.play_explosion(Vec3::ZERO, 1.0);
        synth.play_footstep(Vec3::ZERO, 1.0);
        synth.set_ambient(Vec3::ZERO, 0.5);
        let mut out = vec![0.0; 4096];
        synth.render(&listener, out.len() / 2, &mut out);
        for (i, &s) in out.iter().enumerate() {
            assert!(s.is_finite(), "sample {i} 应为有限值");
            assert!(s.abs() <= 1.0, "sample {i} = {s} 超出 [-1,1]");
        }
        assert!(out.iter().any(|&s| s != 0.0), "应包含非零样本");
    }

    #[test]
    fn dsp_synth_shot_decays_to_silence() {
        let mut synth = DspSynth::new(48_000);
        let listener = AudioListener::new(Vec3::ZERO);
        synth.play_shot(Vec3::ZERO, 1.0);
        let mut out = vec![0.0; 24_000];
        synth.render(&listener, 12_000, &mut out);
        assert!(out.iter().any(|&s| s != 0.0), "枪声应有输出");
        // 指数衰减：头部能量应大于尾部
        let head: f32 = out[..800].iter().map(|s| s.abs()).sum();
        let tail: f32 = out[out.len() - 800..].iter().map(|s| s.abs()).sum();
        assert!(head > tail, "枪声应随时间衰减");
        assert_eq!(synth.voice_count(), 0, "0.25s 后枪声声部应结束");
        let mut rest = vec![0.0; 2400];
        synth.render(&listener, 1200, &mut rest);
        assert!(rest.iter().all(|&s| s == 0.0), "枪声结束后应静音");
    }

    #[test]
    fn dsp_synth_explosion_outlasts_footstep() {
        let listener = AudioListener::new(Vec3::ZERO);
        // 脚步：~60ms 衰减，0.1s 后应已结束
        let mut f = DspSynth::new(48_000);
        f.play_footstep(Vec3::ZERO, 1.0);
        let mut buf = vec![0.0; 9600];
        f.render(&listener, 4800, &mut buf);
        assert!(buf.iter().any(|&s| s != 0.0), "脚步应有输出");
        assert_eq!(f.voice_count(), 0, "脚步 0.1s 内应结束");
        let mut rest = vec![0.0; 9600];
        f.render(&listener, 4800, &mut rest);
        assert!(rest.iter().all(|&s| s == 0.0), "脚步结束后应静音");
        // 爆炸：decay 0.8s，0.2s 时仍活跃且有输出，~0.9s 后结束
        let mut e = DspSynth::new(48_000);
        e.play_explosion(Vec3::ZERO, 1.0);
        let mut buf = vec![0.0; 9600];
        e.render(&listener, 4800, &mut buf);
        assert!(buf.iter().any(|&s| s != 0.0), "爆炸应有输出");
        assert!(e.voice_count() >= 1, "爆炸 decay 0.8s，0.1s 时仍活跃");
        let mut mid = vec![0.0; 9600];
        e.render(&listener, 4800, &mut mid);
        assert!(mid.iter().any(|&s| s != 0.0), "爆炸 0.2s 后仍有轰鸣");
        for _ in 0..7 {
            let mut more = vec![0.0; 9600];
            e.render(&listener, 4800, &mut more);
        }
        assert_eq!(e.voice_count(), 0, "爆炸 ~0.9s 后应结束");
        let mut final_out = vec![0.0; 9600];
        e.render(&listener, 4800, &mut final_out);
        assert!(final_out.iter().all(|&s| s == 0.0), "爆炸结束后应静音");
    }

    #[test]
    fn dsp_synth_distance_attenuation_matches_formula() {
        let listener = AudioListener::new(Vec3::ZERO);
        let att = distance_attenuation(100.0, DEFAULT_ROLLOFF);
        let mut near = DspSynth::new(48_000);
        near.play_shot(Vec3::ZERO, 0.5);
        let mut far = DspSynth::new(48_000);
        far.play_shot(Vec3::new(100.0, 0.0, 0.0), 0.5);
        let mut a = vec![0.0; 4800];
        let mut b = vec![0.0; 4800];
        near.render(&listener, 2400, &mut a);
        far.render(&listener, 2400, &mut b);
        // 两路事件序列相同 → 原始样本一致，仅距离衰减不同（音量 0.5 无钳位干扰）
        for i in (0..a.len()).step_by(2) {
            let expected = a[i] * att;
            assert!(
                (b[i] - expected).abs() < 1e-5,
                "frame {}: {} vs {}",
                i / 2,
                b[i],
                expected
            );
        }
    }

    #[test]
    fn dsp_synth_mix_clamps_under_loud_polyphony() {
        let mut synth = DspSynth::new(48_000);
        let listener = AudioListener::new(Vec3::ZERO);
        // 8 个同时起爆的爆炸声部（正弦分量同相叠加）必然超 1，验证钳位
        for _ in 0..8 {
            synth.play_explosion(Vec3::ZERO, 1.0);
        }
        let mut out = vec![0.0; 9600];
        synth.render(&listener, 4800, &mut out);
        for (i, &s) in out.iter().enumerate() {
            assert!(s.is_finite(), "sample {i} 应为有限值");
            assert!(s.abs() <= 1.0, "sample {i} = {s} 超出 [-1,1]");
        }
        assert!(out.iter().any(|&s| s.abs() == 1.0), "多声部叠加应触发钳位");
    }

    #[test]
    fn dsp_synth_ambient_wind_toggle() {
        let mut synth = DspSynth::new(48_000);
        let listener = AudioListener::new(Vec3::ZERO);
        assert!(!synth.ambient_active(), "默认无环境风");
        synth.set_ambient(Vec3::ZERO, 0.0);
        assert!(!synth.ambient_active(), "volume=0 视为关闭");
        synth.set_ambient(Vec3::new(0.0, 2.0, 0.0), 0.4);
        assert!(synth.ambient_active());
        let mut out = vec![0.0; 4800];
        synth.render(&listener, 2400, &mut out);
        assert!(out.iter().any(|&s| s != 0.0), "环境风应有输出");
        assert!(out.iter().all(|&s| s.is_finite() && s.abs() <= 1.0));
        synth.set_ambient(Vec3::ZERO, 0.0);
        let mut silent = vec![0.0; 4800];
        synth.render(&listener, 2400, &mut silent);
        assert!(silent.iter().all(|&s| s == 0.0), "关闭环境风后应静音");
    }

    #[test]
    fn dsp_synth_deterministic() {
        let listener = AudioListener::new(Vec3::ZERO);
        let mut a = DspSynth::new(48_000);
        let mut b = DspSynth::new(48_000);
        for s in [&mut a, &mut b] {
            s.play_shot(Vec3::ZERO, 0.8);
            s.play_explosion(Vec3::new(10.0, 0.0, 0.0), 0.6);
            s.play_footstep(Vec3::ZERO, 0.5);
            s.set_ambient(Vec3::ZERO, 0.3);
        }
        let mut oa = vec![0.0; 9600];
        let mut ob = vec![0.0; 9600];
        a.render(&listener, 4800, &mut oa);
        b.render(&listener, 4800, &mut ob);
        assert_eq!(oa, ob, "同一事件序列应逐样本一致");
    }

    #[test]
    fn dsp_synth_voice_cap_drops_oldest() {
        let mut synth = DspSynth::new(48_000);
        for _ in 0..MAX_SYNTH_VOICES + 4 {
            synth.play_shot(Vec3::ZERO, 0.5);
        }
        assert_eq!(synth.voice_count(), MAX_SYNTH_VOICES, "超限应丢弃最旧声部");
    }

    #[test]
    fn dsp_synth_zero_sample_rate_falls_back() {
        let synth = DspSynth::new(0);
        assert_eq!(synth.sample_rate, 44_100.0);
    }

    #[test]
    fn adsr_envelope_stages() {
        let adsr = Adsr::new(0.01, 0.1, 0.2, 0.05);
        let mut env = AdsrEnv::new(adsr);
        // 起音：半程 ~0.5，满程 ~1.0 后进入衰减
        let mid = env.advance(&adsr, 0.005);
        assert!((mid - 0.5).abs() < 1e-3, "attack 半程应 ~0.5");
        let full = env.advance(&adsr, 0.005);
        assert!((full - 1.0).abs() < 1e-3, "attack 满程应 ~1.0");
        // 衰减：decay 满程后进入 Sustain 且电平 ≈ sustain
        let _ = env.advance(&adsr, 0.1);
        assert!((env.level - 0.2).abs() < 1e-3, "decay 结束应到 sustain");
        // 释音：半程衰减中，满程结束
        env.release();
        let _ = env.advance(&adsr, 0.025);
        assert!(env.level < 0.2, "release 半程应衰减");
        let _ = env.advance(&adsr, 0.025);
        assert_eq!(env.stage, EnvStage::Done);
        assert_eq!(env.level, 0.0);
        // sustain=0 的单发音色：衰减结束直接 Done
        let one_shot = Adsr::new(0.001, 0.05, 0.0, 0.0);
        let mut env = AdsrEnv::new(one_shot);
        let _ = env.advance(&one_shot, 0.001);
        let _ = env.advance(&one_shot, 0.05);
        assert_eq!(env.stage, EnvStage::Done, "sustain=0 衰减结束即结束");
    }

    #[test]
    fn lowpass_alpha_bounds() {
        assert_eq!(lowpass_alpha(48_000.0, 0.0), 0.0);
        assert_eq!(lowpass_alpha(0.0, 1000.0), 0.0);
        let a = lowpass_alpha(48_000.0, 24_000.0);
        assert!(a > 0.0 && a < 1.0, "奈奎斯特处系数应在 (0,1)");
        let lo = lowpass_alpha(48_000.0, 200.0);
        let hi = lowpass_alpha(48_000.0, 4000.0);
        assert!(lo < hi, "截止越高系数越大（带宽越宽）");
    }

    #[test]
    fn audio_player_tick_renders_synth_into_sink() {
        let mut player = AudioPlayer::new(CollectingSink::new(44_100, 2));
        player.synth_mut().play_shot(Vec3::ZERO, 0.5);
        player.tick(&AudioListener::new(Vec3::ZERO), 128);
        let got = &player.sink().samples;
        assert_eq!(got.len(), 256);
        assert!(got.iter().any(|&s| s != 0.0), "DSP 声部应进入 sink");
        assert!(got.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    // ---------- 程序化环境音乐 ----------

    /// 用纯函数路径渲染 `frames` 帧音乐（gain 前置乘），返回交错立体声缓冲
    fn render_music_buffer(time_secs: f32, sample_rate: u32, frames: usize, gain: f32) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        render_music_into(time_secs, sample_rate as f32, gain, &mut out);
        out
    }

    #[test]
    fn music_wave_is_deterministic_pure() {
        // 同 time_secs → 同输出（纯函数）
        for t in [0.0f32, 0.5, 1.234, 60.0, 12345.678] {
            assert_eq!(music_wave(t, 48_000.0), music_wave(t, 48_000.0), "t={t}");
        }
        // 相位驱动：连续时间推进输出应变化（非常量）
        let mut prev = music_wave(0.0, 48_000.0);
        let mut changed = false;
        for i in 1..100 {
            let cur = music_wave(i as f32 * 0.001, 48_000.0);
            if (cur - prev).abs() > 1e-6 {
                changed = true;
                break;
            }
            prev = cur;
        }
        assert!(changed, "音乐波形应随时间变化");
        // 缓冲级确定性：同起始 time_secs 两次渲染逐样本一致
        assert_eq!(
            render_music_buffer(3.21, 48_000, 2000, 1.0),
            render_music_buffer(3.21, 48_000, 2000, 1.0)
        );
    }

    #[test]
    fn music_render_output_length_and_stereo_format() {
        let frames = 137;
        let out = render_music_buffer(3.21, 48_000, frames, 1.0);
        assert_eq!(out.len(), frames * 2, "交错立体声：2×frames 样本");
        for pair in out.chunks_exact(2) {
            assert_eq!(pair[0], pair[1], "音乐单声道复制到 L/R");
        }
    }

    #[test]
    fn music_render_samples_finite_and_bounded() {
        let out = render_music_buffer(0.0, 48_000, 4 * 48_000, 1.0); // 4s（约 2 小节）
        for (i, &s) in out.iter().enumerate() {
            assert!(s.is_finite(), "sample {i} 应为有限值");
            assert!(s.abs() <= 1.0, "sample {i} = {s} 超出 [-1,1]");
        }
        assert!(out.iter().any(|&s| s != 0.0), "应有非零样本");
    }

    #[test]
    fn music_bass_pad_dominates_low_frequency_band() {
        // 渲染 2 小节（~4.29s）：一阶低通 150Hz 后保留的能量占比应显著（pad/底鼓均低频）
        let sr = 48_000.0;
        let frames = (2.0 * 4.0 * MARCH_BEAT * sr) as usize;
        let out = render_music_buffer(0.0, 48_000, frames, 1.0);
        let alpha = lowpass_alpha(sr, 150.0);
        let mut lp = 0.0f32;
        let mut low_energy = 0.0f32;
        let mut total_energy = 0.0f32;
        for &s in out.iter() {
            lp += alpha * (s - lp);
            low_energy += lp * lp;
            total_energy += s * s;
        }
        assert!(total_energy > 0.0);
        let ratio = low_energy / total_energy;
        assert!(ratio > 0.3, "低频能量占比 {ratio} 过低（pad/底鼓应主导）");
    }

    #[test]
    fn music_march_rhythm_makes_energy_pulse() {
        // 0.1s 窗能量随鼓点起伏：最大/最小窗能量比应显著 > 1
        let sr = 48_000.0;
        let frames = (2.0 * 4.0 * MARCH_BEAT * sr) as usize;
        let out = render_music_buffer(0.0, 48_000, frames, 1.0);
        let win = (0.1 * sr) as usize;
        let mut wins = Vec::new();
        for chunk in out.chunks(win * 2) {
            let e: f32 = chunk.iter().map(|s| s * s).sum::<f32>() / (chunk.len() as f32);
            wins.push(e);
        }
        let max = wins.iter().cloned().fold(0.0f32, f32::max);
        let min = wins.iter().cloned().fold(f32::MAX, f32::min);
        assert!(min > 0.0 && max > 0.0);
        assert!(max / min > 1.3, "窗能量比 {:.3} 应体现鼓点起伏", max / min);
    }

    #[test]
    fn music_fade_step_is_monotonic_and_converges() {
        // 0 → 1，fade_secs=1.5，dt=0.15 → 10 步恰好到达 target，且单调不减、不越过
        let mut level = 0.0f32;
        let mut prev = level;
        for i in 1..=10 {
            level = fade_step(level, 1.0, 0.15, 1.5);
            assert!(level >= prev, "淡入应单调不减");
            assert!(level <= 1.0, "不得越过 target");
            if i < 10 {
                assert!(level < 1.0, "第 {i} 步不应提前到达");
            }
            prev = level;
        }
        assert!((level - 1.0).abs() < EPS, "10 步后应恰好到达 target");
        // 淡出方向同样单调且收敛
        let mut level = 1.0f32;
        for _ in 0..10 {
            let next = fade_step(level, 0.0, 0.15, 1.5);
            assert!(next <= level, "淡出应单调不减音量");
            level = next;
        }
        assert_eq!(level, 0.0, "10 步后淡出完成");
    }

    #[test]
    fn music_fade_retargets_mid_fade_and_edge_cases() {
        // 中途改目标 → 收敛到新目标
        let mut level = 0.0f32;
        for _ in 0..6 {
            level = fade_step(level, 0.8, 0.1, 1.5);
        }
        assert!((level - 0.4).abs() < 1e-5, "0.6s 后应到 0.4，实际 {level}");
        for _ in 0..10 {
            level = fade_step(level, 0.2, 0.1, 1.5);
        }
        assert!((level - 0.2).abs() < 1e-5, "改目标后应收敛到 0.2");
        // fade_secs=0 → 直接跳变
        assert_eq!(fade_step(0.3, 0.9, 0.1, 0.0), 0.9);
        // dt=0 → 不动
        assert_eq!(fade_step(0.3, 0.9, 0.0, 1.5), 0.3);
        // target 越界 clamp 到 [0,1]，并按一步逼近（不是直接跳变）
        let v = fade_step(0.5, 2.0, 0.1, 1.5);
        assert!(
            (v - (0.5 + 0.1 / 1.5)).abs() < 1e-5,
            "clamp 后向 target 一步逼近，实际 {v}"
        );
    }

    #[test]
    fn music_synth_default_silent_then_fades_in() {
        let mut m = MusicSynth::new(48_000);
        assert_eq!(m.level(), 0.0);
        assert_eq!(m.fade_target, 0.0);
        assert_eq!(m.fade_secs, MUSIC_FADE_SECS);
        // 默认静音：渲染不产生任何输出（输出保持不变）
        let mut out = vec![0.5; 8];
        m.render(4, &mut out);
        assert_eq!(out, vec![0.5; 8], "静音时不应改动输出");
        // 设置目标后逐 tick 淡入
        m.set_target(1.0);
        let mut out = vec![0.0; 8];
        m.render(4, &mut out);
        assert!(out.iter().any(|&s| s != 0.0), "淡入开始后应有输出");
        assert!(m.level() > 0.0 && m.level() < 1.0, "淡入中：0 < level < 1");
        // 推进超过淡入时长 → 到达满音量
        let need = (MUSIC_FADE_SECS * 48_000.0) as usize / 4;
        for _ in 0..need {
            m.render(4, &mut out);
        }
        assert!((m.level() - 1.0).abs() < 1e-3, "淡入完成后 level 应到 1.0");
    }

    #[test]
    fn music_synth_fades_out_to_silence() {
        let mut m = MusicSynth::new(48_000);
        m.set_target(1.0);
        let mut buf = vec![0.0f32; 4800];
        let need = (MUSIC_FADE_SECS * 48_000.0) as usize / 2400;
        for _ in 0..need {
            m.render(2400, &mut buf);
        }
        assert!((m.level() - 1.0).abs() < 1e-3, "淡入完成");
        // 设为 0 → 淡出到静音
        m.set_target(0.0);
        let mut scratch = vec![0.0f32; 4800];
        for _ in 0..need {
            m.render(2400, &mut scratch);
        }
        assert_eq!(m.level(), 0.0, "淡出完成后应静音");
        let mut final_out = vec![0.0f32; 4800];
        m.render(2400, &mut final_out);
        assert!(final_out.iter().all(|&s| s == 0.0), "静音后渲染应全零");
    }

    #[test]
    fn audio_player_bus_music_channel_gates_music_only() {
        // 混音总线：Music 通道音量 = 0 → 音乐不进入输出，且 Sfx 声音不受影响
        let clip = const_clip(0.5, 8);
        // 参考：音乐默认静音（target=0）→ 纯 Sfx
        let mut ref_player = AudioPlayer::new(CollectingSink::new(48_000, 2));
        ref_player
            .mixer_mut()
            .play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        ref_player.tick(&AudioListener::new(Vec3::ZERO), 8);

        // 音乐目标满音量 + Music 通道 0 → 输出应与「音乐静音」参考逐样本一致
        let mut muted = AudioPlayer::new(CollectingSink::new(48_000, 2));
        muted.set_music_target(1.0);
        muted.mixer_mut().set_channel_volume(Channel::Music, 0.0);
        muted
            .mixer_mut()
            .play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        muted.tick(&AudioListener::new(Vec3::ZERO), 8);

        // 音乐通道 1.0 → 音乐进入输出（与静音参考不同）
        let mut full = AudioPlayer::new(CollectingSink::new(48_000, 2));
        full.set_music_target(1.0);
        full.mixer_mut().set_channel_volume(Channel::Music, 1.0);
        full
            .mixer_mut()
            .play(clip.clone(), AudioSource::new(Vec3::ZERO, 1.0), Channel::Sfx, false);
        full.tick(&AudioListener::new(Vec3::ZERO), 8);

        assert_eq!(
            muted.sink().samples,
            ref_player.sink().samples,
            "Music 通道 0 → 与无音乐参考一致"
        );
        assert_ne!(
            full.sink().samples,
            muted.sink().samples,
            "Music 通道 1 → 音乐应进入输出"
        );
        assert!(full.sink().samples.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn audio_player_music_tick_frames_and_silent_sink_no_panic() {
        // 静默后端 + 音乐淡入：tick 多帧不 panic，增益单调收敛到 target（采样率/帧数正确）
        let mut player = AudioPlayer::new(SilentSink::new(48_000, 2));
        player.set_music_target(0.8);
        let mut prev = player.music_gain();
        for _ in 0..300 {
            player.tick(&AudioListener::new(Vec3::ZERO), 256);
            let cur = player.music_gain();
            assert!(cur >= prev, "tick 中音乐增益应单调逼近 target");
            assert!(cur <= 0.8 + 1e-6, "增益不得越过 target");
            prev = cur;
        }
        // 300×256 帧 = 1.6s > 1.5s 淡入时长 → 应已收敛到 0.8
        assert!((player.music_gain() - 0.8).abs() < 1e-3, "淡入完成应达 target");
    }

    #[test]
    fn dsp_synth_music_does_not_break_sfx_chain() {
        // 默认静音下现有事件合成链路不变（枪声有输出、结束后静音）
        let listener = AudioListener::new(Vec3::ZERO);
        let mut synth = DspSynth::new(48_000);
        synth.play_shot(Vec3::ZERO, 1.0);
        // 枪声 ADSR 约 0.122s：一次渲染 0.175s 播完整个枪声
        let mut out = vec![0.0; 16_800];
        synth.render(&listener, 8400, &mut out);
        assert!(out.iter().any(|&s| s != 0.0), "枪声链路不受音乐影响");
        assert_eq!(synth.voice_count(), 0, "枪声播完声部应清理");
        let mut rest = vec![0.0; 4800];
        synth.render(&listener, 2400, &mut rest);
        assert!(rest.iter().all(|&s| s == 0.0), "默认音乐静音：结束后输出全零");
        // 开启音乐：音乐叠加进同一缓冲，且样本仍有界
        let mut synth2 = DspSynth::new(48_000);
        synth2.play_shot(Vec3::ZERO, 1.0);
        synth2.set_music_target(1.0);
        let mut out2 = vec![0.0; 4800];
        synth2.render(&listener, 2400, &mut out2);
        assert!(out2.iter().any(|&s| s != 0.0), "音乐开启后输出应含音乐");
        assert!(out2.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }

    // ---------- 枪声参数化（M1 / Thompson）与手榴弹投掷/落地音 ----------

    /// 统计缓冲的过零率（近似高频含量；正弦窄带音低、宽带噪声高）
    fn zero_crossing_count(buf: &[f32]) -> usize {
        let mut count = 0usize;
        let mut prev = 0.0f32;
        for &s in buf {
            if (prev < 0.0 && s >= 0.0) || (prev >= 0.0 && s < 0.0) {
                count += 1;
            }
            prev = s;
        }
        count
    }

    #[test]
    fn shot_params_m1_and_thompson_differ() {
        // M1 与 Thompson 音色参数必须可区分（音高/时长/闷度至少一项不同）
        assert_ne!(M1_SHOT.pitch, THOMPSON_SHOT.pitch, "音高应不同（M1 高脆 / Thompson 低闷）");
        assert_ne!(
            M1_SHOT.duration, THOMPSON_SHOT.duration,
            "时长应不同（Thompson 尾巴更长）"
        );
        assert_ne!(M1_SHOT.cutoff, THOMPSON_SHOT.cutoff, "低通应不同（Thompson 更闷）");
        assert_ne!(M1_SHOT, THOMPSON_SHOT, "参数集整体应不同");
        // 参数域合理性：爆鸣/噪声幅度有界、时长为正
        assert!(M1_SHOT.thump_gain >= 0.0 && M1_SHOT.thump_gain <= 1.0);
        assert!(THOMPSON_SHOT.thump_gain > 0.0, "Thompson 应有低频爆鸣分量");
        assert!(M1_SHOT.noise_scale > 0.0 && THOMPSON_SHOT.noise_scale > 0.0);
    }

    #[test]
    fn play_shot_delegates_to_m1_params() {
        // 旧签名 play_shot 委托 M1_SHOT：与 play_shot_with(M1_SHOT) 逐样本一致，输出非全零
        let listener = AudioListener::new(Vec3::ZERO);
        let mut a = DspSynth::new(48_000);
        let mut b = DspSynth::new(48_000);
        a.play_shot(Vec3::ZERO, 0.8);
        b.play_shot_with(Vec3::ZERO, 0.8, M1_SHOT);
        let mut oa = vec![0.0; 9600];
        let mut ob = vec![0.0; 9600];
        a.render(&listener, 4800, &mut oa);
        b.render(&listener, 4800, &mut ob);
        assert_eq!(oa, ob, "play_shot 应委托 M1_SHOT 参数（逐位一致）");
        assert!(oa.iter().any(|&s| s != 0.0), "M1 枪声输出应非全零");
    }

    #[test]
    fn shot_with_params_deterministic() {
        // 同参数同事件序列 → 逐样本一致
        let listener = AudioListener::new(Vec3::ZERO);
        let mut a = DspSynth::new(48_000);
        let mut b = DspSynth::new(48_000);
        for s in [&mut a, &mut b] {
            s.play_shot_with(Vec3::ZERO, 0.8, THOMPSON_SHOT);
        }
        let mut oa = vec![0.0; 9600];
        let mut ob = vec![0.0; 9600];
        a.render(&listener, 4800, &mut oa);
        b.render(&listener, 4800, &mut ob);
        assert_eq!(oa, ob, "Thompson 同参数应逐样本一致");
    }

    #[test]
    fn thompson_has_longer_tail_than_m1() {
        // 0.16s 时：M1（duration 0.12 + release ≈ 0.14s 总长）已播完、Thompson（0.17s + release）仍活跃
        let listener = AudioListener::new(Vec3::ZERO);
        let mut m1 = DspSynth::new(48_000);
        m1.play_shot_with(Vec3::ZERO, 0.8, M1_SHOT);
        m1.render(&listener, 4800, &mut vec![0.0; 9600]); // 0.1s
        m1.render(&listener, 2880, &mut vec![0.0; 5760]); // 0.16s
        assert_eq!(m1.voice_count(), 0, "M1 0.16s 内应播完");
        let mut th = DspSynth::new(48_000);
        th.play_shot_with(Vec3::ZERO, 0.8, THOMPSON_SHOT);
        th.render(&listener, 4800, &mut vec![0.0; 9600]); // 0.1s
        th.render(&listener, 2880, &mut vec![0.0; 5760]); // 0.16s
        assert!(th.voice_count() >= 1, "Thompson 0.16s 时仍活跃（尾巴更长）");
    }

    #[test]
    fn grenade_whistle_deterministic_and_bounded() {
        // 哨声确定性 + 输出有界（|x|≤1 无 NaN/Inf）+ 非全零
        let listener = AudioListener::new(Vec3::ZERO);
        let mut a = DspSynth::new(48_000);
        let mut b = DspSynth::new(48_000);
        a.play_grenade_throw(Vec3::ZERO);
        b.play_grenade_throw(Vec3::ZERO);
        let mut oa = vec![0.0; 24_000];
        let mut ob = vec![0.0; 24_000];
        a.render(&listener, 12_000, &mut oa);
        b.render(&listener, 12_000, &mut ob);
        assert_eq!(oa, ob, "哨声合成应确定性（同参数同输出）");
        for (i, &s) in oa.iter().enumerate() {
            assert!(s.is_finite(), "sample {i} 应有限");
            assert!(s.abs() <= 1.0, "sample {i} = {s} 超出 [-1,1]");
        }
        assert!(oa.iter().any(|&s| s != 0.0), "哨声应有输出");
    }

    #[test]
    fn grenade_whistle_differs_from_shot() {
        // 哨声 vs 枪声：样本不同 + 高频含量差异（哨声正弦窄带过零低、枪声宽带噪声过零高）
        let listener = AudioListener::new(Vec3::ZERO);
        let mut shot = DspSynth::new(48_000);
        shot.play_shot_with(Vec3::ZERO, 0.8, M1_SHOT);
        let mut sbuf = vec![0.0; 4800];
        shot.render(&listener, 2400, &mut sbuf);
        let mut whis = DspSynth::new(48_000);
        whis.play_grenade_throw(Vec3::ZERO);
        let mut wbuf = vec![0.0; 4800];
        whis.render(&listener, 2400, &mut wbuf);
        assert_ne!(sbuf, wbuf, "哨声与枪声样本应不同");
        let shot_zcr = zero_crossing_count(&sbuf);
        let whis_zcr = zero_crossing_count(&wbuf);
        assert!(shot_zcr > 300, "枪声宽带噪声过零应较多，实际 {shot_zcr}");
        assert!(whis_zcr < 500, "哨声正弦窄带过零应较少，实际 {whis_zcr}");
        assert!(whis_zcr < shot_zcr, "哨声高频含量应低于枪声（{whis_zcr} vs {shot_zcr}）");
    }

    #[test]
    fn grenade_bounce_output_and_lifetime() {
        // 落地 thud：有输出、有界、~0.1s 后结束
        let listener = AudioListener::new(Vec3::ZERO);
        let mut s = DspSynth::new(48_000);
        s.play_grenade_bounce(Vec3::ZERO);
        let mut buf = vec![0.0; 9600];
        s.render(&listener, 4800, &mut buf);
        assert!(buf.iter().any(|&x| x != 0.0), "落地音应有输出");
        assert!(buf.iter().all(|&x| x.is_finite() && x.abs() <= 1.0));
        assert!(s.voice_count() >= 1, "0.1s 时 thud 仍在衰减尾段");
        let mut rest = vec![0.0; 9600];
        s.render(&listener, 4800, &mut rest);
        assert_eq!(s.voice_count(), 0, "0.2s 后落地音应结束");
    }

    #[test]
    fn grenade_whistle_multi_tick_no_panic_and_sample_rate() {
        // 哨声分块 tick 不 panic；采样率正确；声部生命周期正常
        let listener = AudioListener::new(Vec3::ZERO);
        let mut s = DspSynth::new(48_000);
        assert_eq!(s.sample_rate, 48_000.0);
        s.play_grenade_throw(Vec3::ZERO);
        let mut total = vec![0.0f32; 0];
        for _ in 0..20 {
            let mut buf = vec![0.0f32; 480];
            s.render(&listener, 240, &mut buf);
            total.extend_from_slice(&buf);
        }
        assert!(total.iter().any(|&x| x != 0.0), "0.1s 分块渲染应有输出");
        assert!(total.iter().all(|&x| x.is_finite() && x.abs() <= 1.0));
        // 哨声 ADSR 约 0.21s：再渲染 0.15s（累计 0.25s）后应已结束
        for _ in 0..30 {
            let mut buf = vec![0.0f32; 480];
            s.render(&listener, 240, &mut buf);
        }
        assert_eq!(s.voice_count(), 0, "0.25s 后哨声应结束");
    }
}
