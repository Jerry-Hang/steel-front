# -*- coding: utf-8 -*-
import io, re
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# 版本常量类型（was u8？——定义处检查——改声明为 u16）
s = s.replace("pub const PROTOCOL_VERSION: u16 = 2;", "pub const PROTOCOL_VERSION: u16 = 2;")
# Client 结构加 refused_reason（找 player_id 字段）
s = s.replace("""pub struct Client {
    /// UDP socket
    socket: UdpSocket,""",
"""pub struct Client {
    /// UDP socket
    socket: UdpSocket,
    /// 服务器拒绝原因（协议版本不匹配等；Some = 停止重试）
    refused_reason: Option<String>,""")
# Client::connect 初始化
s = s.replace("""            has_snapshot: false,""",
"""            has_snapshot: false,
            refused_reason: None,""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('client field ok')
