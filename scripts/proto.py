# -*- coding: utf-8 -*-
import io
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()

# encode ：Join 写 version（找 314 附近）
s = s.replace("""            NetworkMessage::Join { player_id, name } => {
                put_u32(&mut p, player_id);
                put_string(&mut p, name);
                (MessageType::Join, p)""",
"""            NetworkMessage::Join { player_id, name, version } => {
                put_u32(&mut p, player_id);
                put_string(&mut p, name);
                p.extend_from_slice(&version.to_be_bytes());
                (MessageType::Join, p)""")

# decode
s = s.replace("""            MessageType::Join => {
                let player_id = r.u32()?;
                let name = r.string()?;
                NetworkMessage::Join { player_id, name }""",
"""            MessageType::Join => {
                let player_id = r.u32()?;
                let name = r.string()?;
                let version = u16::from_be_bytes([r.u8()?, r.u8()?]);
                NetworkMessage::Join { player_id, name, version }""")

# Refuse encode/decode 追加：加到 Snapshot 分支前
s = s.replace("""            MessageType::Snapshot => {""",
"""            MessageType::Refuse { reason } => {
                put_string(&mut p, reason);
                (MessageType::Refuse, p)
            }
            MessageType::Snapshot => {""")
s = s.replace("""            MessageType::Snapshot => {
                let seq =""",
"""            MessageType::Refuse => {
                let reason = r.string()?;
                NetworkMessage::Refuse { reason }
            }
            MessageType::Snapshot => {
                let seq =""")

io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('encode decode done')
