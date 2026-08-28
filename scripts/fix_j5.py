# -*- coding: utf-8 -*-
import io, re
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# 1) version: SESSION_VERSION,name: "p"/"player1".into() , version: PROTOCOL_VERSION 双字段
s = s.replace('version: SESSION_VERSION,name: "player1".into() , version: PROTOCOL_VERSION', 'version: SESSION_VERSION, name: "player1".into()')
# 上面错序——保序：修成 name 在前
s = s.replace('NetworkMessage::Join { player_id: 0, version: SESSION_VERSION, name: "player1".into() }', 'NetworkMessage::Join { player_id: 0, name: "player1".into(), version: SESSION_VERSION }')
# 2) 更坏形态（含前逗号差异）
s = s.replace('NetworkMessage::Join { player_id: 0, version: SESSION_VERSION,name: "player1".into() , version: PROTOCOL_VERSION }', 'NetworkMessage::Join { player_id: 0, name: "player1".into(), version: SESSION_VERSION }')
s = s.replace('NetworkMessage::Join { player_id: 1, version: SESSION_VERSION,name: "p".into() , version: PROTOCOL_VERSION }', 'NetworkMessage::Join { player_id: 1, name: "p".into(), version: SESSION_VERSION }')
# 3) handle_join 2 参
s = s.replace('server.handle_join(from, "alice".to_string()).unwrap()', 'server.handle_join(from, "alice".to_string(), SESSION_VERSION).unwrap()')
s = s.replace('server.handle_join(from, "player1".into()).unwrap()', 'server.handle_join(from, "player1".into(), SESSION_VERSION).unwrap()')
# 4) 1138 缺 version（玩家名测试）
s = s.replace('''        let m = NetworkMessage::Join {
            player_id: 1,
            name: "玩家".repeat(40), // 240 字节（≤ 255），应完整往返
        };''', '''        let m = NetworkMessage::Join {
            player_id: 1,
            name: "玩家".repeat(40), // 240 字节（≤ 255），应完整往返
            version: SESSION_VERSION,
        };''')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('all patched')
