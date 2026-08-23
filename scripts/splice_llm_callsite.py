# -*- coding: utf-8 -*-
import io
p = 'src/engine/game.rs'
s = io.open(p, encoding='utf-8').read()

old = """                // LLM 指挥官覆盖红营命令（最新有效命令；无/失效 → None 走启发式）
                let llm_ov: Option<Vec<crate::engine::ai_command::CmdOverride>> = self
                    .llm
                    .as_ref()
                    .and_then(|l| l.take_latest())
                    .map(|cmds| {
                        cmds.into_iter()
                            .map(|c| crate::engine::ai_command::CmdOverride {
                                order: match c.order {
                                    crate::llm_cmd::LlmOrder::Assault => {
                                        crate::engine::ai_command::CompanyOrder::Assault
                                    }
                                    crate::llm_cmd::LlmOrder::Hold => {
                                        crate::engine::ai_command::CompanyOrder::Hold
                                    }
                                    crate::llm_cmd::LlmOrder::FlankL => {
                                        crate::engine::ai_command::CompanyOrder::Flank(1)
                                    }
                                    crate::llm_cmd::LlmOrder::FlankR => {
                                        crate::engine::ai_command::CompanyOrder::Flank(-1)
                                    }
                                    crate::llm_cmd::LlmOrder::Regroup => {
                                        crate::engine::ai_command::CompanyOrder::Regroup
                                    }
                                },
                                x: c.x,
                                z: c.z,
                            })
                            .collect()
                    });
                cmd.0.update(&self.npcs, &grid, dt, 0, bc, llm_ov.as_deref());
                cmd.1.update(&self.npcs, &grid, dt, 0, rc, None);
                // 态势推送（红营视角 → LLM 决策红营；蓝营保持启发式）
                if let Some(l) = &self.llm {
                    l.push_situation(&build_llm_situation(&cmd.0));
                }"""

new = """                // LLM 指挥官：红蓝各一独立上下文窗口互搏（无/失效 → 该侧启发式）
                let to_ov = |cmds: Vec<crate::llm_cmd::CompanyCmd>| {
                    cmds.into_iter()
                        .map(|c| crate::engine::ai_command::CmdOverride {
                            order: match c.order {
                                crate::llm_cmd::LlmOrder::Assault => {
                                    crate::engine::ai_command::CompanyOrder::Assault
                                }
                                crate::llm_cmd::LlmOrder::Hold => {
                                    crate::engine::ai_command::CompanyOrder::Hold
                                }
                                crate::llm_cmd::LlmOrder::FlankL => {
                                    crate::engine::ai_command::CompanyOrder::Flank(1)
                                }
                                crate::llm_cmd::LlmOrder::FlankR => {
                                    crate::engine::ai_command::CompanyOrder::Flank(-1)
                                }
                                crate::llm_cmd::LlmOrder::Regroup => {
                                    crate::engine::ai_command::CompanyOrder::Regroup
                                }
                            },
                            x: c.x,
                            z: c.z,
                        })
                        .collect::<Vec<_>>()
                };
                let llm_red: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_red()).map(to_ov);
                let llm_blue: Option<Vec<crate::engine::ai_command::CmdOverride>> =
                    self.llm.as_ref().and_then(|l| l.take_blue()).map(to_ov);
                cmd.0.update(&self.npcs, &grid, dt, 0, bc, llm_red.as_deref());
                cmd.1.update(&self.npcs, &grid, dt, 0, rc, llm_blue.as_deref());
                // 态势推送（红/蓝各独立上下文）
                if let Some(l) = &self.llm {
                    let sr = build_llm_situation(&cmd.0);
                    let sb = build_llm_situation(&cmd.1);
                    l.push_red(&sr, cmd.0.companies.len());
                    l.push_blue(&sb, cmd.1.companies.len());
                }"""

assert old in s
s = s.replace(old, new, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('callsite updated')
