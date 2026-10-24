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
//! 限制与后续 TODO：单数据报不做分片/重组（快照 NPC 上限 MAX_SNAPSHOT_NPCS，超出截断）。
//! **已接线**（2026-09-26 复查逐条对过代码，别再照旧条目找）：远端实体插值渲染
//! （`main.rs` 按 `entity_state_at` 取平滑位置）、自身位置上行校正（`game.rs`，水平差 >3m
//! 硬对齐）、断线自动重连（`reset_connection` + `retry_join`；重连后按**新 id** 加入
//! ⇒ **会话恢复**未做）、中继注册/解析 + 打洞探测包（`rdv_register` / `rdv_resolve`，第一步）。
//! **未做**：NAT 双进程真机验证、快照增量压缩、输入预测/回滚、真实两机实战场。
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
/// 协议版本（wire 布局变更时必须递增：v2 = 快照行含 team/firing）
/// 服务器拒绝不匹配版本的 Join（客户端收到 Refuse 停止重试）
pub const SESSION_VERSION: u16 = 3;

/// 挑战/会话token长度（字节）。S2：Join 挑战应答 + 每次会话一个 16 B token。
pub const TOKEN_LEN: usize = 16;

/// 挑战/会话token字节数组（S2）。
pub type Token = [u8; TOKEN_LEN];

/// S7：把网络来源的字符串压成"一行、有限长"后再进日志。
///
/// 验收判据读的就是日志文本（panics/VUID/hits 都在里头），而 `Refuse.reason` 由对端任意填写 ——
/// 一条带换行的 reason 就能伪造整行日志。控制字符（含 \n/\r/\t 与 U+2028/U+2029）一律换成空格，
/// 并截到 128 个字符。
pub fn sanitize_log_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(256));
    for (i, c) in s.chars().enumerate() {
        if i >= 128 {
            out.push_str("...");
            break;
        }
        let unsafe_char = c.is_control() || c == '\u{2028}' || c == '\u{2029}';
        out.push(if unsafe_char { ' ' } else { c });
    }
    out
}

/// 生成一枚挑战 nonce（std 哈希；只要求同网段不可预测，不追求密码学强度）。
fn fresh_nonce(from: SocketAddr) -> Token {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut out = [0u8; TOKEN_LEN];
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = DefaultHasher::new();
        from.hash(&mut h);
        nanos.hash(&mut h);
        i.hash(&mut h);
        chunk.copy_from_slice(&h.finish().to_be_bytes());
    }
    out
}

/// 由 nonce + 地址 + id 派生会话token。
fn derive_token(nonce: &Token, from: SocketAddr, id: u32) -> Token {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut out = [0u8; TOKEN_LEN];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut h = DefaultHasher::new();
        nonce.hash(&mut h);
        from.hash(&mut h);
        id.hash(&mut h);
        i.hash(&mut h);
        chunk.copy_from_slice(&h.finish().to_be_bytes());
    }
    out
}
/// 单条 UDP 数据报读取缓冲上限：取 UDP 载荷上限 65507（IPv4）。
/// 快照需承载 64v64 压力模式全部 NPC（128 × 24B ≈ 3KB），1400 的 MTU 余量会截断快照；
/// 局域网碎片化可接受，本轮不做分片/重组（见模块头部协议注释）。
pub const MAX_DATAGRAM: usize = 65507;
/// 单条快照最多携带的实体数：超出截断（保护 length 字段 u16 上限；本轮 128 NPC 远低于此）
pub const MAX_SNAPSHOT_NPCS: usize = 1024;

/// 复核 S4（2026-10-24）：客户端实体插值表上限。
pub const MAX_ENTITIES: usize = 4096;

/// 复核 S4：远端玩家插值表上限（与服务器 MAX_CLIENTS 对齐）。
pub const MAX_REMOTE_PLAYERS: usize = 64;

/// 解码快照时的**容量提示**：按协议上限封顶。
///
/// 🔴 2026-09-23 复查：`Snapshot` 分支原来是裸的 `Vec::with_capacity(n)`，而 `n` 是
/// **报文里的 2 字节 u16**（最大 65535）⇒ 伪造报文可以只花 2 字节就让接收方一次预留
/// ~1.6 MB（`NpcSnapshot` 约 28B × 65535），放大比约 800,000:1。
/// 旁边 `ObjectiveState` 分支**早就**这么防了（`n.min(MAX_OBJECTIVE_POINTS)`，注释写着
/// "防止攻击者仅凭 2 字节 count 触发大分配"）—— 这条只是把同一个判据补到快照上。
/// ⚠️ 只压容量提示，**不改"接受多少条"的语义**（逐条读取本来就有边界检查）。
fn snapshot_capacity_hint(n: usize) -> usize {
    n.min(MAX_SNAPSHOT_NPCS)
}
/// 单条目标状态最多携带的据点数：超出截断（单关据点通常 1-5 个，64 为防御性上限；
/// 每个据点约 6-261B，64 个最坏约 17KB，远低于 MAX_DATAGRAM）
pub const MAX_OBJECTIVE_POINTS: usize = 64;
/// 客户端断线判定：超过该时长未收到任何数据报视为超时（UDP 尽力而为，无重传）
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(3);
/// 服务端超时判定：客户端超过该时长无数据报则移除注册（断线重连为后续 TODO）
pub const SERVER_TIMEOUT: Duration = Duration::from_secs(5);

/// 快照里**远端玩家**的 id 区：服务端发 `NET_PLAYER_BASE + player_id`，客户端据此把这些
/// 实体与本地 NPC（小 id）区分开。定义在协议层是因为**服务端构包时就要用**，
/// 而客户端解析 `Leave` 也要用它把 `player_id` 换算成实体 id。
pub const NET_PLAYER_BASE: u32 = 100_000;

/// 远端实体"多久没出现在快照里就当它已经离场"（秒）。
///
/// 🔴 2026-09-26 修：客户端以前**从不清理**实体表 —— 服务端超时把客户端从注册表里摘掉后
/// （`timeout_clients`），客户端这边的实体仍以最后一帧的姿态留着，而 `main.rs` 对
/// id ≥ `NET_PLAYER_BASE` 的实体是**无条件进画面**的 ⇒ 断线的人会**永久站在场上**。
/// 取 2.0s：小于 `CLIENT_TIMEOUT`(3s)，即"判定断线之前先让幽灵消失"，同时容忍偶发丢包
/// （快照每帧都发，连续 2 秒收不到说明真的没了）。
pub const ENTITY_STALE_AFTER: f64 = 2.0;

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
    /// 服务端 → 客户端：拒绝加入（协议版本不匹配等；reason 文本）
    Refuse = 0x08,
    /// 服务端 → 客户端：握手挑战（S2），客户端必须回 JoinProof 回显 nonce
    Challenge = 0x09,
    /// 客户端 → 服务端：回显挑战 nonce（S2）
    JoinProof = 0x0A,
    /// 服务端 → 客户端：会话建立（分配 id + 16 B 会话token）
    Welcome = 0x0B,
    /// 任意方向：带token的封装包（body = 完整内层帧，含魔数/版本/类型）
    Sealed = 0x0C,
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
            0x08 => MessageType::Refuse,
            0x09 => MessageType::Challenge,
            0x0A => MessageType::JoinProof,
            0x0B => MessageType::Welcome,
            0x0C => MessageType::Sealed,
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
    /// 阵营（0=Red 1=Blue；客户端渲染直接权威使用，2026-08-25）
    pub team: u8,
    /// 开火指示（1=本帧/最近 0.2s 内开火；客户端枪口焰联动）
    pub firing: u8,
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
    /// 玩家加入；`player_id == 0` 表示申请加入，服务端回复分配后的 id；
    /// version = 协议版本（不匹配 → 服务端回 Refuse）
    Join { player_id: u32, name: String, version: u16 },
    /// 服务端拒绝（协议不匹配等；reason 供客户端显示）
    Refuse { reason: String },
    /// 服务端挑战（S2）：客户端必须原样回显 `nonce` 才允许注册。
    Challenge { nonce: [u8; TOKEN_LEN] },
    /// 客户端回显挑战（S2）。
    JoinProof { nonce: [u8; TOKEN_LEN] },
    /// 服务端确认会话（S2）：分配 player_id + 16 B 会话token。
    Welcome { player_id: u32, token: [u8; TOKEN_LEN] },
    /// 带token封装（S2）：`body` 是完整内层帧；收包方先校验token再解内层 ——
    /// 伪造源地址的 Snapshot/Input/Leave 全部失效。
    Sealed { token: [u8; TOKEN_LEN], body: Vec<u8> },
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
    /// 浮点字段是 NaN / ±inf（线路边界一律拒绝，见 `Reader::f32` 的理由）
    NonFiniteField,
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
            NetError::NonFiniteField => write!(f, "non-finite float field (NaN or inf)"),
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

/// 解析中继对 `REG` 的回复（**纯函数**，便于单测）。
///
/// 协议：`OK [<SocketAddr>]` / `BAD port` / `BUSY name` / `FULL`。
///
/// 🔴 **2026-10-17 外部复核指出**：原来客户端对 `REG` 是 **fire-and-forget**
/// （`send_to` 之后直接 `Ok(())`）⇒ 中继回的 `BAD port` / `BUSY name` / `FULL`
/// **一个都收不到**，注册失败在客户端完全静默（而我们上一轮刚加了这三种错误回复）。
pub fn parse_rdv_reply(line: &str) -> Result<Option<SocketAddr>, String> {
    let mut it = line.split_whitespace();
    match it.next() {
        Some("OK") => Ok(it.next().and_then(|s| s.parse::<SocketAddr>().ok())),
        Some("BAD") => Err(format!("中继拒绝了端口（{line}）")),
        Some("BUSY") => Err(format!("房间名已被其它 IP 占用（{line}）")),
        Some("FULL") => Err(format!("中继注册表已满（{line}）")),
        other => Err(format!("中继回复无法识别: {other:?}（{line}）")),
    }
}

/// 中继注册：通知接入点本机地址（接入点记录其观测到的公网地址）
/// 返回注册 OK 与否（尽力而为；不做失败重试——客户端可稍后 WHO 检索）
pub fn rdv_register(rdv: &str, name: &str) -> io::Result<Option<SocketAddr>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    let raddr: SocketAddr = rdv
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("中继地址解析失败: {e}")))?;
    sock.send_to(format!("REG {name}").as_bytes(), raddr)?;
    // ⚠️ **这个函数用的是自己新建的临时 socket** ⇒ 读回复是安全的（不会吞游戏包）。
    // 2s 上限；中继的 `OK <addr>` 带**它观测到的我们的公网地址**（STUN 式反馈），
    // 旧版中继只回 `OK` ⇒ 解析成 `Ok(None)`，仍然兼容。
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buf = [0u8; 128];
    match sock.recv_from(&mut buf) {
        Ok((n, from)) if from == raddr => {
            let line = String::from_utf8_lossy(&buf[..n]);
            parse_rdv_reply(line.trim()).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
        }
        Ok(_) => Ok(None), // 不是中继发来的 ⇒ 忽略
        Err(e) => Err(e),
    }
}

/// 中继解析：向接入点查询主机地址（公网观测地址；NAT 打洞第一步）
/// 超时 5s；回复 ADDR <name> <addr> 或 NONE
pub fn rdv_resolve(rdv: &str, name: &str) -> io::Result<SocketAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_secs(5)))?;
    let raddr: SocketAddr = rdv
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("中继地址解析失败: {e}")))?;
    sock.send_to(format!("WHO {name}").as_bytes(), raddr)?;
    let mut buf = [0u8; 512];
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            // 🔴 **只收中继地址的回包**（2026-10-19 外部复核缺陷 3-A，已确认）：兄弟函数
            // `rdv_register` 早就有 `if from == raddr`，这里漏了 ⇒ 任何能往本机端口发 UDP 的
            // 进程都能伪造 `ADDR <name> <ip:port>`，把客户端引到任意地址（还会帮攻击者
            // 反射一次 `PING`）。
            Ok((n, from)) if from == raddr => {
                let line = String::from_utf8_lossy(&buf[..n]);
                let mut it = line.split_whitespace();
                if it.next() == Some("ADDR") {
                    if let (Some(nm), Some(addr)) = (it.next(), it.next()) {
                        if nm == name {
                            if let Ok(a) = addr.parse() {
                                // 打洞：向目标地址发送一次探测（打开 NAT 映射）
                                let _ = sock.send_to(b"PING", a);
                                return Ok(a);
                            }
                        }
                    }
                }
            }
            Ok(_) => {
                // 非中继来源的杂包：忽略，继续等
            }
            Err(_) => {
                // 超时继续/wouldblock
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, format!("中继查询 {name} 超时")))
}

/// 非阻塞收包（Server/Client 共用）：无数据 Ok(None)，协议错误映射 InvalidData
/// 单次 `recv_msg` 里**最多跳过**多少条解不开的数据报。
///
/// 为什么要有上界：非本协议报文（中继打洞的 `b"PING"`、局域网杂包、端口扫描）是**常态**，
/// 跳过它们才是正确行为；但**如果对面一直灌**，无界跳过会让这一帧的收包循环空转
/// ⇒ 超过上界就当作"本帧没数据了"，把控制权还给调用方。
const MAX_JUNK_SKIP: usize = 32;

/// 收一条**本协议**报文。
///
/// 🔴 2026-10-15 修（外部复核指出 B4，已确认）：原来解不开就返回
/// `Err(InvalidData)`，而所有调用方都写成 `while let Ok(Some(..)) = recv()` ——
/// **一条杂包就把整帧的收包循环打断**，内核缓冲区里排在它后面的合法玩家报文
/// 全部要等到下一帧才处理。打洞 `PING` 是每次连接都会发的，这不是边角情况。
///
/// 现在：解不开的**跳过并继续收**（最多 [`MAX_JUNK_SKIP`] 条），
/// 只有真正的 I/O 错误才向上返回 `Err`。
fn recv_msg(socket: &std::net::UdpSocket) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
    let mut buf = [0u8; MAX_DATAGRAM];
    for _ in 0..=MAX_JUNK_SKIP {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => match NetworkMessage::decode(&buf[..n]) {
                Ok(msg) => return Ok(Some((msg, from))),
                Err(e) => {
                    // 非本协议 / 畸形报文：**跳过这一条继续收**。
                    // 用 debug 级：局域网里这类包可能有稳定的背景量，warn 会刷屏。
                    log::debug!("net: 跳过一条解不开的 {n}B 数据报（{e}）");
                    continue;
                }
            },
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    // 跳够上限：对面在灌垃圾，本帧到此为止（别把帧时间耗在丢弃上）
    log::debug!("net: 连续跳过 {MAX_JUNK_SKIP} 条杂包，本帧收包到此为止");
    Ok(None)
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

    /// 读一个 f32，并**拒绝非有限值**（NaN / ±inf）。
    ///
    /// 🔴 理由（2026-09-26 复查）：`from_bits` 会把任意 4 字节原样变成浮点数，NaN 也算合法读出，
    /// 于是一张伪造/损坏的报文能把 NaN 一路送进插值与实例矩阵 ——
    /// `lerp_state` 的 `alpha.clamp(0.0, 1.0)` 对 NaN 返回 NaN（`a <= 0.0` 与 `a >= 1.0` 都不成立），
    /// 远端实体位置变 NaN ⇒ 矩阵 NaN ⇒ **静默不画**（不崩、不报 VUID、日志干净）；
    /// 时间戳 NaN 更彻底：`interpolate_at` 的 `dt <= 0.0` 对 NaN 为假，插值永远走 NaN 分支。
    /// 合法对端不会发非有限值 ⇒ 这里一律按协议错误拒绝，**线路边界是唯一的收口点**。
    /// 判据 = `non_finite_floats_are_rejected_at_the_wire`。
    fn f32(&mut self) -> Result<f32, NetError> {
        let v = f32::from_bits(self.u32()?);
        if !v.is_finite() {
            return Err(NetError::NonFiniteField);
        }
        Ok(v)
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
            NetworkMessage::Refuse { reason } => {
                let rb = reason.as_bytes();
                let n = rb.len().min(255);
                let mut p = Vec::with_capacity(5 + n);
                p.push(n as u8);
                p.extend_from_slice(&rb[..n]);
                (MessageType::Refuse, p)
            }
            NetworkMessage::Challenge { nonce } => (MessageType::Challenge, nonce.to_vec()),
            NetworkMessage::JoinProof { nonce } => (MessageType::JoinProof, nonce.to_vec()),
            NetworkMessage::Welcome { player_id, token } => {
                let mut p = Vec::with_capacity(4 + TOKEN_LEN);
                put_u32(&mut p, *player_id);
                p.extend_from_slice(token);
                (MessageType::Welcome, p)
            }
            NetworkMessage::Sealed { token, body } => {
                let n = body.len().min(u16::MAX as usize);
                let mut p = Vec::with_capacity(TOKEN_LEN + 2 + n);
                p.extend_from_slice(token);
                p.extend_from_slice(&(n as u16).to_be_bytes());
                p.extend_from_slice(&body[..n]);
                (MessageType::Sealed, p)
            }
            NetworkMessage::Join { player_id, name, version } => {
                let name = name.as_bytes();
                let n = name.len().min(255);
                let mut p = Vec::with_capacity(5 + n + 2);
                put_u32(&mut p, *player_id);
                p.push(n as u8);
                p.extend_from_slice(&name[..n]);
                p.extend_from_slice(&version.to_be_bytes());
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
                    p.push(npc.team);
                    p.push(npc.firing);
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
                let version = u16::from_be_bytes([r.u8()?, r.u8()?]);
                NetworkMessage::Join { player_id, name, version }
            }
            MessageType::Challenge => {
                let mut nonce = [0u8; TOKEN_LEN];
                nonce.copy_from_slice(r.bytes(TOKEN_LEN)?);
                NetworkMessage::Challenge { nonce }
            }
            MessageType::JoinProof => {
                let mut nonce = [0u8; TOKEN_LEN];
                nonce.copy_from_slice(r.bytes(TOKEN_LEN)?);
                NetworkMessage::JoinProof { nonce }
            }
            MessageType::Welcome => {
                let player_id = r.u32()?;
                let mut token = [0u8; TOKEN_LEN];
                token.copy_from_slice(r.bytes(TOKEN_LEN)?);
                NetworkMessage::Welcome { player_id, token }
            }
            MessageType::Sealed => {
                let mut token = [0u8; TOKEN_LEN];
                token.copy_from_slice(r.bytes(TOKEN_LEN)?);
                let n = u16::from_be_bytes([r.u8()?, r.u8()?]) as usize;
                let body = r.bytes(n)?.to_vec();
                NetworkMessage::Sealed { token, body }
            }
            MessageType::Refuse => {
                let len = r.u8()? as usize;
                let reason = String::from_utf8(r.bytes(len)?.to_vec())
                    .map_err(|_| NetError::InvalidUtf8)?;
                NetworkMessage::Refuse { reason }
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
                // 容量提示封顶（见 `snapshot_capacity_hint`）：不让伪造的 2 字节 count 触发大分配
                let mut npcs = Vec::with_capacity(snapshot_capacity_hint(n));
                for _ in 0..n {
                    let id = r.u32()?;
                    let pos = [r.f32()?, r.f32()?, r.f32()?];
                    let facing = r.f32()?;
                    let hp = r.f32()?;
                    let team = r.u8()?;
                    let firing = r.u8()?;
                    npcs.push(NpcSnapshot { id, pos, facing, hp, team, firing });
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
    /// 最近一次收到的 `Position` 序号（**只由 `update_seq` 维护**；`None` = 还没收到过）。
    ///
    /// 🔴 2026-10-15 补（外部复核指出 B4，已确认）：`Position` 分支此前把 `seq` 用 `..`
    /// 直接忽略 ⇒ **乱序/重复包会把位置直接回滚**（UDP 不保证顺序，这是常态而非边角）。
    /// `Snapshot` 那条路一直有 `seq_is_newer` 判据，只有 `Position` 漏了。
    last_seq: Option<u32>,
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
            last_seq: None,
        }
    }

    /// 收到新状态：当前状态降级为上一状态
    pub fn update(&mut self, state: PlayerState, time: f64) {
        self.prev = self.curr;
        self.prev_time = self.curr_time;
        self.curr = state;
        self.curr_time = time;
    }

    /// 收到带**序号**的状态（`Position` 那条路专用）：乱序/重复包**直接丢弃**。
    ///
    /// 返回 `false` = 该包过期、状态**没有被改动**（调用方不要再去动 `delay` 之类）。
    ///
    /// 为什么和 `update` 分开而不是改 `update` 的签名：快照那条路（`Snapshot` 的 NPC 循环）
    /// 用的是**整帧的** `seq`，逐实体再判一次会把同帧的实体全判成"重复"。
    /// 两条路的序号语义不同 ⇒ 分成两个入口，各用各的判据，别硬凑成一个。
    ///
    /// 序号判据复用 `seq_is_newer`（它自己就是"快照与目标状态两条丢弃路径"的单一真源，
    /// 见其文档）—— **不要在这里再写一遍比较**，那正是实例 buffer 那类静默 bug 的来源。
    pub fn update_seq(&mut self, seq: u32, state: PlayerState, time: f64) -> bool {
        if let Some(last) = self.last_seq {
            if !seq_is_newer(seq, last) {
                return false;
            }
        }
        self.last_seq = Some(seq);
        self.prev = self.curr;
        self.prev_time = self.curr_time;
        self.curr = state;
        self.curr_time = time;
        true
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
    /// 阵营（0=Red 1=Blue；来自 NpcSnapshot，渲染权威使用）
    pub team: u8,
    /// 开火指示（最近 0.2s 内开火；渲染枪口焰联动）
    pub firing: bool,
    /// 最近一次**出现在快照里**的接收时刻（`Client::now()` 时间基准）。
    /// 用于清理离场实体（见 `ENTITY_STALE_AFTER` 与 `Client::prune_stale_entities`）。
    pub last_seen: f64,
}

// ---------------------------------------------------------------------------
// UDP Server / Client
// ---------------------------------------------------------------------------

/// 序号算术的半程（2^31）：`wrapping_sub` 差值达到它即视为「过期」而非「前进」。
///
/// 🔴 这里以前两处各写一遍 `diff >= u32::MAX / 2`（= 2^31-1，比文档说的 2^31 **小 1**），
/// 而模块文档与两处注释都写「差值 ≥ 2^31 视为过期」⇒ 最远的**合法**前进（差值 2^31-1）
/// 被当成了过期。判据 = `sequence_staleness_uses_the_documented_half_range`。
pub const SEQ_HALF_RANGE: u32 = 0x8000_0000;

/// `seq` 是否比 `last` 新（序号算术，RFC 1982 风格）。
///
/// **单一真源**：快照与目标状态两条丢弃路径都走这里，别再各写一遍判据
/// （两处副本正是实例 buffer 那类静默 bug 的常见来源）。
/// 相等 = 重复、差值 ≥ [`SEQ_HALF_RANGE`] = 乱序/过期 ⇒ 都返回 `false`。
pub fn seq_is_newer(seq: u32, last: u32) -> bool {
    let diff = seq.wrapping_sub(last);
    diff != 0 && diff < SEQ_HALF_RANGE
}

/// UDP 服务器：绑定监听地址，跟踪已注册客户端并为 Join 分配玩家 id
pub struct Server {
    socket: UdpSocket,
    next_player_id: u32,
    clients: HashMap<SocketAddr, u32>,
    /// 各客户端最近一次收到数据报的时刻（超时判定用，recv 内更新）
    last_seen: HashMap<SocketAddr, Instant>,
    /// S2：已发出、等待回显的挑战 nonce
    pending_nonces: HashMap<SocketAddr, Token>,
    /// S2：挑战时登记的名字（JoinProof 时回填）
    pending_names: HashMap<SocketAddr, String>,
    /// S2：会话token（注册成功后才有；封印包必须与它匹配）
    tokens: HashMap<SocketAddr, Token>,
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
            pending_nonces: HashMap::new(),
            pending_names: HashMap::new(),
            tokens: HashMap::new(),
        })
    }

    /// 本地监听地址（获取随机端口用）
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// 查询某地址的 player id（Join 注册后有效；未注册返回 None）
    pub fn player_id_of(&self, addr: SocketAddr) -> Option<u32> {
        self.clients.get(&addr).copied()
    }

    /// 向中继注册房间（用真实监听套接字发送：源 IP=中继观测、端口=配置端口显式声明）
    pub fn rdv_register(&self, rdv: &str, name: &str, port: u16) -> io::Result<()> {
        let raddr: SocketAddr = rdv
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("中继地址解析失败: {e}")))?;
        self.socket.send_to(format!("REG {name} {port}").as_bytes(), raddr)?;
        Ok(())
    }

    /// 注册客户端地址并分配玩家 id；已注册则返回原 id
    pub fn register(&mut self, addr: SocketAddr) -> Option<u32> {
        // 🔴 **玩家 id 与客户端数都要有上限**（2026-10-19 外部复核缺陷 3-D，已确认）：
        // `next_player_id` 原来无界累加，而客户端实体键是 `NET_PLAYER_BASE + player_id`
        // ⇒ 加到 `u32::MAX - NET_PLAYER_BASE` 之后会**回绕落进 NPC id 区间**（0..100_000）
        // ⇒ 玩家与 NPC 串号。`clients` 字典也没有容量上限（伪造大量源端口的 Join 可撑爆内存）。
        // 上限取 `NET_PLAYER_BASE`：保证 `NET_PLAYER_BASE + id <= 2*NET_PLAYER_BASE - 1`，
        // 既不回绕、也不会与 NPC 段（< NET_PLAYER_BASE）重叠。
        const MAX_CLIENTS: usize = 64;
        if let Some(id) = self.clients.get(&addr) {
            return Some(*id);
        }
        if self.clients.len() >= MAX_CLIENTS {
            log::warn!("net: 拒绝注册新客户端（已达上限 {MAX_CLIENTS}）");
            return None;
        }
        if self.next_player_id >= NET_PLAYER_BASE {
            log::warn!(
                "net: 拒绝注册新客户端（player_id 已达上限 {}，避免实体 id 回绕）",
                NET_PLAYER_BASE
            );
            return None;
        }
        let id = self.next_player_id;
        self.next_player_id += 1;
        self.clients.insert(addr, id);
        Some(id)
    }

    /// 查询客户端地址对应的玩家 id
    pub fn client_id(&self, addr: SocketAddr) -> Option<u32> {
        self.clients.get(&addr).copied()
    }

    /// 注销一个客户端（收到 `Leave` 时用），返回它原来的玩家 id。
    ///
    /// 与 `timeout_clients` 的区别：那条按"多久没说话"批量清理，这条是**收到明确离场通知**
    /// 时立刻清（客户端正常退出会发 `Leave`；服务端收到后要停止广播它的实体，
    /// 否则每个客户端都会看到一个站着的幽灵 —— 见 game.rs::step_net_server）。
    pub fn unregister(&mut self, addr: SocketAddr) -> Option<u32> {
        self.last_seen.remove(&addr);
        self.pending_nonces.remove(&addr);
        self.pending_names.remove(&addr);
        self.tokens.remove(&addr);
        self.clients.remove(&addr)
    }

    /// 已注册客户端数量
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// 当前已注册的全部玩家 id —— **广播名单的唯一真源**（超时/离场/任何移除路径都经这里
    /// 反映出来，见 `game.rs::step_net_server` 用它停掉已离场玩家的广播）。
    pub fn player_ids(&self) -> Vec<u32> {
        self.clients.values().copied().collect()
    }

    /// 移除超过 `timeout` 未收到任何数据报的客户端，返回被移除的玩家 id。
    /// UDP 尽力而为：超时即丢弃注册。客户端侧会自动重新握手（`reset_connection` +
    /// `retry_join`），但那是以**新 id** 重新加入 ⇒ 服务端这边的旧 id 不会被恢复。
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
        loop {
            let (msg, from) = match recv_msg(&self.socket)? {
                Some(p) => p,
                None => return Ok(None),
            };
            self.last_seen.insert(from, Instant::now());
            match msg {
                // S2：封印包先校验token，再解内层（伪造源地址的 Input/Leave 在这里被丢掉）。
                NetworkMessage::Sealed { token, body } => {
                    if self.tokens.get(&from).copied() != Some(token) {
                        log::debug!("net: 丢弃token不匹配的封印包（伪造源地址？）");
                        continue;
                    }
                    match NetworkMessage::decode(&body) {
                        Ok(inner) => return Ok(Some((inner, from))),
                        Err(e) => {
                            log::debug!("net: 封印包内层解码失败（{e}），丢弃");
                            continue;
                        }
                    }
                }
                other => return Ok(Some((other, from))),
            }
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
    /// 测试用：当前已注册客户端数（判据 `register_refuses_beyond_limits` 用）
    #[cfg(test)]
    pub fn client_count_for_test(&self) -> usize {
        self.clients.len()
    }

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

    /// 处理一条 Join：自动注册该地址，回发分配后的 Join 确认（内部已 `send_to`，调用方无需再发送）。
    /// 协议版本不匹配 → 回 Refuse（不注册客户端，阻止旧版本客户端误连）
    pub fn handle_join(&mut self, from: SocketAddr, name: String, version: u16) -> io::Result<NetworkMessage> {
        if version != SESSION_VERSION {
            let refuse = NetworkMessage::Refuse {
                // `VERSION` 前缀 = 永久拒绝（重试无用）；客户端按前缀分类，不靠中文文案。
                reason: format!("VERSION 协议版本不匹配（客户端 {version}，服务器 {SESSION_VERSION}）"),
            };
            self.send_to(&refuse, from)?;
            return Ok(refuse);
        }
        // S2：已注册的客户端重发 Join ⇒ 直接重发 Welcome（不重复挑战）。
        if let (Some(id), Some(token)) = (
            self.clients.get(&from).copied(),
            self.tokens.get(&from).copied(),
        ) {
            let welcome = NetworkMessage::Welcome { player_id: id, token };
            self.send_to(&welcome, from)?;
            return Ok(welcome);
        }
        // S2：先发挑战，客户端回显 nonce 后才注册 —— 伪造源地址收不到挑战，注册因此不可伪造。
        let nonce = fresh_nonce(from);
        self.pending_nonces.insert(from, nonce);
        self.pending_names.insert(from, name);
        let challenge = NetworkMessage::Challenge { nonce };
        self.send_to(&challenge, from)?;
        Ok(challenge)
    }

    /// S2：客户端回显挑战 nonce → 注册并下发会话token（内部已发送 Welcome）。
    pub fn handle_join_proof(
        &mut self,
        from: SocketAddr,
        nonce: Token,
        version: u16,
    ) -> io::Result<NetworkMessage> {
        if version != SESSION_VERSION {
            let refuse = NetworkMessage::Refuse {
                reason: format!("VERSION 协议版本不匹配（客户端 {version}，服务器 {SESSION_VERSION}）"),
            };
            self.send_to(&refuse, from)?;
            return Ok(refuse);
        }
        match self.pending_nonces.remove(&from) {
            Some(expected) if expected == nonce => {
                let name = self.pending_names.remove(&from).unwrap_or_default();
                let _ = name;
                let Some(player_id) = self.register(from) else {
                    let refuse = NetworkMessage::Refuse {
                        reason: "BUSY 服务器已满（或玩家 id 已耗尽）".to_string(),
                    };
                    self.send_to(&refuse, from)?;
                    return Ok(refuse);
                };
                let token = derive_token(&nonce, from, player_id);
                self.tokens.insert(from, token);
                let reply = NetworkMessage::Welcome { player_id, token };
                self.send_to(&reply, from)?;
                Ok(reply)
            }
            _ => {
                let refuse = NetworkMessage::Refuse {
                    reason: "VERSION 握手 nonce 不匹配".to_string(),
                };
                self.send_to(&refuse, from)?;
                Ok(refuse)
            }
        }
    }

    /// S2：已注册客户端与各自token（快照要逐客户端封装）。
    pub fn clients_with_tokens(&self) -> Vec<(SocketAddr, Token)> {
        self.clients
            .keys()
            .filter_map(|a| self.tokens.get(a).map(|t| (*a, *t)))
            .collect()
    }

    /// S2：直接发送原始字节（封印包用）。
    pub fn send_raw(&self, bytes: &[u8], to: SocketAddr) -> io::Result<usize> {
        self.socket.send_to(bytes, to)
    }
}

/// UDP 客户端：连接服务器，维护自身 id、远端玩家插值缓冲与快照实体插值表
/// `Refuse` 的原因是否属于「重试无用」的永久拒绝。
///
/// 只有协议/版本不匹配是永久的；`BUSY`（服务器满、玩家 id 耗尽）是瞬态——
/// 旧版把所有 Refuse 都当成永久 ⇒ 安全复核 S6：占满 64 个名额后真实玩家**永远无法重连**。
/// 未知原因按永久处理（服务器明确说了不，不制造重连风暴）。
pub fn refuse_is_permanent(reason: &str) -> bool {
    !reason.starts_with("BUSY") && (reason.starts_with("VERSION") || reason.contains("版本不匹配"))
}

pub struct Client {
    socket: UdpSocket,
    server: SocketAddr,
    player_id: Option<u32>,
    /// 服务器拒绝原因（协议版本不匹配等；Some = 停止重试）
    refused_reason: Option<String>,
    remote_players: HashMap<u32, RemotePlayer>,
    clock_start: Instant,
    /// 最近快照序号（丢弃乱序/重复）
    snapshot_seq: u32,
    /// 最近快照的服务端模拟时间（秒）
    snapshot_time: f32,
    /// 是否收到过快照
    has_snapshot: bool,
    /// 服务端本机（房主）玩家的权威状态。
    ///
    /// ⚠️ 这是**房主**的状态，不是客户端自己的：客户端自身的位置校正在 `game.rs` 里走
    /// `entity_state_at(NET_PLAYER_BASE + own)`（服务端把每个客户端也当远端实体广播）。
    /// 当前消费点只有 `game.rs` 的诊断行；重连会把它清掉（见 `reset_connection`）。
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
    /// 上一帧快照的**接收时刻**（本地时钟，秒）：用于估快照间隔
    last_snapshot_at: f64,
    /// 远端实体插值延迟（秒）≈ 快照间隔：渲染时刻取 `now - delay`。
    ///
    /// 🔴 2026-09-26 加：以前 `RemotePlayer::delay` **从来没被赋过值**（恒 0），
    /// 于是 `state_at(now)` 里 `now > curr_time` ⇒ alpha 被 clamp 到 1 ⇒ 插值退化成
    /// "直接用最新快照" ⇒ 远端玩家/实体按快照频率**一顿一顿地跳**（包抖动时更明显）。
    /// 取 delay = 一个快照间隔，渲染点就落回 [prev_time, curr_time] 区间内 ⇒ 平滑，
    /// 代价是 0~1 个快照间隔的显示延迟（标准做法，见 `interp_delay`）。
    interp_delay: f64,
    /// S2：最近一次收到的挑战 nonce（下一次 Join 必须回显它）
    pending_nonce: Option<Token>,
    /// S2：会话token（Welcome 之后才有；Input/Leave 用封印包携带它）
    token: Option<Token>,
}

impl Client {
    /// 连接服务器（本地绑定**按服务端地址族**选未指定地址，随机端口）。
    ///
    /// 🔴 **2026-10-15 修（外部复核指出，已对着代码确认）**：这里原本硬编码
    /// `UdpSocket::bind("127.0.0.1:0")`（连文档注释都写着"绑定 127.0.0.1"）。
    /// 绑环回地址的套接字**只能向环回接口发包** —— 传局域网/公网地址时
    /// `send_to` 直接 `EINVAL`/`ENETUNREACH` ⇒ **除本机回环外任何跨机联机全部失败**。
    ///
    /// 为什么仓库里"双进程真机握手已验通"没抓到：那对进程**跑在同一台机器上、走环回**
    /// ⇒ 正好落在唯一能用的那条路上。**判据本身没错，是覆盖面没覆盖到这个维度。**
    ///
    /// 按服务端地址族绑 `0.0.0.0:0`（IPv4）或 `[::]:0`（IPv6），端口交给内核分配。
    pub fn connect(server: impl ToSocketAddrs) -> io::Result<Client> {
        // 先解析出服务端地址（只为确定地址族），再据此选绑定地址
        let server_addr = server
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "服务端地址解析为空"))?;
        let bind_addr: SocketAddr = if server_addr.is_ipv4() {
            SocketAddr::from(([0u8, 0, 0, 0], 0))
        } else {
            SocketAddr::from(([0u16; 8], 0))
        };
        let socket = UdpSocket::bind(bind_addr)?;
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
            pending_nonce: None,
            token: None,
            refused_reason: None,
            own_state: None,
            entities: HashMap::new(),
            last_rx: Instant::now(),
            timeout: CLIENT_TIMEOUT,
            // 首次 retry_join 立即发送握手包（checked_sub：进程启动不足 1 小时时无下溢）
            last_join_at: Instant::now()
                .checked_sub(Duration::from_secs(3600))
                .unwrap_or_else(Instant::now),
            objective: Vec::new(),
            objective_rule: String::new(),
            objective_seq: 0,
            has_objective: false,
            last_snapshot_at: 0.0,
            interp_delay: 0.0,
        })
    }

    /// 远端实体插值延迟（秒）：渲染时刻 = `本地时刻 - 它`。0 = 还没有两个快照可量。
    pub fn interp_delay(&self) -> f64 {
        self.interp_delay
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
        // S2：拿到token后，Input/Leave 一律封在token包里 —— 伪造源地址的包服务端直接丢弃。
        let sealed;
        let outgoing = match (msg, self.token) {
            (
                NetworkMessage::Input { .. } | NetworkMessage::Leave { .. },
                Some(token),
            ) => {
                sealed = NetworkMessage::Sealed {
                    token,
                    body: msg.encode(),
                };
                &sealed
            }
            _ => msg,
        };
        let buf = outgoing.encode();
        self.socket.send_to(&buf, self.server)
    }

    /// 非阻塞接收：无数据时返回 `Ok(None)`
    pub fn recv(&self) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
        // 🔴 **只接受来自 `self.server` 的数据报**（2026-10-19 外部复核缺陷 3-B，已确认）：
        // `Client::connect` 只 `UdpSocket::bind`、**没有 `connect(server)`** ⇒ 内核不过滤来源，
        // 任何发往本机端口的包都会进 `recv()`；而调用方（`game/net.rs:305`）原来把 `from` 丢掉
        // ⇒ 局域网里的伪造 `Refuse` / `Join` 可以断连或改写 `player_id`。
        // 兄弟函数 `Server::register` 一直按 `from` 记录，这里补齐同一约束。
        loop {
            match recv_msg(&self.socket) {
                Ok(Some((msg, from))) => {
                    if from == self.server {
                        // S2：服务端下行同样带token；token不符的快照/目标状态一律丢弃。
                        if self.token.is_some()
                            && matches!(
                                msg,
                                NetworkMessage::Snapshot { .. } | NetworkMessage::ObjectiveState { .. }
                            )
                        {
                            log::debug!("net: 丢弃未封印的下行状态包（伪造？）");
                            continue;
                        }
                        if let NetworkMessage::Sealed { token, body } = msg {
                            if self.token != Some(token) {
                                log::debug!("net: 丢弃token不匹配的下行封印包（伪造？）");
                                continue;
                            }
                            match NetworkMessage::decode(&body) {
                                Ok(inner) => return Ok(Some((inner, from))),
                                Err(e) => {
                                    log::debug!("net: 下行封印包内层解码失败（{e}），丢弃");
                                    continue;
                                }
                            }
                        }
                        return Ok(Some((msg, from)));
                    }
                    log::debug!("net: 丢弃非服务器来源的数据报 {from}（期望 {}）", self.server);
                }
                other => return other,
            }
        }
    }

    /// 处理一条消息：Join 确认记录自身 id；Position 更新远端玩家插值缓冲；
    /// Snapshot 进入实体插值表并记录本机权威状态；任何消息都刷新 last_rx（断线判定）。
    ///
    /// 返回值：处理了 `Leave` 时返回离场玩家的 id（调用方据此打一行日志），其余为 `None`。
    pub fn handle_message(&mut self, msg: NetworkMessage) -> Option<u32> {
        self.last_rx = Instant::now();
        let t = self.now();
        let left = match msg {
            NetworkMessage::Challenge { nonce } => {
                // S2：收到挑战后立刻回显（JoinProof 不需要token）。
                self.pending_nonce = Some(nonce);
                if let Err(e) = self.send(&NetworkMessage::JoinProof { nonce }) {
                    log::warn!("net: JoinProof 发送失败（{e}）");
                }
                None
            }
            NetworkMessage::Welcome { player_id, token } => {
                // S2：会话建立 —— 拿到 id 与token。
                self.player_id = Some(player_id);
                self.token = Some(token);
                log::info!("net: client 会话建立 id={player_id}");
                None
            }
            NetworkMessage::JoinProof { .. } | NetworkMessage::Sealed { .. } => None,
            NetworkMessage::Join { player_id, .. } => {
                if player_id != 0 && self.player_id.is_none() {
                    self.player_id = Some(player_id);
                }
                None
            }
            NetworkMessage::Position { player_id, seq, state } => {
                // 🔴 两处此前都缺（2026-10-15，外部复核指出 B4，已确认）：
                //   ① `seq` 被 `..` 忽略 ⇒ **乱序/重复包直接把位置回滚**
                //      （`Snapshot` 那条路一直有 `seq_is_newer`，只有这里漏了）；
                //   ② 新建的 `RemotePlayer` 用默认 `delay = 0.0`，而 **`Snapshot` 那条路
                //      会显式 `e.state.delay = self.interp_delay`**（见 `interp_delay` 的文档：
                //      "delay 从来没被赋过值（恒 0）⇒ 插值退化成直接用最新快照 ⇒ 一顿一顿地跳"）
                //      ⇒ **只修了快照那半边，`Position` 这半边一直是跳的。**
                match self.remote_players.get_mut(&player_id) {
                    Some(rp) => {
                        if rp.update_seq(seq, state, t) {
                            rp.delay = self.interp_delay;
                        }
                    }
                    None => {
                        // 复核 S4：只接受玩家 id 空间（< NET_PLAYER_BASE）内的新 id，且表满即忽略。
                        if player_id < NET_PLAYER_BASE && self.remote_players.len() < MAX_REMOTE_PLAYERS {
                            let mut rp = RemotePlayer::new(player_id, state, t);
                            rp.last_seq = Some(seq);
                            rp.delay = self.interp_delay;
                            self.remote_players.insert(player_id, rp);
                        } else {
                            log::debug!("net: 忽略超量/越界的 Position id={player_id}");
                        }
                    }
                }
                None
            }
            NetworkMessage::Snapshot { seq, time, player_id, player, npcs } => {
                // 丢弃乱序/重复快照（判据 = `seq_is_newer`）
                if self.has_snapshot && !seq_is_newer(seq, self.snapshot_seq) {
                    return None;
                }
                self.has_snapshot = true;
                self.snapshot_seq = seq;
                self.snapshot_time = time;
                // 快照间隔 → 插值延迟（首个样本直接取间隔，之后滑动平均；上限 0.25s 防卡顿后
                // 拖出一个巨大的延迟）。见 `interp_delay` 字段的文档。
                if self.last_snapshot_at > 0.0 {
                    let interval = t - self.last_snapshot_at;
                    if interval > 0.0 && interval <= 0.25 {
                        self.interp_delay = if self.interp_delay == 0.0 {
                            interval
                        } else {
                            self.interp_delay * 0.9 + interval * 0.1
                        };
                    }
                }
                self.last_snapshot_at = t;
                // 服务端本机玩家：进插值表（渲染用）+ 权威状态缓存（修正用）
                self.own_state = Some(player);
                // 🔴 **实体键必须是 `NET_PLAYER_BASE + player_id`，不是裸 `player_id`**
                //    （2026-10-15，外部复核指出 B3，已对着两端代码确认）。
                //
                // `entities` 这张表用的是**实体 id 空间**：
                //   · NPC 占 `0..NET_PLAYER_BASE`（服务端逐个 NPC 从 0 开始编号）；
                //   · **玩家占 `NET_PLAYER_BASE` 起** —— 见 `NET_PLAYER_BASE` 的文档
                //     （服务端把每个客户端当成 `NET_PLAYER_BASE + p.id` 的 NPC 广播，
                //     `game/net.rs` 里就是 `id: NET_PLAYER_BASE + p.id`），
                //     以及 `Leave` 的删除键 `NET_PLAYER_BASE.wrapping_add(player_id)`。
                //
                // 原写法用裸 `player_id` ⇒ **房主（服务端把该字段恒填 0）被写进 `entities[0]`**，
                // 与同一条消息里 id 为 0 的 NPC **撞在同一个槽位**：
                //   · NPC 那一段随后覆盖它 ⇒ 房主在所有客户端画面里消失；
                //   · 且同一帧对 `entities[0]` 连续两次 `state.update(.., t)` ⇒
                //     `prev` 是房主坐标、`curr` 是 0 号 NPC 坐标、两个时间戳相同 ⇒ 插值器损坏；
                //   · 又因为 `Leave` 删的是 `NET_PLAYER_BASE + player_id`（另一个键）
                //     ⇒ 就算没被覆盖，这个实体也**永远删不掉**。
                let host_eid = NET_PLAYER_BASE.wrapping_add(player_id);
                match self.entities.get_mut(&host_eid) {
                    Some(e) => {
                        e.state.update(player, t);
                        e.state.delay = self.interp_delay;
                        e.last_seen = t;
                    }
                    None => {
                        self.entities.insert(
                            host_eid,
                            RemoteEntity {
                                state: RemotePlayer::new(player_id, player, t),
                                hp: 100.0,
                                team: 0, // 服务器玩家默认红营（远端蓝营见 NPC 行）
                                firing: false,
                                last_seen: t,
                            },
                        );
                    }
                }
                // NPC：位置/朝向进插值表，血量取最新值
                for npc in npcs {
                    // 🔴 **跳过与房主实体撞号的 NPC**（2026-10-19 外部复核缺陷 3-C，已确认）：
                    // 上面刚对 `host_eid` 做过一次 `state.update(.., t)`；若服务端/伪造包在
                    // `npcs` 里塞入 `id == NET_PLAYER_BASE + player_id`，这里会对**同一实体**
                    // 在同一帧**第二次** `update(.., t)` ⇒ `prev_time == curr_time`
                    // ⇒ **插值器损坏**（正是本文件 2026-10-15 修过的那类事故）。
                    // 正常协议里 NPC id 恒 < NET_PLAYER_BASE，撞号只可能来自坏数据。
                    if npc.id == host_eid {
                        log::debug!("net: 快照 NPC id 与房主实体撞号（{}），已跳过", npc.id);
                        continue;
                    }
                    let nstate = PlayerState::new(npc.pos, npc.facing);
                    match self.entities.get_mut(&npc.id) {
                        Some(e) => {
                            e.state.update(nstate, t);
                            e.state.delay = self.interp_delay;
                            e.hp = npc.hp;
                            e.team = npc.team;
                            e.firing = npc.firing == 1;
                            e.last_seen = t;
                        }
                        None => {
                            self.entities.insert(
                                npc.id,
                                RemoteEntity {
                                    state: RemotePlayer::new(npc.id, nstate, t),
                                    hp: npc.hp,
                                    team: npc.team,
                                    firing: npc.firing == 1,
                                    last_seen: t,
                                },
                            );
                        }
                    }
                }
                None
            }
            NetworkMessage::Leave { player_id, reason } => {
                // 🔴 2026-09-26：这条以前落在 `_ => {}` 里 —— 服务端既不发（见 game.rs 的
                // 超时分支）客户端也不处理。现在两头都接上：把该玩家的实体与插值缓冲一起清掉，
                // 否则它会以最后一帧的姿态站在场上（客户端对远端玩家实体无条件进画面）。
                let eid = NET_PLAYER_BASE.wrapping_add(player_id);
                self.entities.remove(&eid);
                self.remote_players.remove(&player_id);
                // 返回值 = "确实处理了一次离场"（调用方据此打一行日志；net.rs 不依赖 log）
                let _ = reason;
                return Some(player_id);
            }
            NetworkMessage::ObjectiveState { seq, rule_kind, points, .. } => {
                // 目标状态（据点归属/进度）：乱序/重复丢弃（与 Snapshot 同一条判据）
                if self.has_objective && !seq_is_newer(seq, self.objective_seq) {
                    return None;
                }
                self.has_objective = true;
                self.objective_seq = seq;
                self.objective_rule = rule_kind;
                self.objective = points;
                None
            }
            // 🔴 **这条分支此前不存在**（2026-10-15，外部复核指出，已对着代码确认）：
            // `Refuse` 直接落到下面的 `_ => None` 被静默丢弃。
            // 后果有两层：
            //   ① `refused_reason` 的文档写着「Some = 停止重试」，但**全仓从没给它赋过值**
            //      ⇒ 那句注释是一句一直没做到的说明；
            //   ② `retry_join` 于是永远不停 ⇒ 版本不匹配时**每 interval 发一次 Join、
            //      服务器回一次 Refuse**，形成永不停止的往返；更糟的是 `handle_message`
            //      开头的 `last_rx = now()` 会被每次 Refuse 刷新 ⇒ **客户端以为连接是活的**
            //      （超时逻辑也不会把它判掉）。
            NetworkMessage::Refuse { reason } => {
                // 🔴 **已连上的会话忽略 Refuse**（2026-10-19 外部复核缺陷 3-B 后半，已确认）：
                // `Refuse` 的语义是"你的握手被拒绝"，只应在**还没拿到 player_id** 时处理。
                // 原来无条件写 `refused_reason` ⇒ 局域网里一个伪造包就能让客户端
                // **停止重连并认为"服务器拒绝了我"**（`recv` 现在虽已过滤来源，
                // 但状态机的契约本身也要对）。
                if self.player_id.is_some() {
                    log::debug!("net: 已连上的会话忽略 Refuse（{}）", sanitize_log_field(&reason));
                    return None;
                }
                if refuse_is_permanent(&reason) {
                    if self.refused_reason.is_none() {
                        log::warn!(
                            "握手被永久拒绝（版本不匹配）⇒ 停止重试。原因：{}",
                            sanitize_log_field(&reason)
                        );
                    }
                    self.refused_reason = Some(reason);
                } else {
                    // 瞬态（BUSY）：不记永久拒绝，推迟一个 interval 再重试。
                    log::warn!("服务器暂满 ⇒ 稍后重试。原因：{}", sanitize_log_field(&reason));
                    self.last_join_at = Instant::now();
                }
                None
            }
            _ => None,
        };
        // 复核 S4：实体表硬上限，超量按 last_seen 清掉最久未见的（统一入口，任何插入路径都受约束）。
        if self.entities.len() > MAX_ENTITIES {
            let mut seen: Vec<(u32, f64)> =
                self.entities.iter().map(|(id, e)| (*id, e.last_seen)).collect();
            seen.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let excess = self.entities.len() - MAX_ENTITIES;
            for (id, _) in seen.into_iter().take(excess) {
                self.entities.remove(&id);
            }
            log::debug!("net: 实体表超限，已清掉 {excess} 个最久未见实体");
        }
        left
    }

    /// 清理"已经不再出现在快照里"的远端实体（离场的玩家、消失的 NPC），返回清理数量。
    ///
    /// 🔴 为什么必须有（2026-09-26 修）：服务端超时只摘掉自己的注册表项，而客户端这边
    /// **从来不清理实体表**；再加上 `main.rs` 对 id ≥ `NET_PLAYER_BASE` 的实体是**无条件
    /// 进画面**的 ⇒ 断线（或安静退出）的远端玩家会以最后一帧的姿态**永久站在场上**。
    /// `Leave` 报文能覆盖"服务端主动通知"的情况，这条覆盖"通知丢了/压根没发"的情况。
    ///
    /// `max_age` 传 [`ENTITY_STALE_AFTER`]：小于断线判定时间，即先让幽灵消失再判重连。
    pub fn prune_stale_entities(&mut self, now: f64, max_age: f64) -> usize {
        let before = self.entities.len();
        self.entities
            .retain(|_, e| now - e.last_seen <= max_age);
        let removed = before - self.entities.len();
        // 🔴 实体退场必须把**插值缓冲**一起带走（2026-09-26 复查）：`remote_players` 的另一个
        // 出口只有 `Leave` 报文 —— 而这条兜底存在的理由正是"`Leave` 可能丢包"。
        // 只清 `entities` 的后果：幽灵玩家的插值缓冲永久留在表里（`net: remote_players=N`
        // 只增不减；`remote_state_at()` 对一个早就离场的人照样返回最后一帧）。
        if removed > 0 {
            let entities = &self.entities;
            self.remote_players
                .retain(|id, _| entities.contains_key(&NET_PLAYER_BASE.wrapping_add(*id)));
        }
        removed
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

    /// 服务端本机（房主）玩家的权威状态（见字段注释：**不是**客户端自己的状态）
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
    #[cfg(test)]
    pub(crate) fn token_for_test(&self) -> Option<Token> {
        self.token
    }

    #[cfg(test)]
    pub(crate) fn socket_for_test(&self) -> &UdpSocket {
        &self.socket
    }

    pub fn entities(&self) -> &HashMap<u32, RemoteEntity> {
        &self.entities
    }

    /// 实体 id 在本地时刻 t 的插值状态（位置平滑）
    ///
    /// 消费点：`main.rs` 用它给远端 NPC / 远端玩家取平滑位置（取不到时退回 `state.curr`），
    /// `game.rs` 用它做自身上行校正。**已接线**，不是待办。
    pub fn entity_state_at(&self, id: u32, t: f64) -> Option<PlayerState> {
        self.entities.get(&id).map(|e| e.state.state_at(t))
    }

    /// 超过超时阈值未收到任何数据报（断线基本判定）
    ///
    /// 重连已经接线：`game.rs::step_net_client` 在 `player_id.is_some() && snapshot_timeout()`
    /// 时调 `reset_connection` 并靠 `retry_join` 重新握手（以新 id 加入，会话恢复未做）。
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
    /// 断线重置（2026-08-25 重连支持）：清除已注册 id/实体/超时状态，
    /// 使下一次 retry_join 重新握手（服务器端也会重新注册为新 id）
    ///
    /// 🔴 2026-09-26 复查补：**会话级**状态必须一起清 —— 只清 `player_id`/`entities`/
    /// `has_snapshot` 会留下一条静默故障：新会话的据点序号从 0 重来，而 `has_objective`
    /// 仍是 true、`objective_seq` 还是上一局的大值 ⇒ `seq_is_newer` 把每一条据点消息都判成
    /// 「过期」⇒ **新会话的据点消息全被丢弃**，HUD 停在上一局的进度上（不报错、日志干净）。
    /// `remote_players` / `own_state` 同属会话状态，留着就是幽灵实体那类问题的另一半。
    /// 判据 = `reset_connection_clears_session_scoped_state`。
    pub fn reset_connection(&mut self) {
        self.player_id = None;
        self.token = None;
        self.pending_nonce = None;
        self.entities.clear();
        // 插值缓冲与实体表必须同生共死（见 `prune_stale_entities` 的同一条理由）
        self.remote_players.clear();
        self.has_snapshot = false;
        self.own_state = None;
        self.objective.clear();
        self.objective_rule.clear();
        self.has_objective = false;
        self.objective_seq = 0;
        // 安全复核 S6：永久拒绝状态必须随会话重置清掉，否则"满"过一次就再也连不上。
        self.refused_reason = None;
        // 🔴 时间基减法必须 `checked_sub`：`Instant` 的原点在 Windows 上是**开机时刻**，
        // 所以"进程启动不足 1 小时"时 `Instant::now() - 3600s` 会**下溢 panic**
        // （2026-09-27 早上重启后跑测试，`reset_connection_clears_session_scoped_state`
        //  就是这条红的；同一份代码在开机几天的机器上一路绿 ⇒ 典型的"上线首小时才炸"）。
        // 推不到 1 小时就退化成"现在"：语义上只把立刻重试推迟一个 interval，无副作用。
        let now = Instant::now();
        self.last_join_at = now.checked_sub(Duration::from_secs(3600)).unwrap_or(now);
    }

    pub fn retry_join(&mut self, name: &str, interval: Duration) -> bool {
        if self.player_id.is_some() {
            return true;
        }
        // 🔴 被服务器**明确拒绝**过就不再重试（2026-10-15 补）。
        // `refused_reason` 的文档一直写着「Some = 停止重试」，但此前它既没被赋值、
        // 也没被读过 ⇒ 那句话一直没有做到，实际行为是**无限重连**。
        // 粘性是刻意的：版本不匹配靠重试解决不了，得换客户端/换服务端。
        // 要恢复重试就新建一个 `Client`（字段在 `connect` 里初始化为 None）。
        if self.refused_reason.is_some() {
            return false;
        }
        if self.last_join_at.elapsed() >= interval {
            let _ = self.send(&NetworkMessage::Join {
                player_id: 0,
                name: name.to_string(),
                version: SESSION_VERSION,
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
    /// 🔴 **客户端必须丢弃「非服务器来源」的数据报**（2026-10-19 外部复核缺陷 3-B，已确认）。
    ///
    /// `Client::connect` 只 `UdpSocket::bind`、没有 `connect(server)` ⇒ 内核不过滤来源，
    /// 任何能往本机端口发 UDP 的进程都能塞进伪造的 `Refuse`（断连）或 `Join`（改 player_id）。
    ///
    /// 判据要**双向**：外来包被丢 + 服务器包照收（否则"永远返回 None"也能绿）。
    #[test]
    fn client_ignores_datagrams_from_a_foreign_source() {
        use std::net::UdpSocket;
        let server = Server::bind("127.0.0.1:0").unwrap();
        let client = Client::connect(server.local_addr().unwrap()).unwrap();
        let client_addr = client.local_addr().unwrap();
        let attacker = UdpSocket::bind("127.0.0.1:0").unwrap();
        let refuse = NetworkMessage::Refuse { reason: "伪造".into() };
        let bytes = refuse.encode();

        attacker.send_to(&bytes, client_addr).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(
            client.recv().unwrap().is_none(),
            "非服务器来源的数据报必须被丢弃（否则一个伪造 Refuse 就能让客户端断连）"
        );

        // 正对照：同一个报文从服务器地址发来，必须能收到
        server.send_to(&refuse, client_addr).unwrap();
        let got = client.recv().unwrap();
        assert!(got.is_some(), "服务器来源的数据报必须能收到（正对照）");
    }

    /// 🔴 **快照里与房主实体撞号的 NPC 必须被跳过**（2026-10-19 外部复核缺陷 3-C，已确认）。
    ///
    /// `handle_message` 先对 `host_eid = NET_PLAYER_BASE + player_id` 做一次
    /// `state.update(.., t)`；若同一快照的 `npcs` 里又有 `id == host_eid`，
    /// 会对同一实体在同一帧**第二次** update ⇒ `prev_time == curr_time` ⇒ 插值器损坏。
    #[test]
    fn snapshot_npc_colliding_with_host_entity_is_skipped() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        let host_eid = NET_PLAYER_BASE; // player_id = 0 ⇒ host_eid = NET_PLAYER_BASE
        let host = PlayerState::new([1.0, 0.0, 0.0], 0.0);
        let npc = NpcSnapshot {
            id: host_eid,
            pos: [9.0, 0.0, 0.0],
            facing: 0.0,
            hp: 50.0,
            team: 1,
            firing: 0,
        };
        client.handle_message(NetworkMessage::Snapshot {
            seq: 1,
            time: 0.0,
            player_id: 0,
            player: host,
            npcs: vec![npc],
        });
        let e = client.entities().get(&host_eid).expect("房主实体必须存在");
        assert!(
            (e.state.curr.pos[0] - 1.0).abs() < 1e-5,
            "房主位置被撞号 NPC 覆盖了（实际 {}），插值器会被同一帧两次 update 破坏",
            e.state.curr.pos[0]
        );
        assert!((e.hp - 100.0).abs() < 1e-5, "房主血量不该被 NPC 行覆盖");
    }

    /// 🔴 **玩家 id 与客户端数必须有上限**（2026-10-19 外部复核缺陷 3-D，已确认）。
    ///
    /// 无界累加 ⇒ `NET_PLAYER_BASE + player_id` 回绕落进 NPC id 区间 ⇒ 玩家与 NPC 串号。
    #[test]
    fn register_refuses_beyond_limits() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        // 正常路径：连续注册拿到递增 id
        let a: SocketAddr = "127.0.0.1:10001".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:10002".parse().unwrap();
        assert_eq!(server.register(a), Some(1));
        assert_eq!(server.register(b), Some(2));
        // 已注册的地址重复注册返回同一个 id（不递增）
        assert_eq!(server.register(a), Some(1));
        // 上限：客户端数封顶 64
        for i in 0..100u32 {
            let addr: SocketAddr = format!("127.0.0.1:{}", 20000 + i).parse().unwrap();
            let _ = server.register(addr);
        }
        assert!(
            server.client_count_for_test() <= 64,
            "客户端数必须封顶（实际 {}）",
            server.client_count_for_test()
        );
    }

    /// 🔴 **乱序 / 重复的 `Position` 必须被丢弃，不能把远端玩家的位置回滚。**
    ///
    /// 存在理由（2026-10-15，外部复核指出 B4，已确认）：`handle_message` 的
    /// `NetworkMessage::Position { player_id, state, .. }` 把 `seq` 用 `..` **直接忽略**
    /// ⇒ UDP 不保证顺序，一个迟到的包就能把远端玩家**拽回旧位置**（然后下一帧再跳回来）。
    /// `Snapshot` 那条路一直有 `seq_is_newer` 判据，只有 `Position` 漏了。
    ///
    /// ⚠️ 判据要**先确认正常路径是通的**（教训 27）：只断言"乱序被丢"的话，
    /// 一个**永远丢弃所有 Position** 的实现也能通过。
    #[test]
    fn out_of_order_position_is_dropped_not_rolled_back() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        let pos = |x: f32, seq: u32| NetworkMessage::Position {
            player_id: 7,
            seq,
            state: PlayerState::new([x, 0.0, 0.0], 0.0),
        };

        // 正常路径：seq 递增必须被接受，位置跟着走
        let _ = client.handle_message(pos(10.0, 1));
        assert_eq!(
            client.remote_players().get(&7).map(|rp| rp.curr.pos[0]),
            Some(10.0),
            "前置失败：正常的 Position 没被接受（后面的判据就什么也没证明）"
        );
        let _ = client.handle_message(pos(20.0, 2));
        assert_eq!(
            client.remote_players().get(&7).map(|rp| rp.curr.pos[0]),
            Some(20.0),
            "seq 递增的 Position 应更新位置"
        );

        // 乱序：一个更旧的 seq 到达 —— **必须被丢**，不能把位置拽回 10
        let _ = client.handle_message(pos(10.0, 1));
        assert_eq!(
            client.remote_players().get(&7).map(|rp| rp.curr.pos[0]),
            Some(20.0),
            "迟到的旧包把位置回滚了 —— 这就是 `seq` 被忽略的后果"
        );

        // 重复：同一 seq 再来一次，同样丢弃（seq_is_newer 对相等返回 false）
        let _ = client.handle_message(pos(99.0, 2));
        assert_eq!(
            client.remote_players().get(&7).map(|rp| rp.curr.pos[0]),
            Some(20.0),
            "重复包（同一 seq）不该改状态"
        );

        // 再递增：必须恢复接收（别把"丢弃"写成"之后全丢"）
        let _ = client.handle_message(pos(30.0, 3));
        assert_eq!(
            client.remote_players().get(&7).map(|rp| rp.curr.pos[0]),
            Some(30.0),
            "乱序包之后，正常递增的包必须照常接受"
        );
    }

    /// 🔴 **一条解不开的数据报不许打断整帧的收包循环。**
    ///
    /// 存在理由（2026-10-15，外部复核指出 B4，已确认）：`recv_msg` 原来解不开就返回
    /// `Err(InvalidData)`，而所有调用方都是 `while let Ok(Some(..)) = recv()`
    /// ⇒ **一条杂包就把整帧的收包循环打断**，排在它后面的合法报文要等下一帧。
    /// 中继打洞的 `b"PING"` 每次连接都会发 —— 这不是边角情况。
    ///
    /// 判据：先塞一条**非本协议**数据报、紧跟着塞一条**合法** Join，
    /// 然后 `recv()` 必须拿到那条 **Join**（而不是 `None` / `Err`）。
    #[test]
    fn junk_datagram_does_not_block_the_valid_one_behind_it() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        // ① 打洞 PING：不是本协议（协议头是魔数 'S'）
        sock.send_to(b"PING", addr).unwrap();
        // ② 紧跟着一条合法的 Join
        let join = NetworkMessage::Join {
            player_id: 0,
            name: "p".into(),
            version: SESSION_VERSION,
        };
        sock.send_to(&join.encode(), addr).unwrap();

        // 给内核一点时间把两条都排进队列（本机回环）
        std::thread::sleep(Duration::from_millis(20));

        let got = server.recv().expect("recv 不该返回 Err（杂包不是 I/O 错误）");
        let (msg, _from) = got.expect(
            "recv 返回 None ⇒ 那条杂包把后面的合法报文挡住了（或整批被吞）\
             —— 这正是「while let Ok(Some(..)) 被一条 Err 打断」的形态",
        );
        assert_eq!(msg, join, "应拿到杂包**后面**的那条 Join");
    }

    /// 🔴 **房主（`player_id = 0`）与 0 号 NPC 必须落在不同的实体槽位。**
    ///
    /// 存在理由（2026-10-15，外部复核指出 B3，已对着**两端**代码确认）：
    /// 服务端构造 `Snapshot` 时把 `player_id` **恒填 0**（`game/net.rs`），
    /// 而 `player` 是房主的状态；同一条消息的 `npcs` 里另有从 **0 开始编号**的 NPC。
    /// 客户端此前用**裸 `player_id`** 当实体键 ⇒ **房主被写进 `entities[0]`**，
    /// 与 0 号 NPC 撞在同一个槽位：
    ///   · NPC 那一段随后覆盖它 ⇒ **房主在所有客户端画面里消失**；
    ///   · 且同一帧对 `entities[0]` 连续两次 `state.update(.., t)` ⇒ `prev` 是房主坐标、
    ///     `curr` 是 0 号 NPC 坐标、两个时间戳相同 ⇒ **插值器损坏**；
    ///   · 又因为 `Leave` 删的是 `NET_PLAYER_BASE + player_id`（**另一个键**）
    ///     ⇒ 就算没被覆盖，这个实体也**永远删不掉**。
    ///
    /// 正确约定：`entities` 用**实体 id 空间** —— NPC 占 `0..NET_PLAYER_BASE`，
    /// 玩家占 `NET_PLAYER_BASE` 起（见常量文档、`game/net.rs` 的 `NET_PLAYER_BASE + p.id`、
    /// 以及 `Leave` 的删除键）。三处必须同源。
    #[test]
    fn snapshot_host_and_npc_zero_land_in_distinct_slots() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        let host = PlayerState::new([10.0, 0.0, 10.0], 0.0);
        let npc0 = NpcSnapshot {
            id: 0,
            pos: [50.0, 0.0, 50.0],
            facing: 0.0,
            hp: 100.0,
            team: 0,
            firing: 0,
        };
        let _ = client.handle_message(NetworkMessage::Snapshot {
            seq: 1,
            time: 1.0,
            player_id: 0, // 服务端恒填 0
            player: host,
            npcs: vec![npc0],
        });

        let ents = client.entities();
        // ① 两个槽位**都存在**（旧写法只有 entities[0]，房主那份被覆盖掉）
        assert!(
            ents.contains_key(&0),
            "0 号 NPC 应在 entities[0]（NPC 占 0..NET_PLAYER_BASE）"
        );
        let host_key = NET_PLAYER_BASE;
        assert!(
            ents.contains_key(&host_key),
            "房主应在 entities[NET_PLAYER_BASE + 0] —— 旧写法把它写进 entities[0]，与 0 号 NPC 撞位"
        );
        // ② 两个槽位**互不覆盖**：各留各的坐标
        let npc_at = ents[&0].state.state_at(1.0).pos;
        assert_eq!(
            [npc_at[0], npc_at[2]],
            [50.0, 50.0],
            "0 号 NPC 的坐标被房主覆盖了（或反之）"
        );
        let host_at = ents[&host_key].state.state_at(1.0).pos;
        assert_eq!(
            [host_at[0], host_at[2]],
            [10.0, 10.0],
            "房主的坐标被 0 号 NPC 覆盖了（或反之）"
        );
        // ③ 与 `Leave` 的删除键**同源** —— 否则这个实体永远删不掉
        let _ = client.handle_message(NetworkMessage::Leave { player_id: 0, reason: 0 });
        assert!(
            !client.entities().contains_key(&host_key),
            "Leave 用的键必须与 Snapshot 写入的键一致，否则房主实体永远留在场上"
        );
    }

    /// 🔴 **被服务器拒绝之后，不许再重试 Join。**
    ///
    /// 存在理由（2026-10-15，外部复核指出，已对着代码确认）：
    /// `Refuse` 此前落在 `handle_message` 的 `_ => None` 里被静默丢弃
    /// ⇒ `refused_reason` **永远是 None**（可它的文档一直写着「Some = 停止重试」）
    /// ⇒ `retry_join` 永远不停。版本不匹配时就是**每 interval 发一次 Join、
    /// 服务器回一次 Refuse** 的无限往返；更糟的是 `handle_message` 开头的
    /// `last_rx = now()` 会被每次 Refuse 刷新 ⇒ **客户端以为连接是活的**，
    /// 连超时逻辑都不会把它判掉。
    ///
    /// ⚠️ 判据**不能**写成 `assert!(!client.retry_join(..))`：那个返回值表示「尚未加入」，
    /// 在**拒绝之前**同样是 false ⇒ 对任何实现都成立（教训 1 的形态）。
    /// 要证明的是「**没有再发出 Join 报文**」，所以直接看服务器收不收得到。
    ///
    /// ⚠️ 还必须带**正对照**：先证明同样的调用在拒绝**之前**确实发得出去 ——
    /// 否则"没收到"可能只是 `retry_join` 根本不发，判据恒真。
    #[test]
    fn refuse_stops_retry_join() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        let join = NetworkMessage::Join {
            player_id: 0,
            name: "p".into(),
            version: SESSION_VERSION,
        };

        // ---- 正对照：拒绝之前，retry_join 必须真的发出 Join ----
        let _ = client.retry_join("p", Duration::ZERO);
        let (ctrl, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(ctrl, join, "正对照：收到的应是 Join");
        assert!(client.refused_reason.is_none(), "此时还不该有拒绝记录");

        // ---- 服务器回一条 Refuse，客户端处理 ----
        server
            .send_to(&NetworkMessage::Refuse { reason: "版本不匹配".into() }, from)
            .unwrap();
        let (got, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(got);
        assert!(
            client.refused_reason.is_some(),
            "收到 Refuse 后必须记下原因 —— 否则下一条判据（停止重试）也就无从判起"
        );

        // ---- 拒绝之后：同样的调用不再发 Join ----
        for _ in 0..3 {
            let _ = client.retry_join("p", Duration::ZERO);
        }
        let extra = server.recv_timeout(Duration::from_millis(300)).unwrap();
        assert!(
            extra.is_none(),
            "被拒后仍在重发 Join —— 这就是无限重连风暴（服务器会一直回 Refuse）"
        );
    }

    /// 安全复核 S6：`BUSY`（服务器满）是瞬态 ⇒ 客户端不得记成永久拒绝，且必须继续重发 Join。
    /// 反向验证：把 `refuse_is_permanent` 改成恒 true ⇒ 本判据红。
    #[test]
    fn transient_full_refusal_keeps_retrying() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        client.retry_join("p", Duration::ZERO);
        let (_, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        server
            .send_to(
                &NetworkMessage::Refuse {
                    reason: "BUSY 服务器已满（或玩家 id 已耗尽）".into(),
                },
                from,
            )
            .unwrap();
        let (got, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(got);
        assert!(
            client.refused_reason.is_none(),
            "BUSY 被记成了永久拒绝 ⇒ 服务器满过一次就永远连不上（S6）"
        );
        client.last_join_at = Instant::now().checked_sub(Duration::from_secs(10)).unwrap();
        let _ = client.retry_join("p", Duration::ZERO);
        let again = server.recv_timeout(Duration::from_millis(500)).unwrap();
        assert!(again.is_some(), "BUSY 之后不再重发 Join ⇒ 永久锁死（S6）");
    }

    /// 安全复核 S6：`reset_connection` 必须清掉永久拒绝状态（换服务器/重开会话要能再试）。
    #[test]
    fn reset_connection_clears_permanent_refusal() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        client.retry_join("p", Duration::ZERO);
        let (_, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        server
            .send_to(
                &NetworkMessage::Refuse {
                    reason: "VERSION 协议版本不匹配（客户端 1，服务器 2）".into(),
                },
                from,
            )
            .unwrap();
        let (got, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(got);
        assert!(client.refused_reason.is_some(), "版本不匹配必须是永久拒绝");
        client.reset_connection();
        assert!(
            client.refused_reason.is_none(),
            "reset_connection 没清永久拒绝 ⇒ 重开会话仍被锁死（S6 后半）"
        );
    }

    /// 复核 S4：实体插值表必须有硬上限（旧版一条 65KB 快照可塞 ~2,500 个新 id）。
    /// 反向验证：删掉 `handle_message` 末尾的裁剪 ⇒ 本判据红。
    #[test]
    fn snapshot_entity_table_is_bounded() {
        let mut client = Client::connect("127.0.0.1:9").unwrap();
        let npcs = (0..(MAX_ENTITIES + 50) as u32)
            .map(|id| NpcSnapshot { id, pos: [0.0, 0.0, 0.0], facing: 0.0, hp: 100.0, team: 0, firing: 0 })
            .collect();
        let _ = client.handle_message(NetworkMessage::Snapshot {
            seq: 1,
            time: 0.0,
            player_id: 0,
            player: PlayerState::new([0.0, 0.0, 0.0], 0.0),
            npcs,
        });
        assert!(
            client.entities().len() <= MAX_ENTITIES,
            "实体表 {} 超过上限 {}（S4）",
            client.entities().len(),
            MAX_ENTITIES
        );
    }

    /// 复核 S4：`Position` 只接受玩家 id 空间内的 id，且远端玩家表有上限。
    #[test]
    fn position_for_unknown_or_excess_player_is_ignored() {
        let mut client = Client::connect("127.0.0.1:9").unwrap();
        let st = PlayerState::new([1.0, 0.0, 1.0], 0.0);
        // 越界 id（≥ NET_PLAYER_BASE）必须忽略
        let _ = client.handle_message(NetworkMessage::Position { player_id: NET_PLAYER_BASE + 7, seq: 1, state: st });
        assert_eq!(client.remote_players().len(), 0, "越界 player_id 被写进了远端玩家表（S4）");
        // 合法 id 大量到达 ⇒ 表不得超过 MAX_REMOTE_PLAYERS
        for id in 0..(MAX_REMOTE_PLAYERS as u32 + 10) {
            let _ = client.handle_message(NetworkMessage::Position { player_id: id, seq: 1, state: st });
        }
        assert!(
            client.remote_players().len() <= MAX_REMOTE_PLAYERS,
            "远端玩家表 {} 超过上限 {}（S4）",
            client.remote_players().len(),
            MAX_REMOTE_PLAYERS
        );
    }


    /// S2：服务端必须丢弃 token 不匹配的封印包（伪造源地址的 Input/Leave 在这里失效）。
    /// 反向验证：去掉 `Server::recv` 里的 token 比较 ⇒ 本判据红。
    #[test]
    fn forged_sealed_packet_is_rejected() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        let from = complete_handshake(&mut server, &mut client, "p");
        // 伪造者：另一个 socket 冒充同一个源地址是不可能的，但可以拿错 token 直接发。
        let attacker = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let inner = NetworkMessage::Input {
            seq: 1,
            time: 0.0,
            input: net_input(),
        };
        let bogus = NetworkMessage::Sealed {
            token: [0xAB; TOKEN_LEN],
            body: inner.encode(),
        };
        attacker.send_to(&bogus.encode(), addr).unwrap();
        let got = server.recv_timeout(Duration::from_millis(300)).unwrap();
        assert!(got.is_none(), "token 不匹配的封印包被服务端接受了（S2）");
        // 正对照：真 token 的封印包必须被解出来
        let token = server.clients_with_tokens()[0].1;
        let good = NetworkMessage::Sealed { token, body: inner.encode() };
        client.socket_for_test().send_to(&good.encode(), addr).unwrap();
        let (msg, from2) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from2, from);
        assert!(matches!(msg, NetworkMessage::Input { .. }), "真 token 的封印包应被接受");
    }

    /// S2：挑战发出后，**nonce 不对**的 JoinProof 必须被拒（且不得注册）。
    /// 反向验证：把 nonce 比较改成忽略（`Some(_)`）⇒ 本判据红。
    #[test]
    fn join_proof_with_wrong_nonce_is_refused() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        // 真发一次 Join，让服务端生成 pending nonce
        client
            .send(&NetworkMessage::Join { player_id: 0, name: "p".into(), version: SESSION_VERSION })
            .unwrap();
        let (_, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        let challenge = server.handle_join(from, "p".into(), SESSION_VERSION).unwrap();
        assert!(matches!(challenge, NetworkMessage::Challenge { .. }));
        // 回一个错误的 nonce
        let reply = server.handle_join_proof(from, [0x11; TOKEN_LEN], SESSION_VERSION).unwrap();
        match reply {
            NetworkMessage::Refuse { reason } => assert!(
                reason.starts_with("VERSION"),
                "错 nonce 必须永久拒绝，实际原因：{reason}"
            ),
            other => panic!("期望 Refuse，实际 {other:?}"),
        }
        assert_eq!(server.client_count(), 0, "错 nonce 不得注册客户端");
    }

    /// S2：拿到 token 后，客户端必须丢弃未封印的 Snapshot（伪造源地址注入）。
    #[test]
    fn client_drops_unsealed_snapshot_after_handshake() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = Client::connect(addr).unwrap();
        let from = complete_handshake(&mut server, &mut client, "p");
        assert!(client.token_for_test().is_some(), "握手后客户端必须有 token");
        let snapshot = NetworkMessage::Snapshot {
            seq: 7,
            time: 1.0,
            player_id: 1,
            player: PlayerState::new([9.0, 0.0, 9.0], 0.0),
            npcs: vec![],
        };
        server.send_to(&snapshot, from).unwrap();
        let got = client.recv().unwrap();
        assert!(got.is_none(), "未封印的 Snapshot 被客户端接受了（S2）");
        assert_eq!(client.snapshot_seq(), 0, "未封印快照不得改写快照状态");
    }

    /// S7：网络字符串进日志前必须压成一行（否则伪造的 reason 能伪造整行日志）。
    /// 反向验证：把 `sanitize_log_field` 改成原样返回 ⇒ 本判据红。
    #[test]
    fn network_strings_never_inject_log_lines() {
        let dirty = "ok\nFAKE VUID=0\r\npanics=0\tdone\u{2028}tail";
        let clean = sanitize_log_field(dirty);
        for bad in ['\n', '\r', '\t', '\u{2028}', '\u{2029}'] {
            assert!(!clean.contains(bad), "整理后仍含 {bad:?}：{clean:?}");
        }
        assert!(clean.contains("FAKE"), "内容丢了：{clean:?}");
        assert_eq!(clean.lines().count(), 1, "整理后仍跨行：{clean:?}");
        let long: String = std::iter::repeat('x').take(5000).collect();
        assert!(sanitize_log_field(&long).chars().count() <= 131, "没有截断");
    }

    /// S2 测试辅助：走完 Join → Challenge → JoinProof → Welcome，返回服务端看到的本端地址。
    fn complete_handshake(server: &mut Server, client: &mut Client, name: &str) -> std::net::SocketAddr {
        let join = NetworkMessage::Join {
            player_id: 0,
            name: name.to_string(),
            version: SESSION_VERSION,
        };
        client.send(&join).unwrap();
        let (req, from) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(req, join);
        let challenge = server.handle_join(from, name.to_string(), SESSION_VERSION).unwrap();
        assert!(matches!(challenge, NetworkMessage::Challenge { .. }), "握手第一步必须是挑战");
        let (msg, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(msg); // Challenge → 自动回 JoinProof
        let (proof, from2) = recv_until(|| server.recv(), Duration::from_millis(1000));
        let nonce = match proof {
            NetworkMessage::JoinProof { nonce } => nonce,
            other => panic!("期望 JoinProof，实际 {other:?}"),
        };
        let welcome = server.handle_join_proof(from2, nonce, SESSION_VERSION).unwrap();
        assert!(matches!(welcome, NetworkMessage::Welcome { .. }), "回显正确后必须是 Welcome");
        let (wmsg, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(wmsg);
        from2
    }

    /// 测试辅助：**服务端会看到的**本端地址。
    ///
    /// 🔴 2026-10-15：客户端改成按地址族绑 `0.0.0.0:0`（修"跨机联机全部失败"那条）之后，
    /// `client.local_addr()` 的 IP 是**未指定地址 `0.0.0.0`**，而**内核会按目的地址选源地址**
    /// —— 本模块的单测一律连 `127.0.0.1`，所以服务端看到的源地址是 `127.0.0.1:<同一个端口>`。
    ///
    /// 于是"本端绑定地址 == 对端看到的源地址"这个**旧前提不再成立**（它只在绑了具体地址时成立）。
    /// 四处断言原本都直接拿 `client.local_addr()` 去比，全部换成这个函数 ——
    /// **不是代码错，是断言的前提过时了。**
    fn peer_seen_addr(client: &Client) -> std::net::SocketAddr {
        let a = client.local_addr().unwrap();
        std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            a.port(),
        )
    }

    /// 🔴 **客户端连接路径不得把本地套接字绑到环回地址。**
    ///
    /// 存在理由（2026-10-15，外部复核指出）：`Client::connect` 原本写死
    /// `UdpSocket::bind("127.0.0.1:0")` —— 绑环回地址的套接字**只能向环回接口发包**，
    /// 传局域网/公网地址时 `send_to` 直接 `EINVAL`/`ENETUNREACH`
    /// ⇒ **除本机回环外任何跨机联机全部失败**。
    ///
    /// 为什么 `net_pair.sh`（双进程真机握手）没抓到：那对进程**跑在同一台机器上、走环回**，
    /// 正好落在唯一能用的那条路上。**判据本身没错，是覆盖面缺了这个维度。**
    ///
    /// 这里用源码级判据而不是行为判据：`Client` 不暴露内部 socket，
    /// 而真连一个网外地址又要依赖网络环境（不该进单测）。
    /// 判据只扫**非测试段**（`#[cfg(test)]` 之前），否则本文件自己的测试
    /// `Server::bind("127.0.0.1:0")` 会把判据撞红 —— 那些是合法的（服务器本来就该能绑环回）。
    #[test]
    fn client_connect_never_binds_loopback() {
        let src = include_str!("net.rs");
        let production = src
            .split_once("#[cfg(test)]")
            .map(|(a, _)| a)
            .unwrap_or(src);
        // 允许 `Server::bind(addr)` 走参数（那条路本来就由调用方决定绑哪），
        // 只禁止**把环回地址写成字面量**。
        //
        // ⚠️ **必须剔除注释行**：第一版直接对整段做 `contains`，结果命中了我自己
        // 那行引用旧代码的文档注释（`UdpSocket::bind("127.0.0.1:0")`）⇒ 判据**恒红**。
        // 这是"判据匹配到自己的文档"这一类失效，仓库里教训 46 同形。
        let code: String = production
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("//") || t.starts_with("*") || t.starts_with("/*"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        for pat in ["bind(\"127.0.0.1", "bind(\"localhost"] {
            assert!(
                !code.contains(pat),
                "客户端连接路径出现写死的环回绑定 `{pat}` —— \
                 绑环回只能发往本机，跨机联机全部失败。应绑 0.0.0.0:0 / [::]:0"
            );
        }
    }

    use super::*;

    fn state(x: f32, y: f32, z: f32, rot: f32) -> PlayerState {
        PlayerState::new([x, y, z], rot)
    }

    fn sample_messages() -> Vec<NetworkMessage> {
        vec![
            NetworkMessage::Join {
                player_id: 0,
                name: String::new(),
                version: SESSION_VERSION,
            },
            NetworkMessage::Join {
                player_id: 42,
                name: "alice".to_string(),
                version: SESSION_VERSION,
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
            version: SESSION_VERSION,
        };
        let decoded = NetworkMessage::decode(&m.encode()).unwrap();
        assert_eq!(decoded, m);
    }

    #[test]
    fn join_name_truncated_to_255_bytes() {
        let m = NetworkMessage::Join {
                player_id: 1,
                name: "a".repeat(300),
                version: SESSION_VERSION,
            };
        let bytes = m.encode();
        // 头部 + player_id(4) + 长度(1) + 名字(截断到 255) + version(2)
        assert_eq!(bytes.len(), HEADER_LEN + 4 + 1 + 255 + 2);
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

    /// 🔴 判据：**任意字节流都不许让 `decode` panic**（网络输入是不可信的）。
    ///
    /// 现有测试是"每种错误各一例"（坏魔数 / 坏版本 / 未知类型 / 截断 / 尾随 / 坏 UTF-8），
    /// 而**字段层的畸形组合**没人扫过：长度前缀与实际不符、超大 count、NaN 位型、
    /// 合法与非法 UTF-8 混排…… 这些正是伪造报文最容易踩到的路径。
    ///
    /// ⚠️ **第一版写成"合法头部 + 随机载荷"，被自检当场抓住**：2000 条**一条都没解出来**
    /// （随机载荷能整体通过字段校验的概率几乎为 0）⇒ 那种写法下"不 panic"是**空洞的**。
    /// 改成**变异模糊**：从 `sample_messages()` 的真实报文出发做 1~4 次随机变异
    /// （改字节 / 截断 / 追加 / 改写载荷长度字段），再叠加一小部分完全随机的字节流。
    ///
    /// 两条不变式：① 不 panic；② 只要 `decode` 返回 `Ok`，`encode` 出来的字节必须**还能被解码**。
    /// （2026-09-26 起 `Reader::f32` 会拒掉 NaN/±inf ⇒ 能解出的报文不可能带非有限值；
    ///  这里仍只验「解得出」——相等性是另一条判据的事，别把它塞进来。）
    #[test]
    fn decode_never_panics_on_mutated_bytes_and_reencodes() {
        let mut seed: u32 = 0x1234_5678;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        let bases: Vec<Vec<u8>> = sample_messages().iter().map(|m| m.encode()).collect();
        let mut ok_count = 0usize;
        for i in 0..3000 {
            let mut buf = if i % 8 == 7 {
                // 每 8 条里放一条完全随机的（覆盖头部校验路径）
                let len = (next() % 256) as usize;
                (0..len).map(|_| (next() >> 24) as u8).collect::<Vec<u8>>()
            } else {
                bases[(next() as usize) % bases.len()].clone()
            };
            // ⚠️ 变异要**偏向载荷**：五字节头部里有"长度必须与实际一致"的硬约束，
            // 在头部随便改一个字节 ⇒ 消息直接作废，整条测试就退化成"只测头部校验"（第一版栽在这）。
            if buf.len() > HEADER_LEN {
                match next() % 10 {
                    0..=4 => {
                        // 载荷里翻一个字节：多数仍能解析 ⇒ 真正走到字段层的校验
                        let at = HEADER_LEN + (next() as usize) % (buf.len() - HEADER_LEN);
                        buf[at] = (next() >> 24) as u8;
                    }
                    5..=6 => {
                        let at = HEADER_LEN + (next() as usize) % (buf.len() - HEADER_LEN);
                        buf[at] = (next() >> 24) as u8;
                        let at2 = HEADER_LEN + (next() as usize) % (buf.len() - HEADER_LEN);
                        buf[at2] = (next() >> 24) as u8;
                    }
                    7 => {
                        // 长度字段与实际不符（必须被 Truncated/TrailingData 拒掉）
                        let fake = (next() % 1024) as u16;
                        buf[3] = (fake >> 8) as u8;
                        buf[4] = fake as u8;
                    }
                    8 => {
                        let cut = HEADER_LEN + (next() as usize) % (buf.len() - HEADER_LEN);
                        buf.truncate(cut);
                    }
                    _ => buf.push((next() >> 24) as u8),
                }
            }
            if let Ok(msg) = NetworkMessage::decode(&buf) {
                ok_count += 1;
                let re = msg.encode();
                assert!(
                    NetworkMessage::decode(&re).is_ok(),
                    "重编码后的字节必须仍可解码（第 {i} 条）"
                );
            }
        }
        // 自检（教训 27）：变异模糊**必须**有一部分仍然解得出，否则这条测试没走到解析层
        println!("decode fuzz: {ok_count}/3000 条变异报文被接受（其余按错误拒绝，全程无 panic）");
        assert!(
            ok_count >= 50,
            "3000 条变异报文只解出 {ok_count} 条 ⇒ 测试大概没生效（第一版就是这么被抓的）"
        );
    }

    /// 🔴 判据：`reset_connection` 必须把**会话级**状态一起清掉。
    ///
    /// 只清 `player_id`/`entities`/`has_snapshot` 的后果（静默）：新会话的据点序号从 0 重来，
    /// 而 `has_objective` 仍是 true、`objective_seq` 还是上一局的大值 ⇒ `seq_is_newer` 把
    /// 每一条都判成「过期」⇒ **新会话的据点消息全被丢弃**，HUD 永远停在上一局的进度上；
    /// `own_state` / `remote_players` 则是"重连后仍留着上一局实体"的种子。
    #[test]
    fn reset_connection_clears_session_scoped_state() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        // 先造出"上一局"的状态：一份高序号快照 + 一条高序号据点消息
        let snap = NetworkMessage::Snapshot {
            seq: 900,
            time: 12.0,
            player_id: 0,
            player: PlayerState::new([5.0, 0.0, 5.0], 0.5),
            npcs: Vec::new(),
        };
        assert!(client.handle_message(snap).is_none());
        assert!(
            client.own_state().is_some(),
            "快照必须真的写进 own_state（否则这条测试没走到目标路径）"
        );
        let obj = NetworkMessage::ObjectiveState {
            seq: 900,
            time: 12.0,
            rule_kind: "capture".into(),
            points: vec![("A".into(), 1, 0.75)],
        };
        assert!(client.handle_message(obj).is_none());
        assert_eq!(client.objective_state().len(), 1);

        client.reset_connection();

        assert!(
            client.own_state().is_none(),
            "重连后不许留着上一局房主状态"
        );
        assert!(
            client.objective_state().is_empty(),
            "重连后不许留着上一局据点"
        );
        assert!(
            client.objective_rule().is_empty(),
            "重连后不许留着上一局规则名"
        );
        assert!(
            client.entities().is_empty() && client.remote_players().is_empty(),
            "实体表与插值缓冲都要清（幽灵实体的另一半）"
        );

        // 🔴 关键后果：新会话从 seq=1 重来也必须被接受（旧 seq=900 会把它判成「过期」）
        let fresh = NetworkMessage::ObjectiveState {
            seq: 1,
            time: 0.1,
            rule_kind: "capture".into(),
            points: vec![("A".into(), 2, 0.1)],
        };
        assert!(client.handle_message(fresh).is_none());
        assert_eq!(
            client.objective_state().len(),
            1,
            "新会话的据点消息必须被接受（旧序号不许继续挡着）"
        );
        assert_eq!(client.objective_state()[0].1, 2, "归属码应来自新会话");
    }

    /// 🔴 判据：非有限浮点（NaN / ±inf）必须在**线路边界**被拒 ——
    /// 它一旦流进去就是静默故障：NaN 位置 ⇒ 实例矩阵 NaN ⇒ 不画（不崩、不报 VUID、日志干净）；
    /// NaN 时间戳 ⇒ `interpolate_at` 的 `dt <= 0.0` 为假 ⇒ 插值恒走 NaN 分支。
    ///
    /// 三个方向各拒一条（NaN 位置 / NaN 时间戳 / inf 坐标），再加一条正对照：
    /// 同一报文把浮点全换成有限值后**必须**照常解出 —— 否则「拒绝」可能来自别的原因。
    #[test]
    fn non_finite_floats_are_rejected_at_the_wire() {
        let nan_pos = NetworkMessage::Position {
            player_id: 1,
            seq: 7,
            state: PlayerState::new([f32::NAN, 0.0, 0.0], 0.0),
        };
        assert_eq!(
            NetworkMessage::decode(&nan_pos.encode()),
            Err(NetError::NonFiniteField),
            "NaN 位置必须被拒"
        );

        let nan_time = NetworkMessage::Snapshot {
            seq: 1,
            time: f32::NAN,
            player_id: 1,
            player: PlayerState::new([0.0; 3], 0.0),
            npcs: Vec::new(),
        };
        assert_eq!(
            NetworkMessage::decode(&nan_time.encode()),
            Err(NetError::NonFiniteField),
            "NaN 时间戳必须被拒"
        );

        let inf_npc = NetworkMessage::Snapshot {
            seq: 1,
            time: 0.5,
            player_id: 1,
            player: PlayerState::new([0.0; 3], 0.0),
            npcs: vec![NpcSnapshot {
                id: 9,
                pos: [0.0, f32::INFINITY, 0.0],
                facing: 0.0,
                hp: 1.0,
                team: 0,
                firing: 0,
            }],
        };
        assert_eq!(
            NetworkMessage::decode(&inf_npc.encode()),
            Err(NetError::NonFiniteField),
            "inf 坐标必须被拒"
        );

        let finite = NetworkMessage::Snapshot {
            seq: 1,
            time: 0.5,
            player_id: 1,
            player: PlayerState::new([1.0, 0.0, -2.0], 0.25),
            npcs: vec![NpcSnapshot {
                id: 9,
                pos: [1.0, 0.0, 1.0],
                facing: 0.1,
                hp: 100.0,
                team: 0,
                firing: 1,
            }],
        };
        assert!(
            NetworkMessage::decode(&finite.encode()).is_ok(),
            "全有限值的同一报文必须照常解出（正对照）"
        );
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

        // S2 握手（Join → Challenge → JoinProof → Welcome）
        let from = complete_handshake(&mut server, &mut client, "alice");
        assert_eq!(server.client_count(), 1);
        assert_eq!(server.client_id(from), Some(1));
        assert_eq!(client.player_id(), Some(1));

        // client -> server：Position 状态上报
        let pos = NetworkMessage::Position {
            player_id: 1,
            seq: 1,
            state: state(1.0, 2.0, 3.0, 0.5),
        };
        client.send(&pos).unwrap();
        let (got3, from3) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from3, peer_seen_addr(&client));
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
        let _ = client.handle_message(pos);
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
                NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0, team: 0 , firing: 0 },
                NpcSnapshot { id: 11, pos: [-2.0, 0.0, 4.0], facing: -1.0, hp: 50.0, team: 0 , firing: 0 },
            ],
        };
        let bytes = m.encode();
        // HEADER + seq(4) + time(4) + player_id(4) + player(16) + count(2) + 2×26（team+firing）
        assert_eq!(bytes.len(), HEADER_LEN + 30 + 52);
        assert_eq!(bytes[2], MessageType::Snapshot as u8);
        assert_eq!(&bytes[HEADER_LEN + 28..HEADER_LEN + 30], &[0x00, 0x02]);
        assert_eq!(&bytes[HEADER_LEN + 30..HEADER_LEN + 34], &[0, 0, 0, 10]);
        let back = NetworkMessage::decode(&bytes).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn snapshot_npcs_capped_at_max() {
        let npcs = (0..(MAX_SNAPSHOT_NPCS + 50) as u32)
            .map(|id| NpcSnapshot { id, pos: [0.0; 3], facing: 0.0, hp: 1.0, team: 0 , firing: 0 })
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

    /// 🔴 快照容量提示必须按协议上限封顶：`n` 是**报文里的 2 字节**，
    /// 不封顶就是"2 字节换 ~1.6MB 分配"（放大比约 8e5）。红了说明 `min` 被去掉。
    #[test]
    fn snapshot_capacity_hint_is_capped_by_protocol_limit() {
        assert_eq!(snapshot_capacity_hint(0), 0);
        assert_eq!(snapshot_capacity_hint(128), 128);
        assert_eq!(snapshot_capacity_hint(MAX_SNAPSHOT_NPCS), MAX_SNAPSHOT_NPCS);
        assert_eq!(
            snapshot_capacity_hint(u16::MAX as usize),
            MAX_SNAPSHOT_NPCS,
            "伪造的 65535 必须被压到协议上限"
        );
    }

    /// 伪造的 `count = 65535`（载荷不足）⇒ 逐条读取撞边界 → `Truncated`，**不 panic**。
    /// 与 `objective_state_decode_rejects_oversized_point_count` 同一判据，补上快照这一侧。
    #[test]
    fn snapshot_decode_rejects_oversized_npc_count() {
        let msg = NetworkMessage::Snapshot {
            seq: 1,
            time: 0.0,
            player_id: 7,
            player: PlayerState::new([0.0, 0.0, 0.0], 0.0),
            npcs: vec![],
        };
        let mut bytes = msg.encode();
        // 载荷布局：seq(4) + time(4) + player_id(4) + pos(12) + rot(4) = 28B，随后是 count(2)
        let count_off = HEADER_LEN + 28;
        bytes[count_off] = 0xFF;
        bytes[count_off + 1] = 0xFF;
        assert_eq!(NetworkMessage::decode(&bytes), Err(NetError::Truncated));
    }

    /// 快照路径的"任何前缀都不 panic"：去掉尾部若干字节，只能是 `Truncated`
    /// （否则就是某处越界读/切片 panic）。与需求侧同款判据，见 obj 版本。
    #[test]
    fn snapshot_decode_rejects_truncated_prefixes() {
        let msg = NetworkMessage::Snapshot {
            seq: 3,
            time: 1.5,
            player_id: 1,
            player: PlayerState::new([1.0, 2.0, 3.0], 0.25),
            npcs: vec![NpcSnapshot {
                id: 42,
                pos: [4.0, 0.0, 5.0],
                facing: 1.0,
                hp: 100.0,
                team: 0,
                firing: 0,
            }],
        };
        let bytes = msg.encode();
        for cut in 0..bytes.len() {
            assert_eq!(
                NetworkMessage::decode(&bytes[..cut]),
                Err(NetError::Truncated),
                "prefix cut={cut} should be Truncated"
            );
        }
        assert_eq!(NetworkMessage::decode(&bytes), Ok(msg));
    }

    // -----------------------------------------------------------------------
    // UDP loopback：握手 → Input 往返 → Snapshot 往返 → 客户端应用快照
    // -----------------------------------------------------------------------

    /// 序号算术的**边界**：文档（模块头 + 两处调用点）写「差值 ≥ 2^31 视为过期」，
    /// 所以 2^31-1 是**最远的合法前进**、必须被接受；2^31 恰好是半程、必须被判过期。
    ///
    /// 旧实现两处各写 `diff >= u32::MAX / 2`（= 2^31-1）⇒ 把最远合法前进当成过期
    /// （注释与代码差 1）。这条测试就是那个差 1 的判据。
    #[test]
    fn sequence_staleness_uses_the_documented_half_range() {
        // 正对照先来：差 1 必须新（保证下面的断言不是因为函数恒 false）
        assert!(seq_is_newer(8, 7), "前进 1 必须视为新");
        // 跨 0 回绕也是前进：last = u32::MAX，seq = 1 ⇒ 前进 2
        assert!(seq_is_newer(1, u32::MAX), "跨 0 回绕必须视为新");
        // 边界两侧
        assert!(
            seq_is_newer(0x7FFF_FFFF, 0),
            "差值 2^31-1 是最远的合法前进，必须接受（旧写法 >= u32::MAX/2 会把它丢掉）"
        );
        assert!(!seq_is_newer(0x8000_0000, 0), "差值 2^31 恰好是半程 ⇒ 视为过期");
        // 反向一步 = 过期；相等 = 重复
        assert!(!seq_is_newer(u32::MAX, 0), "差值 2^32-1 = 反向一步 ⇒ 过期");
        assert!(!seq_is_newer(7, 7), "相等 = 重复 ⇒ 丢弃");
        // 两条丢弃路径共用同一条判据（单一真源），常量本身也不许再被写错
        assert_eq!(SEQ_HALF_RANGE, 0x8000_0000, "半程 = 2^31，别写成 u32::MAX/2");
    }

    #[test]
    fn udp_loopback_handshake_input_snapshot_roundtrip() {
        let mut server = Server::bind("127.0.0.1:0").unwrap();
        let server_addr = server.local_addr().unwrap();
        let mut client = Client::connect(server_addr).unwrap();

        // S2 握手：Join → Challenge → JoinProof → Welcome（辅助函数内部逐步断言）
        let from = complete_handshake(&mut server, &mut client, "player1");
        assert_eq!(client.player_id(), Some(1));
        assert!(client.is_connected());

        // 客户端上报输入 → 服务端收到（确定性往返）
        let input = net_input();
        client
            .send(&NetworkMessage::Input { seq: 1, time: 0.5, input })
            .unwrap();
        let (msg, from2) = recv_until(|| server.recv(), Duration::from_millis(1000));
        assert_eq!(from2, peer_seen_addr(&client));
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
                NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.25, hp: 100.0, team: 0 , firing: 0 },
                NpcSnapshot { id: 11, pos: [2.0, 0.0, 2.0], facing: -0.5, hp: 40.0, team: 0 , firing: 0 },
            ],
        };
        let token = server.clients_with_tokens().into_iter().next().expect("握手后应有 token").1;
        let sealed = NetworkMessage::Sealed { token, body: snapshot.encode() };
        server.send_to(&sealed, from2).unwrap();
        let (got2, _) = recv_until(|| client.recv(), Duration::from_millis(1000));
        let _ = client.handle_message(got2);
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
        let _ = client.handle_message(snapshot);
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
        let _ = client.handle_message(NetworkMessage::Join { player_id: 1, name: "p".into(), version: SESSION_VERSION });
        assert!(client.player_id().is_some());
        std::thread::sleep(Duration::from_millis(30));
        assert!(client.snapshot_timeout(), "超过阈值无数据报应判定断线");
        assert!(!client.is_connected());
        // 任何数据报刷新 last_rx → 恢复连接判定
        let _ = client.handle_message(NetworkMessage::Join { player_id: 1, name: "p".into(), version: SESSION_VERSION });
        assert!(!client.snapshot_timeout());
        assert!(client.is_connected());
    }

    /// 🔴 判据：**插值延迟必须真的由快照间隔驱动**，且渲染时刻真的落在两个快照之间。
    ///
    /// 真机代价（2026-09-26 复查）：`RemotePlayer::delay` 从来没被赋过值（恒 0）
    /// ⇒ `state_at(now)` 里 `now > curr_time` ⇒ alpha 被 clamp 到 1 ⇒ 插值退化成"用最新快照"
    /// ⇒ 远端实体按快照频率**一顿一顿地跳**。这条测试同时钉住三件事：
    /// ① 两个快照之后 `interp_delay > 0`；② 它约等于快照间隔；③ 用 `entity_state_at` 取到的
    /// 位置**严格落在 prev 与 curr 之间**（既不是 prev 也不是 curr = 真的在插值）。
    #[test]
    fn snapshot_interval_drives_interpolation_delay() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        let snap = |seq: u32, x: f32| NetworkMessage::Snapshot {
            seq,
            time: seq as f32 * 0.02,
            player_id: 0,
            player: state(0.0, 0.0, 0.0, 0.0),
            npcs: vec![NpcSnapshot {
                id: 10,
                pos: [x, 0.0, 0.0],
                facing: 0.0,
                hp: 100.0,
                team: 0,
                firing: 0,
            }],
        };
        assert_eq!(client.handle_message(snap(1, 0.0)), None);
        assert_eq!(client.interp_delay(), 0.0, "只有一个快照时量不出间隔");
        // 第二个快照：间隔 ≈ 20ms（真实 sleep，容忍调度抖动）
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(client.handle_message(snap(2, 10.0)), None);
        let delay = client.interp_delay();
        assert!(
            (0.005..0.25).contains(&delay),
            "延迟应由快照间隔驱动（≈0.02s），实际 {delay}"
        );
        // 渲染时刻 = now - delay 落在 [prev_time, curr_time] 内 ⇒ 位置严格在 0 与 10 之间
        let st = client.entity_state_at(10, client.now()).expect("实体应在表里");
        assert!(
            st.pos[0] > 0.0 && st.pos[0] < 10.0,
            "渲染位置应落在两个快照之间（真的在插值），实际 {}",
            st.pos[0]
        );
        // 兜底：delay 为 0（还没量出间隔）时退化成"最新状态"，不会 panic 也不会 NaN
        let mut fresh = Client::connect(server.local_addr().unwrap()).unwrap();
        assert_eq!(fresh.handle_message(snap(1, 3.0)), None);
        assert_eq!(
            fresh.entity_state_at(10, fresh.now()).map(|s| s.pos[0]),
            Some(3.0),
            "没有第二个快照时取最新状态（与旧行为一致）"
        );
    }

    /// 🔴 判据：`Leave` 必须把该玩家的实体与插值缓冲一起清掉。
    ///
    /// 真机代价（2026-09-26 复查）：客户端以前把 `Leave` 丢进 `_ => {}`、实体表又从不清理，
    /// 而 `main.rs` 对 id ≥ `NET_PLAYER_BASE` 的实体是**无条件进画面**的 ⇒ 断线（或安静退出）
    /// 的远端玩家会以最后一帧的姿态**永久站在场上**。
    #[test]
    fn leave_removes_the_remote_player_entity() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        // 先让快照里出现两个远端玩家（id 5 / 6 ⇒ 实体 id = BASE+5 / BASE+6）与一个 NPC
        let snap = NetworkMessage::Snapshot {
            seq: 1,
            time: 1.0,
            player_id: 0,
            player: state(0.0, 0.0, 0.0, 0.0),
            npcs: vec![
                NpcSnapshot { id: NET_PLAYER_BASE + 5, pos: [5.0, 0.0, 5.0], facing: 0.0, hp: 100.0, team: 1, firing: 0 },
                NpcSnapshot { id: NET_PLAYER_BASE + 6, pos: [6.0, 0.0, 6.0], facing: 0.0, hp: 100.0, team: 1, firing: 0 },
                NpcSnapshot { id: 10, pos: [1.0, 0.0, 1.0], facing: 0.0, hp: 100.0, team: 0, firing: 0 },
            ],
        };
        assert_eq!(client.handle_message(snap), None, "快照不是离场通知");
        assert_eq!(client.entities().len(), 4, "本机玩家 + 2 远端玩家 + 1 NPC");
        // 玩家 5 离场
        let left = client.handle_message(NetworkMessage::Leave { player_id: 5, reason: 0 });
        assert_eq!(left, Some(5), "处理 Leave 时应把离场者 id 报给调用方");
        assert!(
            !client.entities().contains_key(&(NET_PLAYER_BASE + 5)),
            "离场玩家的实体必须被移除（否则就是站在场上的幽灵）"
        );
        assert!(
            client.entities().contains_key(&(NET_PLAYER_BASE + 6)),
            "没离场的人不许被误删"
        );
        assert!(client.entities().contains_key(&10), "NPC 不受 Leave 影响");
    }

    /// 🔴 判据：快照里长时间不出现的实体必须被清掉（`Leave` 可能丢包，这是兜底）。
    /// 同时钉住"**新**实体不会被误清"（`last_seen` 每帧刷新）。
    #[test]
    fn stale_entities_are_pruned_but_fresh_ones_survive() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        let snap = |seq: u32| NetworkMessage::Snapshot {
            seq,
            time: 1.0,
            player_id: 0,
            player: state(0.0, 0.0, 0.0, 0.0),
            npcs: vec![NpcSnapshot {
                id: 10,
                pos: [1.0, 0.0, 1.0],
                facing: 0.0,
                hp: 100.0,
                team: 0,
                firing: 0,
            }],
        };
        assert_eq!(client.handle_message(snap(1)), None);
        assert_eq!(client.entities().len(), 2, "服务端本机玩家(id 0) + NPC(10)");
        // 立刻清理：没有东西过期
        let now = client.now();
        assert_eq!(client.prune_stale_entities(now, ENTITY_STALE_AFTER), 0);
        // 超过陈旧阈值之后再清理：这一帧收到的两个实体都已经"很久没出现"了
        let later = now + ENTITY_STALE_AFTER + 0.001;
        assert_eq!(
            client.prune_stale_entities(later, ENTITY_STALE_AFTER),
            2,
            "超过 ENTITY_STALE_AFTER 没出现的实体应被清理"
        );
        assert!(client.entities().is_empty());
        // 新快照（含 NPC）又回来了 ⇒ 重新出现且不被误清
        assert_eq!(client.handle_message(snap(2)), None);
        let now2 = client.now();
        assert_eq!(client.entities().len(), 2, "本机玩家 + NPC");
        assert_eq!(client.prune_stale_entities(now2, ENTITY_STALE_AFTER), 0);
        assert_eq!(client.entities().len(), 2);
    }

    /// 🔴 判据：兜底清理必须把**插值缓冲**一起退场 —— `remote_players` 的另一个出口只有
    /// `Leave` 报文，而这条兜底存在的理由正是"`Leave` 可能丢包"。
    ///
    /// 真机代价（2026-09-26 复查）：`prune_stale_entities` 原来只 `retain` 了 `entities`，
    /// `remote_players` 谁都不删 ⇒ 幽灵玩家的插值缓冲永久留在表里：`net: remote_players=N`
    /// 只增不减，`remote_state_at()` 对一个早就离场的人照样返回最后一帧。
    #[test]
    fn stale_prune_also_drops_the_interpolation_buffer() {
        let server = Server::bind("127.0.0.1:0").unwrap();
        let mut client = Client::connect(server.local_addr().unwrap()).unwrap();
        // 远端玩家 7：`Position` 报文建插值缓冲；快照里以 NPC 行（id = BASE+7）建实体
        assert_eq!(
            client.handle_message(NetworkMessage::Position {
                player_id: 7,
                seq: 1,
                state: state(7.0, 0.0, 7.0, 0.0),
            }),
            None
        );
        assert!(
            client.remote_players().contains_key(&7),
            "Position 报文应建立插值缓冲"
        );
        let snap = NetworkMessage::Snapshot {
            seq: 1,
            time: 1.0,
            player_id: 0,
            player: state(0.0, 0.0, 0.0, 0.0),
            npcs: vec![NpcSnapshot {
                id: NET_PLAYER_BASE + 7,
                pos: [7.0, 0.0, 7.0],
                facing: 0.0,
                hp: 100.0,
                team: 1,
                firing: 0,
            }],
        };
        assert_eq!(client.handle_message(snap), None);
        assert!(client.entities().contains_key(&(NET_PLAYER_BASE + 7)));
        // 还新鲜：实体与插值缓冲都不许动
        let now = client.now();
        assert_eq!(client.prune_stale_entities(now, ENTITY_STALE_AFTER), 0);
        assert!(
            client.remote_players().contains_key(&7),
            "新鲜时不许清插值缓冲"
        );
        // 超过陈旧阈值：实体（本机玩家 + 远端玩家）退场，插值缓冲必须跟着退场
        let later = now + ENTITY_STALE_AFTER + 0.001;
        assert_eq!(client.prune_stale_entities(later, ENTITY_STALE_AFTER), 2);
        assert!(!client.entities().contains_key(&(NET_PLAYER_BASE + 7)));
        assert!(
            !client.remote_players().contains_key(&7),
            "实体退场时插值缓冲必须一起退场（否则就是永久幽灵）"
        );
        assert!(
            client.remote_state_at(7, later).is_none(),
            "已离场的人不该再插值出状态"
        );
    }
}

#[cfg(test)]
mod rdv_reply_tests {
    use super::parse_rdv_reply;

    /// 🔴 **中继对 `REG` 的回复必须被解析**（2026-10-17，外部复核指出）。
    ///
    /// 原来客户端对 `REG` 是 **fire-and-forget**（`send_to` 之后直接 `Ok(())`），
    /// 而中继会回 `OK` / `BAD port` / `BUSY name` / `FULL` —— **一个都收不到**，
    /// 注册失败在客户端完全静默（而 `rdv_register_report` 只能报"发送失败"）。
    ///
    /// 这条判据钉住四个分支 + `OK <addr>` 的新字段（STUN 式反馈，旧中继无地址仍兼容）。
    #[test]
    fn reg_reply_is_parsed_and_errors_are_surfaced() {
        // 旧版中继：只有 OK，没有地址 ⇒ Ok(None)（向后兼容）
        assert_eq!(parse_rdv_reply("OK"), Ok(None));
        // 新版中继：OK 带**观测到的公网地址**
        let addr: std::net::SocketAddr = "203.0.113.7:54321".parse().unwrap();
        assert_eq!(parse_rdv_reply("OK 203.0.113.7:54321"), Ok(Some(addr)));
        // 三种拒绝都必须变成 Err，而不是静默
        assert!(parse_rdv_reply("BAD port").is_err(), "BAD 必须报错");
        assert!(parse_rdv_reply("BUSY name").is_err(), "BUSY 必须报错");
        assert!(parse_rdv_reply("FULL").is_err(), "FULL 必须报错");
        // 无法识别的回复也不能当成功
        assert!(parse_rdv_reply("WAT").is_err());
        assert!(parse_rdv_reply("").is_err());
    }
}
