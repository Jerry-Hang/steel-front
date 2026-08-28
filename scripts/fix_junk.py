# -*- coding: utf-8 -*-
import io
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
junk = """            NetworkMessage::Refuse { reason } => {
                // 服务器拒绝（协议版本不匹配等）：标记拒绝并停止重试
                log::warn!("net: 服务器拒绝加入: {reason}");
                self.refused_reason = Some(reason);
                self.player_id = None;
            }
"""
n = s.count(junk)
print('junk blocks:', n)
s = s.replace(junk, '')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
