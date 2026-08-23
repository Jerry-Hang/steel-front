# -*- coding: utf-8 -*-
import io
p = 'docs/HANDOFF-2026-08-22.md'
s = io.open(p, encoding='utf-8').read()
extra_lines = [
'',
'---',
'',
'## 十一、LLM 战时指挥官（2026-08-23 凌晨，阶段 A）',
'',
'**做了什么**',
'- 新增 src/llm_cmd.rs：零依赖 LLM 指挥官接入（llama.cpp llama-server 的 OpenAI 兼容接口）。',
'  - 迷你 JSON 解析器（手写约 150 行）、最小 HTTP/1.1 POST 客户端（std::net，无第三方依赖）、',
'    独立后台线程 + 双缓冲（游戏主循环零阻塞）、严格 schema 校验（命令枚举/坐标范围/连队数量）。',
'- RV3D_LLM=1（默认 http://127.0.0.1:8080）或自定义 URL；RV3D_LLM_INTERVAL=秒 决策周期（默认 90s）；默认关闭不影响现有玩法。',
'- 集成线：游戏每 0.5s 指挥节拍 → 红营态势 JSON（兵力/重心/接敌/当前命令 + 敌营重心）→ LLM 线程',
'  按周期 POST → 解析+校验 → CmdOverride 覆盖红营各连命令与目标点；校验失败/超时/断连 → 回退内置启发式司令。',
'- scripts/llm_probe.py（喉测）；llama-server 实测 LFM2.5-2.6B Q8 在 RTX 上约 19-62 tok/s。',
'',
'**关键决策**',
'- LLM 只做战役级战略决策（每 30-90s 一次），执行层（ai_command 三三制/编队/掩体）由代码承担——',
'  LLM 思维 + 代码手脚 架构，避免 LLM 直接指挥单兵（延迟/格式/越界风险不可控）。',
'- 响应提取兼容 content / reasoning_content 两种字段（部分模型带 thinking 模板）；',
'  围栏/解释通过截取首个 { 到末个 } 剥离；no_think:true 抑制思考消耗。',
'- 未微调模型格式不稳定（实测：一次采纳成功 + 数次校验拒绝回退）——预期行为，校验器与回退保证',
'  游戏不失控；微调数据集（模仿学习）为阶段 B。',
'',
'**验证结果**',
'- 端到端全通：llmcmd: 命令已采纳 [("Assault", 120, -30)]（LLM 命令被采纳并下发红营）；',
'  非法/截断响应对应 命令校验失败/输出非JSON 警告并自动回退；无 panic。',
'- 注意：llama-server 与游戏共 GPU 时，推理期间游戏 FPS 短暂下降（350→约210）；生产建议 90s 周期或用独立机器跑服务。',
'',
'**后续（阶段 B/C）**',
'- 阶段 B：态势录制器 + 自动回放 → 生成数万条 态势→命令 训练对（内置司令为老师）+ 战术教科书注入；llama.cpp LoRA 微调脚本。',
'- 阶段 C：微调版 GGUF 挂载 + A/B 对比。',
'',
]
extra = '\n'.join(extra_lines)
io.open(p, 'w', encoding='utf-8', newline='').write(s + extra)
print('appended', len(extra))
