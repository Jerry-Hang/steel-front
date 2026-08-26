//! 联机中继（NAT 打洞辅助）：极简文本 UDP——注册主机名→公网地址，查询方拿地址直连。
//! 用法：steel-front-rdv.exe [bind]（默认 0.0.0.0:27016）
//! 协议（单行 UTF-8）：REG <name>\n  /  WHO <name>\n → 回复 ADDR <name> <addr>（或 NONE）
use std::net::{SocketAddr, UdpSocket};

fn main() {
    let bind = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:27016".to_string());
    let sock = UdpSocket::bind(&bind).expect("中继绑定失败");
    println!("steel-front-rdv: 监听 {bind}");
    let mut registry: std::collections::HashMap<String, std::net::SocketAddr> = std::collections::HashMap::new();
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
                    // 源 IP 取自数据报来源（NAT 公网观测）；端口取载荷明示（游戏监听端口）
                    let port: u16 = fields
                        .next()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(from.port());
                    registry.insert(name.to_string(), SocketAddr::new(from.ip(), port));
                    let _ = sock.send_to(b"OK\n", from);
                }
            }
            Some("WHO") => {
                if let Some(name) = fields.next() {
                    if let Some(addr) = registry.get(name) {
                        // 通知目标：有查询者（打洞双方互相发往对方可见地址）
                        let _ = sock.send_to(format!("PUNCH {}\n", from).as_bytes(), *addr);
                        let _ = sock.send_to(format!("ADDR {name} {}\n", addr).as_bytes(), from);
                    } else {
                        let _ = sock.send_to(b"NONE\n", from);
                    }
                }
            }
            _ => {}
        }
    }
    // 常驻循环
    #[allow(unreachable_code)]
    ()
}