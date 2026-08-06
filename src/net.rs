//! 网络同步模块
//!
//! - UDP 客户端-服务器架构：基于 `std::net::UdpSocket` 封装 `Server` / `Client`，
//!   loopback 即可测试，无需真正联网
//! - 玩家位置/旋转序列化：手写字节编码（f32 位置 + f32 旋转），带协议头
//!   （magic / version / type / length），大端网络字节序
//! - 远端玩家插值：`lerp_state` 纯函数 + `RemotePlayer` 时间戳插值平滑
//! - `NetworkMessage` 枚举：Join / Leave / Position / Action，支持序列化往返
//!
//! 本模块仅使用 `std`，不引入外部依赖；如将来需要新依赖，在文件头部按
//! `// DEP: crate = version` 声明。
//! 尚未接入 main.rs 主循环，整体允许 dead_code 警告。

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
/// 单条 UDP 数据报读取缓冲上限（UDP 载荷上限为 65507，这里取安全余量）
pub const MAX_DATAGRAM: usize = 1400;

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
}

impl MessageType {
    /// 从字节还原消息类型
    pub fn from_byte(b: u8) -> Option<MessageType> {
        Some(match b {
            0x01 => MessageType::Join,
            0x02 => MessageType::Leave,
            0x03 => MessageType::Position,
            0x04 => MessageType::Action,
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

/// 网络消息：Join / Leave / Position / Action
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

// ---------------------------------------------------------------------------
// UDP Server / Client
// ---------------------------------------------------------------------------

/// UDP 服务器：绑定监听地址，跟踪已注册客户端并为 Join 分配玩家 id
pub struct Server {
    socket: UdpSocket,
    next_player_id: u32,
    clients: HashMap<SocketAddr, u32>,
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

    /// 非阻塞接收：无数据时返回 `Ok(None)`，协议错误映射为 `InvalidData`
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

    /// 阻塞接收直到超时；超时返回 `Ok(None)`
    pub fn recv_timeout(&self, timeout: Duration) -> io::Result<Option<(NetworkMessage, SocketAddr)>> {
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

    /// 处理一条 Join：自动注册该地址，回发分配后的 Join 确认
    pub fn handle_join(&mut self, from: SocketAddr, name: String) -> io::Result<NetworkMessage> {
        let player_id = self.register(from);
        let reply = NetworkMessage::Join { player_id, name };
        self.send_to(&reply, from)?;
        Ok(reply)
    }
}

/// UDP 客户端：连接服务器，维护自身 id 与远端玩家插值缓冲
pub struct Client {
    socket: UdpSocket,
    server: SocketAddr,
    player_id: Option<u32>,
    remote_players: HashMap<u32, RemotePlayer>,
    clock_start: Instant,
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

    /// 处理一条消息：Join 确认记录自身 id；Position 更新远端玩家插值缓冲
    pub fn handle_message(&mut self, msg: NetworkMessage) {
        match msg {
            NetworkMessage::Join { player_id, .. } => {
                if player_id != 0 && self.player_id.is_none() {
                    self.player_id = Some(player_id);
                }
            }
            NetworkMessage::Position { player_id, state, .. } => {
                let t = self.now();
                match self.remote_players.get_mut(&player_id) {
                    Some(rp) => rp.update(state, t),
                    None => {
                        self.remote_players
                            .insert(player_id, RemotePlayer::new(player_id, state, t));
                    }
                }
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
        recv: impl Fn() -> io::Result<Option<(NetworkMessage, SocketAddr)>>,
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
        let server = Server::bind("127.0.0.1:0").unwrap();
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
}
