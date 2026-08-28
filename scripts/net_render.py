# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = """            let mut npc_visuals: Vec<engine::renderer::NpcVisual> = self
                .game
                .npcs
                .iter()
                .enumerate()
                // 隔墙透视修复：被障碍物完全遮挡的 NPC 不渲染
                .filter(|(i, _)| !self.game.npc_occluded(*i))
                .map(|(_, n)| {"""
new = """            // 客户端联机模式：显示服务器世界（快照实体：位置/朝向/血量来自服务器权威），
            // 阵营色借用本地同 id NPC 的归属（同一确定性地图/波次，id 对齐）
            let net_mode = self.game.net_client.is_some();
            let mut npc_visuals: Vec<engine::renderer::NpcVisual> = if net_mode {
                let client = self.game.net_client.as_ref().unwrap();
                client
                    .entities()
                    .iter()
                    .filter(|(id, e)| (**id < 100_000 && e.hp > 0.0) || **id == 0 || **id >= 100_000)
                    .map(|(id, e)| {
                        let idn = **id;
                        let team = self
                            .game
                            .npcs
                            .iter()
                            .find(|n| n.id as u32 == idn)
                            .map(|n| n.team);
                        let tint = if idn == 0 {
                            [0.95, 0.12, 0.08, 1.0]
                        } else if idn >= 100_000 {
                            [0.08, 0.35, 0.98, 1.0]
                        } else {
                            match team {
                                Some(Team::Blue) => [0.08, 0.35, 0.98, 1.0],
                                _ => [0.95, 0.12, 0.08, 1.0],
                            }
                        };
                        engine::renderer::NpcVisual {
                            pos: [e.state.curr.pos[0], e.state.curr.pos[1], e.state.curr.pos[2]],
                            yaw: e.state.curr.rot,
                            tint,
                            phase: self.anim_clock,
                            moving: true,
                            firing: false,
                        }
                    })
                    .collect()
            } else {
                self
                .game
                .npcs
                .iter()
                .enumerate()
                // 隔墙透视修复：被障碍物完全遮挡的 NPC 不渲染
                .filter(|(i, _)| !self.game.npc_occluded(*i))
                .map(|(_, n)| {"""
assert old in s, 'anchor not found'
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('client world render inserted')
