//! 联机中继（NAT 打洞辅助）：极简文本 UDP——注册主机名→公网地址，查询方拿地址直连。
//! 用法：steel-front-rdv.exe [bind]（默认 0.0.0.0:27016）
//! 协议（单行 UTF-8）：REG <name>\n  /  WHO <name>\n → 回复 ADDR <name> <addr>（或 NONE）
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// 条目寿命：超过这个时间没再 REG 的条目视为下线。
///
/// 🔴 **2026-10-17 加**（外部复核指出：`registry` 无过期清掉与容量上界）。这是**对公网开**的
/// 服务 ⇒ 只 `insert` 不清理，随机 `REG` 能把它撑爆内存；且旧房间主机下线后名字永久残留
/// （`WHO` 会回一个早已失效的地址，打洞必然失败）。
const ENTRY_TTL: Duration = Duration::from_secs(120);
/// 容量上界：常驻服务必须有界（仓库教训 44）。
const MAX_ENTRIES: usize = 4096;

fn log_line(msg: &str) {
    println!("steel-front-rdv: {msg}");
}

fn main() {
    let bind = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:27016".to_string());
    let sock = UdpSocket::bind(&bind).expect("中继绑定失败");
    println!("steel-front-rdv: 监听 {bind}");
    let mut registry: std::collections::HashMap<String, (SocketAddr, Instant)> =
        std::collections::HashMap::new();
    let mut buf = [0u8; 512];
    loop {
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg = String::from_utf8_lossy(&buf[..n]).to_string();
        let mut fields = msg.split_whitespace();
        match fields.next() {
            Some("REG") => {
                if let Some(name) = fields.next() {
                    // 🔴 **端口校验**（外部复核指出第 4 条）：只接受 1..=65535。
                    // 原来 `REG foo 0` 会存进一个非法的 0 端口 —— 之后的 PUNCH 打不出去，
                    // 而且给攻击者留了「借 WHO 向任意地址反射 PUNCH」的口子。
                    let claimed: Option<u16> = fields.next().and_then(|p| p.parse().ok());
                    let port = match claimed {
                        Some(p) if p != 0 => p,
                        _ => from.port(), // 未给/非法 ⇒ 退回数据报的 NAT 观测端口
                    };
                    let now = Instant::now();
                    // 先清过期（顺手清掉死房间）
                    registry.retain(|_, (_, seen)| now.duration_since(*seen) < ENTRY_TTL);
                    let is_new = !registry.contains_key(name);
                    if is_new && registry.len() >= MAX_ENTRIES {
                        // 满了就**拒新键**，不清掉活条目（否则两台主机会互相把对方踢下线）
                        log_line(&format!("REG 拒绝：注册表已满（{MAX_ENTRIES}）"));
                        let _ = sock.send_to(b"FULL\n", from);
                    } else {
                        registry.insert(
                            name.to_string(),
                            (SocketAddr::new(from.ip(), port), now),
                        );
                        let _ = sock.send_to(b"OK\n", from);
                    }
                }
            }
            Some("WHO") => {
                if let Some(name) = fields.next() {
                    // 过期条目视为不存在（并顺手清掉）
                    let live = registry.get(name).and_then(|(addr, seen)| {
                        (Instant::now().duration_since(*seen) < ENTRY_TTL).then_some(*addr)
                    });
                    if let Some(addr) = live {
                        // 通知目标：有查询者（打洞双方互相发往对方可见地址）
                        let _ = sock.send_to(format!("PUNCH {}\n", from).as_bytes(), addr);
                        let _ = sock.send_to(format!("ADDR {name} {}\n", addr).as_bytes(), from);
                    } else {
                        registry.remove(name);
                        let _ = sock.send_to(b"NONE\n", from);
                    }
                }
            }
            _ => {}
        }
    }
}