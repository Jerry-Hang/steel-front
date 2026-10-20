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
/// 单个源 IP 的活跃房间数上限（防单 IP 灌满注册表）。
const MAX_ROOMS_PER_IP: usize = 4;
/// 房间名合法字符集与长度（只允许 `[A-Za-z0-9_-]`，1..=32）。
fn valid_room_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

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
                    // 🔴 名字校验（2026-10-20 第 16 批复核第 2 条）：长度/字符集都不设限时，
                    // 一个包就能塞 500 字节房间名并在 ADDR 回包/日志里原样放大。
                    if !valid_room_name(name) {
                        let _ = sock.send_to(b"BAD name\n", from);
                        continue;
                    }
                    // 🔴 单 IP 活跃房间数上限：否则一个源 IP 用 4096 个不同名字
                    // 就能在 TTL(120s) 内把注册表填满、把正常玩家全挡在 FULL 外。
                    let same_ip = registry
                        .values()
                        .filter(|(a, _)| a.ip() == from.ip())
                        .count();
                    if !registry.contains_key(name) && same_ip >= MAX_ROOMS_PER_IP {
                        let _ = sock.send_to(b"BUSY ip\n", from);
                        continue;
                    }
                    // 🔴 **端口校验**（外部复核指出第 4 条）：只接受 1..=65535。
                    // 原来 `REG foo 0` 会存进一个非法的 0 端口 —— 之后的 PUNCH 打不出去，
                    // 而且给攻击者留了「借 WHO 向任意地址反射 PUNCH」的口子。
                    // 🔴 **显式给了非法端口要拒绝，不能静默回退**（2026-10-17，外部复核指出）：
                    // 原来 `REG foo 0` / `REG foo abc` 会被当成「没给端口」而回 `OK`
                    // ⇒ 客户端以为自己注册对了、实际用的是 NAT 观测端口，排障时完全看不出来。
                    let field = fields.next();
                    let port = match field {
                        None => from.port(), // 没给 ⇒ 用 NAT 观测端口
                        Some(tok) => match tok.parse::<u16>() {
                            Ok(p) if p != 0 => p,
                            _ => {
                                let _ = sock.send_to(b"BAD port\n", from);
                                continue;
                            }
                        },
                    };
                    let now = Instant::now();
                    // 先清过期（顺手清掉死房间）
                    registry.retain(|_, (_, seen)| now.duration_since(*seen) < ENTRY_TTL);
                    // 防抢注：未过期且**换了源 IP** 的同名注册一律拒
                    //（否则任何人都能覆写别人的房间名 → 把 PUNCH 反射到任意地址）
                    if let Some((addr, seen)) = registry.get(name) {
                        if now.duration_since(*seen) < ENTRY_TTL && addr.ip() != from.ip() {
                            let _ = sock.send_to(b"BUSY name\n", from);
                            continue;
                        }
                    }
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
                        // 🔴 **回带中继观测到的公网地址**（2026-10-17，外部复核指出）：
                        // 客户端据此校准自己在 NAT 后的**实际映射端口**——这正是 STUN 的基本反馈，
                        // 没有它客户端只能猜。旧客户端只认第一个词 `OK` ⇒ **向后兼容**。
                        let observed = SocketAddr::new(from.ip(), port);
                        log_line(&format!("REG {name} -> {observed}"));
                        let _ = sock.send_to(format!("OK {observed}\n").as_bytes(), from);
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

/// 名字校验的判据（2026-10-20 第 16 批复核第 2 条）：
/// 长度/字符集不设限时，一个包能塞 500 字节房间名并在回包/日志里原样放大。
#[cfg(test)]
mod tests {
    use super::valid_room_name;

    #[test]
    fn room_names_are_bounded_to_safe_identifiers() {
        for ok in ["room1", "a", "abc-def_123", "x".repeat(32).as_str()] {
            assert!(valid_room_name(ok), "合法名字被拒: {ok:?}");
        }
        for bad in [
            "",
            "has space",
            "bad/name",
            "newline\n",
            "控制\u{7}",
            "x".repeat(33).as_str(),
            "中文名字",
        ] {
            assert!(!valid_room_name(bad), "非法名字被放行: {bad:?}");
        }
    }
}
