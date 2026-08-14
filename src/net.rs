//! 网络同步模块
//!
//! - UDP 客户端-服务器架构：基于 `std::net::UdpSocket` 封装 `Server` / `Client`，
//!   loopback 即可测试，无需真正联网
//! - 玩家位置/旋转序列化：手写字节编码（f32 位置 + f32 旋转），带协议头
//!   （magic / version / type / length），大端网络字节序
//! - 远端玩家插值：`lerp_state` 纯函数 + `RemotePlayer` 时间戳插值平滑
//! - `NetworkMessage` 枚举：Join / Leave / Position / Action / Input / Snapshot /
//!   ObjectiveState，支持序列化往返
//!
//! # 对战协议（RV3D_NET=server|client）
//!
//! 报文格式：`magic(1) + version(1) + type(1) + length(2, BE) + payload`，
//! 单条数据报 ≤ MAX_DATAGRAM（UDP 载荷上限 65507），全部大端网络字节序。
//!
//! - `Join(0x01)`：客户端申请加入（player_id=0）→ 服务端回发分配后的 id 确认
//! - `Input(0x06)`：客户端每 tick 上报输入/姿态（移动标志 + 开火 + yaw/pitch）
//! - `Snapshot(0x05)`：服务端每 tick 广播整帧快照（seq + 服务端时间 + 本机玩家 + NPC 列表）
//! - `ObjectiveState(0x07)`：服务端广播目标状态（据点 id/归属码/进度，seq + 时间 +
//!   rule_kind + 据点列表）；为多人在线铺路，客户端应用逻辑由上层桥接
//! - `Leave(0x02)`：正常退出；`Position(0x03)` / `Action(0x04)` 为早期演示消息，保留兼容
//!
//! 向后兼容：`decode` 对未知 MessageType 返回 `UnknownMessageType` 错误（见 `NetError`）。
//! 旧客户端（不认识 0x07）收到 ObjectiveState 时同样走该路径——由调用方按需忽略新消息；
//! 新客户端收到旧版报文则按各自分支正常解码，故协议版本号无需递增。
//!
//! 序列号与顺序：Input / Snapshot 各自携带单调递增 seq（u32 wrapping）；接收方用
//! `wrapping_sub` 丢弃乱序与重复（差值 ≥ 2^31 视为过期）。
//!
//! 可靠性（UDP 尽力而为，本轮不做复杂可靠传输）：
//! - 快照/输入不重传——丢包直接跳过，下一帧新数据覆盖（游戏状态本身是自描述的）
//! - 唯一带重试的报文是 Join 握手：客户端每 0.5s 重发直到确认（`retry_join`）
//! - 超时判定：客户端 3s 未收到任何数据报视为断线（`snapshot_timeout`）；
//!   服务端 5s 未收到某客户端数据报则移除其注册（`timeout_clients`）
//!
//! 限制与后续 TODO：单数据报不做分片/重组（快照 NPC 上限 MAX_SNAPSHOT_NPCS，超出截断）；
//! NAT 穿透、断线重连/恢复、真实两机实战场、快照增量压缩、输入预测/回滚均未实现。
//!
//! 本模块仅使用 `std`，不引入外部依赖；如将来需要新依赖，在文件头部按
//! `// DEP: crate = version` 声明。
//! 网络层由 RV3D_NET env 开关接入 main.rs/game.rs，未启用时整体允许 dead_code 警告。

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

/// 协议魔数（'S' = Steel Front）
pub const PROTOCOL_MAGIC: u8 = 0x53;
/// 协议版本号
pub const PROTOCOL_VERSION: u8 = 0x01;
/// 头部长度：magic(1) + version(1) + type(1) + length(2, BE)
pub const HEADER_LEN: usize = 5;
/// 单条 UDP 数据报读取缓冲上限：取 UDP 载荷上限 65507（IPv4）。
/// 快照需承载 64v64 压力模式全部 NPC（128 × 24B ≈ 3KB），1400 的 MTU 余量会截断快照；
/// 局域网碎片化可接受，本轮不做分片/重组（见模块头部协议注释）。
pub const MAX_DATAGRAM: usize = 65507;
/// 单条快照最多携带的实体数：超出截断（保护 length 字段 u16 上限；本轮 128 NPC 远低于此）
pub const MAX_SNAPSHOT_NPCS: usize = 1024;
/// 单条目标状态最多携带的据点数：超出截断（单关据点通常 1-5 个，64 为防御性上限；
/// 每个据点约 6-261B，64 个最坏约 17KB，远低于 MAX_DATAGRAM）
pub const MAX_OBJECTIVE_POINTS: usize = 64;
/// 客户端断线判定：超过该时长未收到任何数据报视为超时（UDP 尽力而为，无重传）
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(3);
/// 服务端超时判定：客户端超过该时长无数据报则移除注册（断线重连为后续 TODO）
pub const SERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// 消息类型标签（协议第 2 字节）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// 玩家加入
    Join = 0x01,
    /// 玩家离开
    Leave = 0x02,
    /// 玩家位置/旋转同步
    Position = 0x03,
    /// 玩家动作
    Action = 0x04,
    /// 服务端 → 客户端：整帧快照（本机玩家 + NPC 列表）
    Snapshot = 0x05,
    /// 客户端 → 服务端：本帧输入/姿态
    Input = 0x06,
    /// 服务端 → 客户端：目标状态（据点归属/进度）同步
    ObjectiveState = 0x07,
}

impl MessageType {
    /// 从字节还原消息类型
    pub fn from_byte(b: u8) -> Option<MessageType> {
        Some(match b {
            0x01 => MessageType::Join,
            0x02 => MessageType::Leave,
            0x03 => MessageType::Position,
            0x04 => MessageType::Action,
            0x05 => MessageType::Snapshot,
            0x06 => MessageType::Input,
            0x07 => MessageType::ObjectiveState,
            _ => return None,
        })
    }
}

/// 玩家状态：三维位置 + 偏航旋转（弧度）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerState {
    /// 位置 (x, y, z)
    pub pos: [f32; 3],
    /// 偏航角（弧度）
    pub rot: f32,
}

impl PlayerState {
    /// 新建玩家状态
    pub const fn new(pos: [f32; 3], rot: f32) -> Self {
        Self { pos, rot }
    }
}

/// NPC 快照条目：id + 位置 + 朝向 + 血量（存活 = hp > 0.0；插值只做位置/朝向平滑）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NpcSnapshot {
    /// 实体 id（服务端 `Npc.id`，与玩家 id 同空间，客户端插值表共用）
    pub id: u32,
    /// 世界位置 (x, y, z)
    pub pos: [f32; 3],
    /// 朝向角（绕 Y 轴，弧度）
    pub facing: f32,
    /// 当前血量
    pub hp: f32,
}

/// 客户端输入/姿态（每 tick 随 Input 上报）：移动标志 + 开火 + 视角（弧度）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetInput {
    /// 前进（W）
    pub forward: bool,
    /// 后退（S）
    pub backward: bool,
    /// 左移（A）
    pub left: bool,
    /// 右移（D）
    pub right: bool,
    /// 开火请求（服务端按武器冷却应用）
    pub fire: bool,
    /// 偏航角（弧度）
    pub yaw: f32,
    /// 俯仰角（弧度）
    pub pitch: f32,
}

/// 网络消息：Join / Leave / Position / Action / Snapshot / Input
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkMessage {
    /// 玩家加入；`player_id == 0` 表示申请加入，服务端回复分配后的 id
    Join { player_id: u32, name: String },
    /// 玩家离开；reason：0=正常退出，1=超时，2=被踢
    Leave { player_id: u32, reason: u8 },
    /// 玩家状态同步；seq 为发送方单调递增序号
    Position { player_id: u32, seq: u32, state: PlayerState },
    /// 玩家动作；action_id 为动作类型，value 为标量参数（数据载荷）
    Action { player_id: u32, action_id: u8, value: f32 },
    /// 服务端 → 客户端：整帧快照。seq = 快照序号，time = 服务端模拟时间（秒）；
    /// player_id/player = 服务端本机玩家（客户端可用作权威修正），npcs = 在场 NPC
    Snapshot {
        seq: u32,
        time: f32,
        player_id: u32,
        player: PlayerState,
        npcs: Vec<NpcSnapshot>,
    },
    /// 客户端 → 服务端：本帧输入。seq = 客户端输入序号（wrapping），time = 客户端本地时间（秒）
    Input { seq: u32, time: f32, input: NetInput },
    /// 服务端 → 客户端：目标状态（据点归属/进度）同步。seq = 发送方单调递增序号，
    /// time = 发送方模拟时间（秒），rule_kind = 关卡规则标识（如 "CapturePoints"），
    /// points = 各据点 (id, 归属码, 进度)，归属码约定：0=中立/None、1=Red、2=Blue
    /// （u8 纯数据，避免跨模块依赖 Team 枚举）。
    /// 注意：本消息仅供多人在线铺路，客户端应用逻辑由上层桥接，本模块只做编解码。
    ObjectiveState {
        seq: u32,
        time: f32,
        rule_kind: String,
        points: Vec<(String, u8, f32)>,
    },
}

/// 协议解码错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// 魔数不匹配
    InvalidMagic,
    /// 协议版本不支持
    UnsupportedVersion(u8),
    /// 未知消息类型
    UnknownMessageType(u8),
    /// 数据截断或长度与头部声明不符
    Truncated,
    /// 载荷尾部存在多余字节
    TrailingData,
    /// 字符串字段（rule_kind / 据点 id）不是合法 UTF-8
    InvalidUtf8,
    /// 名字不是合法 UTF-8
    InvalidName,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::InvalidMagic => write!(f, "invalid protocol magic"),
            NetError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            NetError::UnknownMessageType(t) => write!(f, "unknown message type {t:#x}"),
            NetError::Truncated => write!(f, "truncated datagram or length mismatch"),
            NetError::TrailingData => write!(f, "trailing bytes after payload"),
            NetError::InvalidUtf8 => write!(f, "string field is not valid UTF-8"),
            NetError::InvalidName => write!(f, "player name is not valid UTF-8"),
        }
    }
}

impl std::error::Error for NetError {}

// ---------------------------------------------------------------------------
// 序列化 / 反序列化
// ---------------------------------------------------------------------------

/// 编码辅助：大端写入
fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn put_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_bits().to_be_bytes());
}

/// 解码辅助：带边界检查的顺序读取器
struct Reader<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, off: 0 }
    }

    fn u8(&mut self) -> Result<u8, NetError> {
        let b = *self.buf.get(self.off).ok_or(NetError::Truncated)?;
        self.off += 1;
        Ok(b)
    }

    fn u32(&mut self) -> Result<u32, NetError> {
        let a = [self.u8()?, self.u8()?, self.u8()?, self.u8()?];
        Ok(u32::from_be_bytes(a))
    }

    fn f32(&mut self) -> Result<f32, NetError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], NetError> {
        if self.off + n > self.buf.len() {
            return Err(NetError::Truncated);
        }
        let s = &self.buf[self.off..self.off + n];
        self.off += n;
        Ok(s)
    }

    /// 确认载荷已消费完毕
    fn finish(&self) -> Result<(), NetError> {
        if self.off == self.buf.len() {
            Ok(())
        } else {
            Err(NetError::TrailingData)
        }
    }
}

impl NetworkMessage {
    /// 编码为带协议头的数据报字节（大端）
    pub fn encode(&self) -> Vec<u8> {
        let (ty, payload) = match self {
            NetworkMessage::Join { player_id, name } => {
                let name = name.as_bytes();
                let n = name.len().min(255);
                let mut p = Vec::with_capacity(5 + n);
                put_u32(&mut p, *player_id);
                p.push(n as u8);
                p.extend_from_slice(&name[..n]);
                (MessageType::Join, p)
            }
            NetworkMessage::Leave { player_id, reason } => {
                let mut p = Vec::with_capacity(5);
                put_u32(&mut p, *player_id);
                p.push(*reason);
                (MessageType::Leave, p)
            }
            NetworkMessage::Position { player_id, seq, state } => {
                let mut p = Vec::with_capacity(24);
                put_u32(&mut p, *player_id);
                put_u32(&mut p, *seq);
                for c in state.pos {
                    put_f32(&mut p, c);
                }
                put_f32(&mut p, state.rot);
                (MessageType::Position, p)
            }
            NetworkMessage::Action { player_id, action_id, value } => {
                let mut p = Vec::with_capacity(9);
                put_u32(&mut p, *player_id);
                p.push(*action_id);
                put_f32(&mut p, *value);
                (MessageType::Action, p)
            }
            NetworkMessage::Snapshot { seq, time, player_id, player, npcs } => {
                // 固定头：seq(4) + time(4) + player_id(4) + player(16) + count(2)
                let n = npcs.len().min(MAX_SNAPSHOT_NPCS) as u16;
                let mut p = Vec::with_capacity(4 + 4 + 4 + 16 + 2 + n as usize * 24);
                put_u32(&mut p, *seq);
                put_f32(&mut p, *time);
                put_u32(&mut p, *player_id);
                for c in player.pos {
                    put_f32(&mut p, c);
                }
                put_f32(&mut p, player.rot);
                p.extend_from_slice(&n.to_be_bytes());
                // 每个 NPC：id(4) + pos(12) + facing(4) + hp(4) = 24B
                for npc in npcs.iter().take(MAX_SNAPSHOT_NPCS) {
                    put_u32(&mut p, npc.id);
                    for c in npc.pos {
                        put_f32(&mut p, c);
                    }
                    put_f32(&mut p, npc.facing);
                    put_f32(&mut p, npc.hp);
                }
                (MessageType::Snapshot, p)
            }
            NetworkMessage::Input { seq, time, input } => {
                // seq(4) + time(4) + 5×bool + yaw(4) + pitch(4) = 21B
                let mut p = Vec::with_capacity(21);
                put_u32(&mut p, *seq);
                put_f32(&mut p, *time);
                p.push(input.forward as u8);
                p.push(input.backward as u8);
                p.push(input.left as u8);
                p.push(input.right as u8);
                p.push(input.fire as u8);
                put_f32(&mut p, input.yaw);
                put_f32(&mut p, input.pitch);
                (MessageType::Input, p)
            }
            NetworkMessage::ObjectiveState { seq, time, rule_kind, points } => {
                // 布局：seq(4) + time(4) + rule_len(1) + rule(≤255) + count(2) +
                // 每据点 [id_len(1) + id(≤255) + owner(1) + progress(4)]
                let rule = rule_kind.as_bytes();
                let rule = &rule[..rule.len().min(255)];
                let n = points.len().min(MAX_OBJECTIVE_POINTS) as u16;
                let mut p = Vec::with_capacity(4 + 4 + 1 + rule.len() + 2 + n as usize * 7);
                put_u32(&mut p, *seq);
                put_f32(&mut p, *time);
                p.push(rule.len() as u8);
                p.extend_from_slice(rule);
                p.extend_from_slice(&n.to_be_bytes());
                for (id, owner, progress) in points.iter().take(MAX_OBJECTIVE_POINTS) {
                    let idb = id.as_bytes();
                    let idb = &idb[..idb.len().min(255)];
                    p.push(idb.len() as u8);
                    p.extend_from_slice(idb);
                    p.push(*owner);
                    put_f32(&mut p, *progress);
                }
                (MessageType::ObjectiveState, p)
            }
        };

        let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
        buf.push(PROTOCOL_MAGIC);
        buf.push(PROTOCOL_VERSION);
        buf.push(ty as u8);
        buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        buf.extend_from_slice(&payload);
        buf
    }

    /// 从数据报字节解码消息，校验协议头与长度
    pub fn decode(buf: &[u8]) -> Result<NetworkMessage, NetError> {
        if buf.len() < HEADER_LEN {
            return Err(NetError::Truncated);
        }
        if buf[0] != PROTOCOL_MAGIC {
            return Err(NetError::InvalidMagic);
        }
        if buf[1] != PROTOCOL_VERSION {
            return Err(NetError::UnsupportedVersion(buf[1]));
        }
        let ty = MessageType::from_byte(buf[2]).ok_or(NetError::UnknownMessageType(buf[2]))?;
        let payload_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
        let total = HEADER_LEN + payload_len;
        if buf.len() < total {
            return Err(NetError::Truncated);
        }
        if buf.len() > total {
            return Err(NetError::TrailingData);
        }
        let mut r = Reader::new(&buf[HEADER_LEN..]);
        let msg = match ty {
            MessageType::Join => {
                let player_id = r.u32()?;
                let name_len = r.u8()? as usize;
                let name = String::from_utf8(r.bytes(name_len)?.to_vec())
                    .map_err(|_| NetError::InvalidName)?;
                NetworkMessage::Join { player_id, name }
            }
            MessageType::Leave => {
                let player_id = r.u32()?;
                let reason = r.u8()?;
                NetworkMessage::Leave { player_id, reason }
            }
            MessageType::Position => {
                let player_id = r.u32()?;
                let seq = r.u32()?;
                let pos = [r.f32()?, r.f32()?, r.f32()?];
                let rot = r.f32()?;
                NetworkMessage::Position {
                    player_id,
                    seq,
                    state: PlayerState::new(pos, rot),
                }
            }
            MessageType::Action => {
                let player_id = r.u32()?;
                let action_id = r.u8()?;
                let value = r.f32()?;
                NetworkMessage::Action {
                    player_id,
                    action_id,
                    value,
                }
            }
            MessageType::Snapshot => {
                let seq = r.u32()?;
                let time = r.f32()?;
                let player_id = r.u32()?;
                let pos = [r.f32()?, r.f32()?, r.f32()?];
                let rot = r.f32()?;
                let n = u16::from_be_bytes([r.u8()?, r.u8()?]) as usize;
                let mut npcs = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u32()?;
                    let pos = [r.f32()?, r.f32()?, r.f32()?];
                    let facing = r.f32()?;
                    let hp = r.f32()?;
                    npcs.push(NpcSnapshot { id, pos, facing, hp });
                }
                NetworkMessage::Snapshot {
                    seq,
                    time,
                    player_id,
                    player: PlayerState::new(pos, rot),
                    npcs,
                }
            }
            MessageType::Input => {
                let seq = r.u32()?;
                let time = r.f32()?;
                let input = NetInput {
                    forward: r.u8()? != 0,
                    backward: r.u8()? != 0,
                    left: r.u8()? != 0,
                    right: r.u8()? != 0,
                    fire: r.u8()? != 0,
                    yaw: r.f32()?,
                    pitch: r.f32()?,
                };
                NetworkMessage::Input { seq, time, input }
            }
            MessageType::ObjectiveState => {
                let seq = r.u32()?;
                let time = r.f32()?;
                let rule_len = r.u8()? as usize;
                let rule_kind = String::from_utf8(r.bytes(rule_len)?.to_vec())
                    .map_err(|_| NetError::InvalidUtf8)?;
                // count 为 u16：合法编码上限是 MAX_OBJECTIVE_POINTS（encode 侧截断）；
                // 恶意/伪造的超大 count 会被后续逐据点边界检查以 Truncated 拒绝，不会 panic。
                // 初始容量按上限预留，防止攻击者仅凭 2 字节 count 触发大分配。
                let n = u16::from_be_bytes([r.u8()?, r.u8()?]) as usize;
                let mut points = Vec::with_capacity(n.min(MAX_OBJECTIVE_POINTS));
                for _ in 0..n {
                    let id_len = r.u8()? as usize;
                    let id = String::from_utf8(r.bytes(id_len)?.to_vec())
                        .map_err(|_| NetError::InvalidUtf8)?;
                    let owner = r.u8()?;
                    let progress = r.f32()?;
                    points.push((id, owner, progress));
                }
                NetworkMessage::ObjectiveState {
                    seq,
                    time,
                    rule_kind,
                    points,
                }
            }
        };
        r.finish()?;
        Ok(msg)
    }
}

// ---------------------------------------------------------------------------
// 插值
// ---------------------------------------------------------------------------

/// 将角度差归一化到 [-PI, PI]，取最短旋转弧
pub fn wrap_angle(a: f32) -> f32 {
    let two_pi = 2.0 * std::f32::consts::PI;
    let mut x = a % two_pi;
    if x > std::f32::consts::PI {
        x -= two_pi;
    } else if x < -std::f32::consts::PI {
        x += two_pi;
    }
    x
}

/// 纯函数：在上一状态与当前状态间按 alpha(0..=1) 线性插值；
/// 位置逐分量 lerp，旋转走最短弧
pub fn lerp_state(prev: PlayerState, curr: PlayerState, alpha: f32) -> PlayerState {
    let a = alpha.clamp(0.0, 1.0);
    if a <= 0.0 {
        return prev;
    }
    if a >= 1.0 {
        return curr;
    }
    let l = |x: f32, y: f32| x + (y - x) * a;
    PlayerState {
        pos: [
            l(prev.pos[0], curr.pos[0]),
            l(prev.pos[1], curr.pos[1]),
            l(prev.pos[2], curr.pos[2]),
        ],
        rot: prev.rot + wrap_angle(curr.rot - prev.rot) * a,
    }
}

/// 纯函数：给定上一状态/当前状态及各自时间戳，返回时刻 t 的平滑状态。
/// t 超出区间时 clamp 到端点；时间戳倒退或相同时返回当前状态
pub fn interpolate_at(
    prev: PlayerState,
    prev_time: f64,
    curr: PlayerState,
    curr_time: f64,
    t: f64,
) -> PlayerState {
    let dt = curr_time - prev_time;
    if dt <= 0.0 {
        return curr;
    }
    let alpha = ((t - prev_time) / dt) as f32;
    lerp_state(prev, curr, alpha)
}

/// 远端玩家：维护上一/当前状态与时间戳，按本地时间插值出渲染状态
#[derive(Debug, Clone)]
pub struct RemotePlayer {
    /// 远端玩家 id
    pub player_id: u32,
    /// 上一状态
    pub prev: PlayerState,
    /// 当前状态
    pub curr: PlayerState,
    /// 上一状态到达时间（秒）
    pub prev_time: f64,
    /// 当前状态到达时间（秒）
    pub curr_time: f64,
    /// 网络滞后补偿：渲染时刻 = 本地时刻 - delay（秒）
    pub delay: f64,
}

impl RemotePlayer {
    /// 以首个状态创建远端玩家
    pub fn new(player_id: u32, state: PlayerState, time: f64) -> Self {
        Self {
            player_id,
            prev: state,
            curr: state,
            prev_time: time,
            curr_time: time,
            delay: 0.0,
        }
    }

    /// 收到新状态：当前状态降级为上一状态
    pub fn update(&mut self, state: PlayerState, time: f64) {
        self.prev = self.curr;
        self.prev_time = self.curr_time;
        self.curr = state;
        self.curr_time = time;
    }

    /// 时刻 t 的插值状态（自动 clamp，并扣除 delay 做滞后补偿）
    pub fn state_at(&self, t: f64) -> PlayerState {
        interpolate_at(self.prev, self.prev_time, self.curr, self.curr_time, t - self.delay)
    }
}

/// 远端实体（NPC 或远端玩家）：位置/朝向时间戳插值 + 最近血量
#[derive(Debug, Clone)]
pub struct RemoteEntity {
    /// 位置/朝向插值器（player_id = 实体 id）
    pub state: RemotePlayer,
    /// 最近血量（存活 = hp > 0.0；血量不做插值）
    pub hp: f32,
}

// ---------------------------------------------------------------------------
// UDP Server / Client
// ---------------------------------------------------------------------------

/// UDP 服务器：绑定监听地址，跟踪已注册客户端并为 Join 分配玩家 id
pub struct Server {
    socket: UdpSocket,
    next_player_id: u32,
    clients: HashMap<SocketAddr, u32>,
    /// 各客户端最近一次收到数据报的时刻（超时判定用，recv 内更新）
    last_seen: HashMap<SocketAddr, Instant>,
}

impl Server {
    /// 绑定监听地址（如 "127.0.0.1:0" 随机端口）
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Server> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Server {
            socket,
            next_player_id: 1,
            clients: HashMap::new(),
            last_seen: HashMap::new(),
        })
    }

    /// 本地监听地址（获取随机端口用）
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// 注册客户端地址并分配玩家 id；已注册则返回原 id
    pub fn register(&mut self, addr: SocketAddr) -> u32 {
        *self.clients.entry(addr).or_insert_with(|| {
            let id = self.next_player_id;
            self.next_player_id += 1;
            id
        })
    }

    /// 查询客户端地址对应的玩家 id
    pub fn client_id(&self, addr: SocketAddr) -> Option<u32> {
        self.clients.get(&addr).copied()
    }

    /// 已注册客户端数量
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// 移除超过 `timeout` 未收到任何数据报的客户端，返回被移除的玩家 id。
    /// UDP 尽力而为：超时即丢弃注册（断线重连/恢复为后续 TODO）
    pub fn timeout_clients(&mut self, timeout: Duration) -> Vec<u32> {
        let now = Instant::now();
        let mut removed = Vec::new();
        self.clients.retain(|addr, id| {
            let keep = self
                .last_seen
                .get(addr)
                .map_or(true, |t| now.duration_since(*t) <= timeout);
            if !keep {
                removed.push(*id);
            }
            keep
        });
        self.last_seen.retain(|addr, _| self.clients.contains_key(addr));
        removed
    }

    /// 非阻塞接收：无数据时返回 `Ok(None)`，协议错误映射为 `InvalidData`
    pub fn recv(&mut self) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
        let mut buf = [0u8; MAX_DATAGRAM];
        match self.socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                self.last_seen.insert(from, Instant::now());
                let msg = NetworkMessage::decode(&buf[..n])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some((msg, from)))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 阻塞接收直到超时；超时返回 `Ok(None)`
    pub fn recv_timeout(&mut self, timeout: Duration) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(p) = self.recv()? {
                return Ok(Some(p));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 发送消息给指定客户端
    pub fn send_to(&self, msg: &NetworkMessage, to: SocketAddr) -> io::Result<usize> {
        let buf = msg.encode();
        self.socket.send_to(&buf, to)
    }

    /// 广播给所有已注册客户端；`except` 用于排除发送方
    pub fn broadcast(&self, msg: &NetworkMessage, except: Option<SocketAddr>) -> io::Result<usize> {
        let buf = msg.encode();
        let mut sent = 0;
        for &addr in self.clients.keys() {
            if Some(addr) != except {
                sent += self.socket.send_to(&buf, addr)?;
            }
        }
        Ok(sent)
    }

    /// 处理一条 Join：自动注册该地址，回发分配后的 Join 确认（内部已 `send_to`，调用方无需再发送）
    pub fn handle_join(&mut self, from: SocketAddr, name: String) -> io::Result<NetworkMessage> {
        let player_id = self.register(from);
        let reply = NetworkMessage::Join { player_id, name };
        self.send_to(&reply, from)?;
        Ok(reply)
    }
}

/// UDP 客户端：连接服务器，维护自身 id、远端玩家插值缓冲与快照实体插值表
pub struct Client {
    socket: UdpSocket,
    server: SocketAddr,
    player_id: Option<u32>,
    remote_players: HashMap<u32, RemotePlayer>,
    clock_start: Instant,
    /// 最近快照序号（丢弃乱序/重复）
    snapshot_seq: u32,
    /// 最近快照的服务端模拟时间（秒）
    snapshot_time: f32,
    /// 是否收到过快照
    has_snapshot: bool,
    /// 服务端本机玩家状态（客户端可用作权威修正，应用为后续 TODO）
    own_state: Option<PlayerState>,
    /// 远端实体插值表（NPC / 玩家共用，key = 实体 id）
    entities: HashMap<u32, RemoteEntity>,
    /// 最近收到任何数据报的时刻（超时判定）
    last_rx: Instant,
    /// 断线超时阈值
    timeout: Duration,
    /// 上次发送 Join 的时刻（握手重试节流）
    last_join_at: Instant,
    /// 目标状态（据点归属/进度）：最近收到（id, 归属码 0=中立/1=Red/2=Blue, 进度）
    objective: Vec<(String, u8, f32)>,
    /// 目标状态规则种类（如 "capture"）
    objective_rule: String,
    /// 最近目标状态序号（丢弃乱序/重复）
    objective_seq: u32,
    /// 是否收到过目标状态
    has_objective: bool,
}

impl Client {
    /// 连接服务器（客户端本地绑定 127.0.0.1 随机端口）
    pub fn connect(server: impl ToSocketAddrs) -> io::Result<Client> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true)?;
        let server = server
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no server address"))?;
        Ok(Client {
            socket,
            server,
            player_id: None,
            remote_players: HashMap::new(),
            clock_start: Instant::now(),
            snapshot_seq: 0,
            snapshot_time: 0.0,
            has_snapshot: false,
            own_state: None,
            entities: HashMap::new(),
            last_rx: Instant::now(),
            timeout: CLIENT_TIMEOUT,
            // 首次 retry_join 立即发送握手包
            last_join_at: Instant::now() - Duration::from_secs(3600),
            objective: Vec::new(),
            objective_rule: String::new(),
            objective_seq: 0,
            has_objective: false,
        })
    }

    /// 本地地址
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// 服务器地址
    pub fn server_addr(&self) -> SocketAddr {
        self.server
    }

    /// 自身玩家 id（收到服务端 Join 确认后才有值）
    pub fn player_id(&self) -> Option<u32> {
        self.player_id
    }

    /// 本地单调时钟（秒），用于插值时间戳
    pub fn now(&self) -> f64 {
        self.clock_start.elapsed().as_secs_f64()
    }

    /// 发送消息到服务器
    pub fn send(&self, msg: &NetworkMessage) -> io::Result<usize> {
        let buf = msg.encode();
        self.socket.send_to(&buf, self.server)
    }

    /// 非阻塞接收：无数据时返回 `Ok(None)`
    pub fn recv(&self) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
        let mut buf = [0u8; MAX_DATAGRAM];
        match self.socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                let msg = NetworkMessage::decode(&buf[..n])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some((msg, from)))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 处理一条消息：Join 确认记录自身 id；Position 更新远端玩家插值缓冲；
    /// Snapshot 进入实体插值表并记录本机权威状态；任何消息都刷新 last_rx（断线判定）
    pub fn handle_message(&mut self, msg: NetworkMessage) {
        self.last_rx = Instant::now();
        let t = self.now();
        match msg {
            NetworkMessage::Join { player_id, .. } => {
                if player_id != 0 && self.player_id.is_none() {
                    self.player_id = Some(player_id);
                }
            }
            NetworkMessage::Position { player_id, state, .. } => {
                match self.remote_players.get_mut(&player_id) {
                    Some(rp) => rp.update(state, t),
                    None => {
                        self.remote_players
                            .insert(player_id, RemotePlayer::new(player_id, state, t));
                    }
                }
            }
            NetworkMessage::Snapshot { seq, time, player_id, player, npcs } => {
                // 丢弃乱序/重复快照（seq wrapping 差值 ≥ 2^31 视为过期；相等视为重复）
                if self.has_snapshot {
                    let diff = seq.wrapping_sub(self.snapshot_seq);
                    if diff == 0 || diff >= u32::MAX / 2 {
                        return;
                    }
                }
                self.has_snapshot = true;
                self.snapshot_seq = seq;
                self.snapshot_time = time;
                // 服务端本机玩家：进插值表（渲染用）+ 权威状态缓存（修正用）
                self.own_state = Some(player);
                match self.entities.get_mut(&player_id) {
                    Some(e) => e.state.update(player, t),
                    None => {
                        self.entities.insert(
                            player_id,
                            RemoteEntity {
                                state: RemotePlayer::new(player_id, player, t),
                                hp: 100.0,
                            },
                        );
                    }
                }
                // NPC：位置/朝向进插值表，血量取最新值
                for npc in npcs {
                    let nstate = PlayerState::new(npc.pos, npc.facing);
                    match self.entities.get_mut(&npc.id) {
                        Some(e) => {
                            e.state.update(nstate, t);
                            e.hp = npc.hp;
                        }
                        None => {
                            self.entities.insert(
                                npc.id,
                                RemoteEntity {
                                    state: RemotePlayer::new(npc.id, nstate, t),
                                    hp: npc.hp,
                                },
                            );
                        }
                    }
                }
            }
            NetworkMessage::ObjectiveState { seq, rule_kind, points, .. } => {
                // 目标状态（据点归属/进度）：乱序/重复丢弃（与 Snapshot 同策略）
                if self.has_objective {
                    let diff = seq.wrapping_sub(self.objective_seq);
                    if diff == 0 || diff >= u32::MAX / 2 {
                        return;
                    }
                }
                self.has_objective = true;
                self.objective_seq = seq;
                self.objective_rule = rule_kind;
                self.objective = points;
            }
            _ => {}
        }
    }

    /// 远端玩家在本地时刻 t 的插值状态
    pub fn remote_state_at(&self, player_id: u32, t: f64) -> Option<PlayerState> {
        self.remote_players.get(&player_id).map(|rp| rp.state_at(t))
    }

    /// 远端玩家插值缓冲（只读）
    pub fn remote_players(&self) -> &HashMap<u32, RemotePlayer> {
        &self.remote_players
    }

    /// 最近快照序号（0 = 尚未收到快照）
    pub fn snapshot_seq(&self) -> u32 {
        self.snapshot_seq
    }

    /// 最近快照的服务端模拟时间（秒）
    pub fn snapshot_time(&self) -> f32 {
        self.snapshot_time
    }

    /// 服务端本机玩家状态（快照权威值；客户端可用作位置修正，应用为后续 TODO）
    pub fn own_state(&self) -> Option<PlayerState> {
        self.own_state
    }

    /// 最近收到的目标状态（据点 id, 归属码 0=中立/1=Red/2=Blue, 进度 0..=1）
    pub fn objective_state(&self) -> &[(String, u8, f32)] {
        &self.objective
    }

    /// 最近目标状态的规则种类（如 "capture"；未收到时为空串）
    pub fn objective_rule(&self) -> &str {
        &self.objective_rule
    }

    /// 是否收到过目标状态
    pub fn has_objective(&self) -> bool {
        self.has_objective
    }

    /// 远端实体插值表（key = 实体 id；NPC 与玩家共用）
    pub fn entities(&self) -> &HashMap<u32, RemoteEntity> {
        &self.entities
    }

    /// 实体 id 在本地时刻 t 的插值状态（位置平滑，渲染消费为后续 TODO）
    pub fn entity_state_at(&self, id: u32, t: f64) -> Option<PlayerState> {
        self.entities.get(&id).map(|e| e.state.state_at(t))
    }

    /// 超过超时阈值未收到任何数据报（断线基本判定；重连为后续 TODO）
    pub fn snapshot_timeout(&self) -> bool {
        self.last_rx.elapsed() > self.timeout
    }

    /// 已加入且未超时（UDP 尽力而为：超时即视为断线，不做重传）
    pub fn is_connected(&self) -> bool {
        self.player_id.is_some() && !self.snapshot_timeout()
    }

    /// 握手重试：未收到 Join 确认时按 `interval` 间隔重发（UDP 尽力而为下
    /// 唯一带重试的报文；快照/输入不重传，靠序列号丢弃乱序与重复）。
    /// 已确认时返回 true。
    pub fn retry_join(&mut self, name: &str, interval: Duration) -> bool {
        if self.player_id.is_some() {
            return true;
        }
        if self.last_join_at.elapsed() >= interval {
            let _ = self.send(&NetworkMessage::Join {
                player_id: 0,
                name: name.to_string(),
            });
            self.last_join_at = Instant::now();
        }
        false
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f32, y: f32, z: f32, rot: f32) -> PlayerState {
        PlayerState::new([x, y, z], rot)
    }

    fn sample_messages() -> Vec<NetworkMessage> {
        vec![
            NetworkMessage::Join {
                player_id: 0,
                name: String::new(),
            },
            NetworkMessage::Join {
                player_id: 42,
                name: "alice".to_string(),
            },
            NetworkMessage::Leave {
                player_id: 7,
                reason: 1,
            },
            NetworkMessage::Position {
                player_id: 3,
                seq: 123,
                state: state(1.5, -2.25, 3.75, 1.2345),
            },
            NetworkMessage::Action {
                player_id: 9,
                action_id: 7,
                value: 2.5,
            },
            NetworkMessage::ObjectiveState {
                seq: 5,
                time: 1.0,
                rule_kind: "CapturePoints".to_string(),
                points: vec![("A".to_string(), 0, 0.0), ("B".to_string(), 2, 0.75)],
            },
        ]
    }

    #[test]
    fn roundtrip_all_variants() {
        for m in sample_messages() {
            let bytes = m.encode();
            let decoded = NetworkMessage::decode(&bytes).unwrap();
            assert_eq!(decoded, m, "roundtrip failed for {m:?}");
        }
    }

    #[test]
    fn roundtrip_unicode_name() {
        let m = NetworkMessage::Join {
            player_id: 1,
            name: "玩家".repeat(40), // 240 字节（≤ 255），应完整往返
        };
        let decoded = NetworkMessage::decode(&m.encode()).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn join_name_truncated_to_255_bytes() {
        let m = NetworkMessage::Join {
            player_id: 1,
            name: "a".repeat(300),
        };
        let bytes = m.encode();
        // 头部 + player_id(4) + 长度(1) + 名字(截断到 255)
        assert_eq!(bytes.len(), HEADER_LEN + 4 + 1 + 255);
        match NetworkMessage::decode(&bytes).unwrap() {
            NetworkMessage::Join { name, .. } => assert_eq!(name.len(), 255),
            _ => unreachable!(),
        }
    }

    #[test]
    fn position_payload_layout_is_fixed() {
        let m = NetworkMessage::Position {
            player_id: 0x0102_0304,
            seq: 0xAABB_CCDD,
            state: state(1.0, 2.0, 3.0, 4.0),
        };
        let bytes = m.encode();
        // HEADER_LEN(5) + player_id(4) + seq(4) + pos(12) + rot(4) = 29
        assert_eq!(bytes.len(), HEADER_LEN + 24);
        // 大端字节序校验
        assert_eq!(&bytes[HEADER_LEN..HEADER_LEN + 4], &[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(bytes[0], PROTOCOL_MAGIC);
        assert_eq!(bytes[1], PROTOCOL_VERSION);
        assert_eq!(bytes[2], MessageType::Position as u8);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut buf = sample_messages()[0].encode();
        buf[0] = 0x00;
        assert_eq!(NetworkMessage::decode(&buf), Err(NetError::InvalidMagic));
    }

    #[test]
    fn decode_rejects_bad_version() {
        let mut buf = sample_messages()[0].encode();
        buf[1] = 0x02;
        assert_eq!(NetworkMessage::decode(&buf), Err(NetError::UnsupportedVersion(0x02)));
    }

    #[test]
    fn decode_rejects_unknown_type() {
        let mut buf = sample_messages()[0].encode();
        buf[2] = 0xFF;
        assert_eq!(NetworkMessage::decode(&buf), Err(NetError::UnknownMessageType(0xFF)));
    }

    #[test]
    fn decode_rejects_truncated() {
        let bytes = sample_messages()[3].encode();
        assert_eq!(NetworkMessage::decode(&[]), Err(NetError::Truncated));
        assert_eq!(
            NetworkMessage::decode(&bytes[..bytes.len() - 1]),
            Err(NetError::Truncated)
        );
    }

    #[test]
    fn decode_rejects_trailing_data() {
        let mut bytes = sample_messages()[1].encode();
        bytes.push(0x00);
        assert_eq!(NetworkMessage::decode(&bytes), Err(NetError::TrailingData));
    }

    #[test]
    fn decode_rejects_invalid_name_utf8() {
        let mut bytes = sample_messages()[1].encode();
        let name_off = HEADER_LEN + 4 + 1; // 跳到名字首字节
        bytes[name_off] = 0xFF;
        assert_eq!(NetworkMessage::decode(&bytes), Err(NetError::InvalidName));
    }

    #[test]
    fn lerp_endpoints() {
        let prev = state(0.0, 0.0, 0.0, 0.0);
        let curr = state(10.0, 20.0, 30.0, 1.0);
        assert_eq!(lerp_state(prev, curr, 0.0), prev);
        assert_eq!(lerp_state(prev, curr, 1.0), curr);
    }

    #[test]
    fn lerp_midpoint() {
        let prev = state(0.0, 0.0, 0.0, 0.0);
        let curr = state(10.0, 20.0, 30.0, 1.0);
        assert_eq!(lerp_state(prev, curr, 0.5), state(5.0, 10.0, 15.0, 0.5));
    }

    #[test]
    fn lerp_clamps_out_of_range() {
        let prev = state(0.0, 0.0, 0.0, 0.0);
        let curr = state(10.0, 20.0, 30.0, 1.0);
        assert_eq!(lerp_state(prev, curr, -1.0), prev);
        assert_eq!(lerp_state(prev, curr, 2.0), curr);
    }

    #[test]
    fn lerp_rotation_takes_shortest_arc() {
        // 3.0 rad -> -3.0 rad：直接差值 -6.0 超出 [-PI, PI]，应走 +0.283 rad 短弧
        let prev = state(0.0, 0.0, 0.0, 3.0);
        let curr = state(0.0, 0.0, 0.0, -3.0);
        let mid = lerp_state(prev, curr, 0.5);
        assert!(
            (mid.rot - std::f32::consts::PI).abs() < 1e-4,
            "midpoint should be near PI, got {}",
            mid.rot
        );
        assert_eq!(lerp_state(prev, curr, 0.0).rot, 3.0);
        assert_eq!(lerp_state(prev, curr, 1.0).rot, -3.0);
    }

    #[test]
    fn interpolate_time_advance() {
        let prev = state(0.0, 0.0, 0.0, 0.0);
        let curr = state(10.0, 0.0, 0.0, 0.0);
        // 中间时刻
        assert_eq!(interpolate_at(prev, 0.0, curr, 1.0, 0.5), state(5.0, 0.0, 0.0, 0.0));
        // t 在区间外时 clamp
        assert_eq!(interpolate_at(prev, 0.0, curr, 1.0, -1.0), prev);
        assert_eq!(interpolate_at(prev, 0.0, curr, 1.0, 2.0), curr);
    }

    #[test]
    fn interpolate_stale_or_equal_timestamps() {
        let prev = state(0.0, 0.0, 0.0, 0.0);
        let curr = state(10.0, 0.0, 0.0, 0.0);
        // 时间戳相同或倒退时返回当前状态
        assert_eq!(interpolate_at(prev, 1.0, curr, 1.0, 1.0), curr);
        assert_eq!(interpolate_at(prev, 2.0, curr, 1.0, 1.5), curr);
    }

    #[test]
    fn remote_player_update_and_query() {
        let mut rp = RemotePlayer::new(1, state(0.0, 0.0, 0.0, 0.0), 0.0);
        rp.update(state(10.0, 20.0, 30.0, 1.0), 1.0);
        assert_eq!(rp.state_at(0.5), state(5.0, 10.0, 15.0, 0.5));
        // 超过当前时刻 clamp 到当前
        assert_eq!(rp.state_at(5.0), state(10.0, 20.0, 30.0, 1.0));
        // delay 滞后补偿：渲染时刻前移 delay 秒
        rp.delay = 0.2;
        assert_eq!(rp.state_at(0.7), state(5.0, 10.0, 15.0, 0.5));
    }

    // -----------------------------------------------------------------------
    // UDP loopback
    // -----------------------------------------------------------------------

    fn recv_until(
        mut recv: impl FnMut() -> io::Result<Option<(NetworkMessage, SocketAddr)>>,
        timeout: Duration,
    ) -> (NetworkMessage, SocketAddr) {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(p) = recv().unwrap() {
                return p;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for datagram");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn udp_loopback_join_position_action() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        let mut client = Client::connect(server_addr).unwrap();

        // client -> server：Join 申请
        let join_req = NetworkMessage::Join {
            player_id: 0,
            name: "alice".to_string(),
        };
        client.send(&join_req).unwrap();
        let (got, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from, client.local_addr().unwrap());
        assert_eq!(got, join_req);

        // server：注册 + 回发 Join 确认
        let reply = server.handle_join(from, "alice".to_string()).unwrap();
        assert_eq!(
            reply,
            NetworkMessage::Join {
                player_id: 1,
                name: "alice".to_string()
            }
        );
        assert_eq!(server.client_count(), 1);
        assert_eq!(server.client_id(from), Some(1));

        // client：收到确认并记录自身 id
        let (got2, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        client.handle_message(got2);
        assert_eq!(client.player_id(), Some(1));

        // client -> server：Position 状态上报
        let pos = NetworkMessage::Position {
            player_id: 1,
            seq: 1,
            state: state(1.0, 2.0, 3.0, 0.5),
        };
        client.send(&pos).unwrap();
        let (got3, from3) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from3, client.local_addr().unwrap());
        assert_eq!(got3, pos);

        // server -> client：广播 Action
        let action = NetworkMessage::Action {
            player_id: 1,
            action_id: 7,
            value: 1.5,
        };
        server.broadcast(&action, None).unwrap();
        let (got4, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        assert_eq!(got4, action);

        // client：处理 Position 进入插值缓冲
        client.handle_message(pos);
        assert!(client.remote_players().contains_key(&1));
        let t = client.now();
        assert_eq!(client.remote_state_at(1, t), Some(state(1.0, 2.0, 3.0, 0.5)));
    }

    #[test]
    fn udp_loopback_leave_roundtrip() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = Client::connect(server_addr).unwrap();

        let leave = NetworkMessage::Leave {
            player_id: 5,
            reason: 0,
        };
        client.send(&leave).unwrap();
        let (got, _) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(got, leave);
    }

    // -----------------------------------------------------------------------
    // Input / Snapshot 序列化（确定性布局）
    // -----------------------------------------------------------------------

    fn net_input() -> NetInput {
        NetInput {
            forward: true,
            backward: false,
            left: true,
            right: false,
            fire: true,
            yaw: 2.0,
            pitch: -0.5,
        }
    }

    #[test]
    fn input_payload_layout_is_fixed() {
        let m = NetworkMessage::Input {
            seq: 0x1122_3344,
            time: 1.5,
            input: net_input(),
        };
        let bytes = m.encode();
        // HEADER(5) + seq(4) + time(4) + 5×bool + yaw(4) + pitch(4) = 26
        assert_eq!(bytes.len(), HEADER_LEN + 21);
        assert_eq!(bytes[0], PROTOCOL_MAGIC);
        assert_eq!(bytes[2], MessageType::Input as u8);
        // seq 大端
        assert_eq!(&bytes[HEADER_LEN..HEADER_LEN + 4], &[0x11, 0x22, 0x33, 0x44]);
        // 5 个 bool 标志字节（seq+time 之后）
        assert_eq!(&bytes[HEADER_LEN + 8..HEADER_LEN + 13], &[1, 0, 1, 0, 1]);
        let back = NetworkMessage::decode(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn snapshot_payload_layout_is_fixed() {
        let m = NetworkMessage::Snapshot {
            seq: 7,
            time: 3.25,
            player_id: 1,
            player: state(1.0, 2.0, 3.0, 0.5),
            npcs: vec![
                NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0 },
                NpcSnapshot { id: 11, pos: [-2.0, 0.0, 4.0], facing: -1.0, hp: 50.0 },
            ],
        };
        let bytes = m.encode();
        // HEADER + seq(4) + time(4) + player_id(4) + player(16) + count(2) + 2×24
        assert_eq!(bytes.len(), HEADER_LEN + 30 + 48);
        assert_eq!(bytes[2], MessageType::Snapshot as u8);
        assert_eq!(&bytes[HEADER_LEN + 28..HEADER_LEN + 30], &[0x00, 0x02]);
        assert_eq!(&bytes[HEADER_LEN + 30..HEADER_LEN + 34], &[0, 0, 0, 10]);
        let back = NetworkMessage::decode(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn snapshot_npcs_capped_at_max() {
        let npcs = (0..(MAX_SNAPSHOT_NPCS + 50) as u32)
            .map(|id| NpcSnapshot { id, pos: [0.0; 3], facing: 0.0, hp: 1.0 })
            .collect::<Vec<_>>();
        let m = NetworkMessage::Snapshot {
            seq: 1,
            time: 0.0,
            player_id: 0,
            player: state(0.0, 0.0, 0.0, 0.0),
            npcs,
        };
        let bytes = m.encode();
        let decoded = NetworkMessage::decode(&bytes).unwrap();
        match decoded {
            NetworkMessage::Snapshot { npcs, .. } => assert_eq!(npcs.len(), MAX_SNAPSHOT_NPCS),
            _ => unreachable!(),
        }
    }

    // -----------------------------------------------------------------------
    // ObjectiveState 序列化（确定性布局 + 防御性解码）
    // -----------------------------------------------------------------------

    /// ObjectiveState 测试用固定 rule_kind（13 字节 ASCII，便于按偏移断言布局）
    const RULE_KIND: &str = "CapturePoints";

    fn obj_msg() -> NetworkMessage {
        NetworkMessage::ObjectiveState {
            seq: 9,
            time: 12.5,
            rule_kind: RULE_KIND.to_string(),
            points: vec![
                ("A".to_string(), 0, 0.0),
                ("B".to_string(), 1, 0.65),
                ("C".to_string(), 2, 1.0),
            ],
        }
    }

    #[test]
    fn objective_state_payload_layout_is_fixed() {
        let m = obj_msg();
        let bytes = m.encode();
        assert_eq!(bytes[0], PROTOCOL_MAGIC);
        assert_eq!(bytes[1], PROTOCOL_VERSION);
        assert_eq!(bytes[2], MessageType::ObjectiveState as u8);
        // HEADER + seq(4) + time(4) + rule_len(1) + rule(13) + count(2) + 3×7
        assert_eq!(bytes.len(), HEADER_LEN + 4 + 4 + 1 + RULE_KIND.len() + 2 + 3 * 7);
        // rule 长度前缀
        assert_eq!(bytes[HEADER_LEN + 8], RULE_KIND.len() as u8);
        // count 大端
        let count_off = HEADER_LEN + 4 + 4 + 1 + RULE_KIND.len();
        assert_eq!(&bytes[count_off..count_off + 2], &[0x00, 0x03]);
        // 第一个据点：id 长度前缀 + owner 码
        let p0_off = count_off + 2;
        assert_eq!(bytes[p0_off], 1); // "A" 长度 1
        assert_eq!(bytes[p0_off + 1], b'A');
        assert_eq!(bytes[p0_off + 2], 0); // owner=0 中立
        let back = NetworkMessage::decode(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn objective_state_roundtrip_multi_point_chinese_ids() {
        let m = NetworkMessage::ObjectiveState {
            seq: u32::MAX - 3,
            time: -0.25,
            rule_kind: "KillCount".to_string(),
            points: vec![
                ("据点-中央广场".to_string(), 0, 0.0),
                ("据点-火车站".to_string(), 1, 0.5),
                ("据点-弹药库".to_string(), 2, 1.0),
            ],
        };
        let back = NetworkMessage::decode(&m.encode()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn objective_state_points_capped_at_max() {
        let points = (0..(MAX_OBJECTIVE_POINTS + 10) as u32)
            .map(|i| (format!("point-{i}"), 1u8, 0.5f32))
            .collect::<Vec<_>>();
        let m = NetworkMessage::ObjectiveState {
            seq: 1,
            time: 0.0,
            rule_kind: "CapturePoints".to_string(),
            points,
        };
        let bytes = m.encode();
        let decoded = NetworkMessage::decode(&bytes).unwrap();
        match decoded {
            NetworkMessage::ObjectiveState { points, .. } => {
                assert_eq!(points.len(), MAX_OBJECTIVE_POINTS);
                assert_eq!(points[0].0, "point-0");
                assert_eq!(points[0].1, 1);
                assert_eq!(points[0].2, 0.5);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn objective_state_decode_rejects_truncated_prefixes() {
        let bytes = obj_msg().encode();
        // 编码总长恰为 HEADER_LEN + payload_len，任何真实前缀都必被头部长度校验
        // 或 Reader 边界检查以 Truncated 拒绝，绝不 panic；仅完整报文成功
        for cut in 0..bytes.len() {
            assert_eq!(
                NetworkMessage::decode(&bytes[..cut]),
                Err(NetError::Truncated),
                "prefix cut={cut} should be Truncated"
            );
        }
        assert_eq!(NetworkMessage::decode(&bytes), Ok(obj_msg()));
    }

    #[test]
    fn objective_state_decode_rejects_oversized_point_count() {
        // 伪造 count=65535：实际载荷不足，逐据点读取必撞边界 → Truncated，不 panic
        let mut bytes = obj_msg().encode();
        let count_off = HEADER_LEN + 4 + 4 + 1 + RULE_KIND.len();
        bytes[count_off] = 0xFF;
        bytes[count_off + 1] = 0xFF;
        assert_eq!(NetworkMessage::decode(&bytes), Err(NetError::Truncated));
    }

    #[test]
    fn objective_state_decode_rejects_invalid_utf8() {
        // rule_kind 首字节改成非法 UTF-8（0xFF）
        let mut bytes = obj_msg().encode();
        let rule_off = HEADER_LEN + 4 + 4 + 1;
        bytes[rule_off] = 0xFF;
        assert_eq!(NetworkMessage::decode(&bytes), Err(NetError::InvalidUtf8));
    }

    // -----------------------------------------------------------------------
    // UDP loopback：握手 → Input 往返 → Snapshot 往返 → 客户端应用快照
    // -----------------------------------------------------------------------

    #[test]
    fn udp_loopback_handshake_input_snapshot_roundtrip() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        let mut client = Client::connect(server_addr).unwrap();

        // 握手：client Join 申请 → server 分配 id 回 ack → client 记录自身 id
        client
            .send(&NetworkMessage::Join { player_id: 0, name: "player1".into() })
            .unwrap();
        let (req, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from, client.local_addr().unwrap());
        assert_eq!(req, NetworkMessage::Join { player_id: 0, name: "player1".into() });
        server.handle_join(from, "player1".into()).unwrap();
        let (got, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        client.handle_message(got);
        assert_eq!(client.player_id(), Some(1));
        assert!(client.is_connected());

        // 客户端上报输入 → 服务端收到（确定性往返）
        let input = net_input();
        client
            .send(&NetworkMessage::Input { seq: 1, time: 0.5, input })
            .unwrap();
        let (msg, from2) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from2, client.local_addr().unwrap());
        match msg {
            NetworkMessage::Input { seq, time, input: got } => {
                assert_eq!(seq, 1);
                assert_eq!(time, 0.5);
                assert_eq!(got, input);
            }
            other => panic!("expected Input, got {other:?}"),
        }

        // 服务端广播快照 → 客户端应用：快照元信息 + 实体插值 + 本机权威状态
        let snapshot = NetworkMessage::Snapshot {
            seq: 1,
            time: 1.0,
            player_id: 1,
            player: state(5.0, 0.0, 5.0, 0.5),
            npcs: vec![
                NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0 },
                NpcSnapshot { id: 11, pos: [2.0, 0.0, 2.0], facing: -0.5, hp: 40.0 },
            ],
        };
        server.send_to(&snapshot, from2).unwrap();
        let (got2, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        client.handle_message(got2);
        assert_eq!(client.snapshot_seq(), 1);
        assert_eq!(client.snapshot_time(), 1.0);
        assert_eq!(client.own_state(), Some(state(5.0, 0.0, 5.0, 0.5)));
        assert_eq!(client.entities().len(), 3, "本机玩家 + 2 个 NPC");
        assert_eq!(
            client.entity_state_at(10, client.now()),
            Some(state(1.0, 0.0, 1.0, 0.25))
        );
        assert_eq!(client.entities().get(&11).map(|e| e.hp), Some(40.0));
        assert!(!client.snapshot_timeout());

        // 重复快照（同 seq）被丢弃：序号与实体表不变
        client.handle_message(snapshot);
        assert_eq!(client.snapshot_seq(), 1);
        assert_eq!(client.entities().len(), 3);
    }

    #[test]
    fn client_snapshot_timeout_detects_disconnect() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        let mut client = Client::connect(server_addr).unwrap();
        // 缩短超时阈值，避免真实时钟等待，保持测试快速且确定
        client.timeout = Duration::from_millis(10);
        client.handle_message(NetworkMessage::Join { player_id: 1, name: "p".into() });
        assert!(client.player_id().is_some());
        std::thread::sleep(Duration::from_millis(30));
        assert!(client.snapshot_timeout(), "超过阈值无数据报应判定断线");
        assert!(!client.is_connected());
        // 任何数据报刷新 last_rx → 恢复连接判定
        client.handle_message(NetworkMessage::Join { player_id: 1, name: "p".into() });
        assert!(!client.snapshot_timeout());
        assert!(client.is_connected());
    }
}
