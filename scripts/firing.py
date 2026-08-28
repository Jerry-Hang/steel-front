# -*- coding: utf-8 -*-
import io
p = 'src/net.rs'
s = io.open(p, encoding='utf-8').read()
# decode: team 后读 firing
s = s.replace("""                    let team = r.u8()?;
                    npcs.push(NpcSnapshot { id, pos, facing, hp, team });""",
"""                    let team = r.u8()?;
                    let firing = r.u8()?;
                    npcs.push(NpcSnapshot { id, pos, facing, hp, team, firing });""")
# encode: team 后写 firing
s = s.replace("""                    put_f32(&mut p, npc.hp);
                    p.push(npc.team);
                }""",
"""                    put_f32(&mut p, npc.hp);
                    p.push(npc.team);
                    p.push(npc.firing);
                }""")
# RemoteEntity + firing
s = s.replace("""    /// 阵营（0=Red 1=Blue；来自 NpcSnapshot，渲染权威使用）
    pub team: u8,
}""",
"""    /// 阵营（0=Red 1=Blue；来自 NpcSnapshot，渲染权威使用）
    pub team: u8,
    /// 开火指示（最近 0.2s 内开火；渲染枪口焰联动）
    pub firing: bool,
}""")
s = s.replace("""                                state: RemotePlayer::new(player_id, player, t),
                                hp: 100.0,
                                team: 0, // 服务器玩家默认红营（远端蓝营见 NPC 行）""",
"""                                state: RemotePlayer::new(player_id, player, t),
                                hp: 100.0,
                                team: 0, // 服务器玩家默认红营（远端蓝营见 NPC 行）
                                firing: false,""")
s = s.replace("""                                    state: RemotePlayer::new(npc.id, nstate, t),
                                    hp: npc.hp,
                                    team: npc.team,
                                },""",
"""                                    state: RemotePlayer::new(npc.id, nstate, t),
                                    hp: npc.hp,
                                    team: npc.team,
                                    firing: npc.firing == 1,
                                },""")
s = s.replace("""                        Some(e) => {
                            e.state.update(nstate, t);
                            e.hp = npc.hp;
                            e.team = npc.team;
                        }""",
"""                        Some(e) => {
                            e.state.update(nstate, t);
                            e.hp = npc.hp;
                            e.team = npc.team;
                            e.firing = npc.firing == 1;
                        }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('net.rs firing done')
