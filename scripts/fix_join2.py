# -*- coding: utf-8 -*-
import io, re
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# 1) 客户端 retry_join 的 Join 加 version
s = s.replace("""            let _ = self.send(&NetworkMessage::Join {
                player_id: 0,
                name: name.to_string(),
            });""",
"""            let _ = self.send(&NetworkMessage::Join {
                player_id: 0,
                name: name.to_string(),
                version: PROTOCOL_VERSION,
            });""")
# 2) 客户端 handle_message 的 Join 分支（接受）+ Refuse 分支（拒绝态）
s = s.replace("""            NetworkMessage::Join {""",
"""            NetworkMessage::Refuse { reason } => {
                // 服务器拒绝（协议版本不匹配等）：标记拒绝并停止重试
                log::warn!("net: 服务器拒绝加入: {reason}");
                self.refused_reason = Some(reason);
                self.player_id = None;
            }
            NetworkMessage::Join {""")
# 3) 测试构造点统一补 version: PROTOCOL_VERSION（name: ... into() 后）
s = re.sub(r'(NetworkMessage::Join \{ [^}]*?name: [^,}]+(?:\),?)? )\}', r'\1, version: PROTOCOL_VERSION }', s)
s = re.sub(r'(NetworkMessage::Join \{ [^}]*?player_id: [^,}]+,) ', r'\1 version: PROTOCOL_VERSION,', s)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('patched', 'refused' in s)
