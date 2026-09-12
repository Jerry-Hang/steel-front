# -*- coding: utf-8 -*-
"""llm_commander.py - 钢铁前线 RV3D_LLM 战术指挥通道的服务端（OpenAI 兼容）

协议（读自 src/llm_cmd.rs，勿凭猜改）
------------------------------------
* 游戏用 `RV3D_LLM=1` -> url = http://127.0.0.1:8080，路径 /v1/chat/completions，
  裸 HTTP/1.1 POST（`Connection: close`，客户端 read_to_end）。
* 请求体是 OpenAI 风格：{"model","temperature","max_tokens","no_think","messages":[...]}，
  messages = system + 最多 4 轮历史(user/assistant 交替) + 最新态势(user)。
* 服务端必须回 JSON，取 `choices[0].message.content`（`reasoning_content` 会拼接上），
  再从中截取第一个 `{` 到最后一个 `}` 之间的内容，解析成：
      {"companies":[{"order":"Assault|Hold|FlankL|FlankR|Regroup","x":..,"z":..}, ...]}
* **companies 的数量必须与态势里的连队数严格相等**，否则整条命令被丢弃（llm_cmd.rs:329）。
  x/z 必须是有限值且 |x|,|z| <= 270（llm_cmd.rs:345）。
* 红蓝各自独立上下文、交替调用（llm_cmd.rs:510）。

态势 JSON 格式（读自 game.rs::build_llm_situation）
--------------------------------------------------
  {"battle":"128v128","side":"red","map_half":270,"enemy":{"x":-110,"z":90},
   "companies":[{"id":0,"strength":36,"x":106,"z":-125,"contact":true,"current":"Assault"}, ...]}

本文件做什么
------------
把"我（总指挥）定的作战条令"落成每轮可执行的命令，并把每一轮的态势与决策落盘到
data/llm_server.jsonl，供复盘与调参。条令参数放在 data/llm_doctrine.json，**每轮重读**，
所以改条令不需要重启服务端。

用法
----
    python scripts/llm_commander.py [--port 8080] [--log data/llm_server.jsonl]
"""
import argparse
import json
import math
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_DOCTRINE = {
    # 与敌接触的连：推进到距敌 contact_push 米处（即压上去打），强度低于 regroup 则回撤
    "contact_push": 55.0,
    # 未接触的连：向敌轴侧偏 flank_deg 度做钳形，推进 flank_range 米
    "flank_deg": 38.0,
    "flank_range": 170.0,
    # 强度低于此值的连脱离接触、向本方质心靠拢
    "regroup_strength": 12.0,
    # 目标点距地图边缘的最小余量（map_half=270）
    "edge_margin": 12.0,
    # 目标点距该连当前位置不足 min_move 米时视为原地固守（否则命令等于没下）
    "min_move": 18.0,
    # 已经贴上敌人时的"固守"判定
    "hold_range": 45.0,
}


def load_doctrine(path):
    d = dict(DEFAULT_DOCTRINE)
    try:
        with open(path, encoding="utf-8") as f:
            d.update(json.load(f))
    except FileNotFoundError:
        pass
    except Exception as e:
        print("doctrine load failed: %s (using defaults)" % e, flush=True)
    return d


def clamp(v, half, margin):
    lim = half - margin
    return max(-lim, min(lim, v))


def decide(situation, doc):
    """把态势翻成命令。返回 (companies, 说明)。"""
    half = float(situation.get("map_half", 270))
    enemy = situation.get("enemy") or {}
    ex, ez = float(enemy.get("x", 0.0)), float(enemy.get("z", 0.0))
    comps = situation.get("companies") or []
    m = doc["edge_margin"]

    # 本方质心（回撤用）
    if comps:
        cx = sum(float(c["x"]) for c in comps) / len(comps)
        cz = sum(float(c["z"]) for c in comps) / len(comps)
    else:
        cx = cz = 0.0

    out, notes = [], []
    flank_i = 0
    for c in comps:
        px, pz = float(c["x"]), float(c["z"])
        strength = float(c.get("strength", 0))
        contact = bool(c.get("contact", False))

        # 向量：本连 -> 敌
        dx, dz = ex - px, ez - pz
        dist = math.hypot(dx, dz) or 1.0

        if strength < float(doc["regroup_strength"]):
            order, tx, tz = "Regroup", cx, cz
        elif contact and dist <= float(doc["hold_range"]):
            # 已经咬上了，原地固守、别再改目标点，避免把接火中的连队拉走
            order, tx, tz = "Hold", px, pz
        elif contact:
            order = "Assault"
            k = max(0.0, dist - float(doc["contact_push"])) / dist
            tx, tz = px + dx * k, pz + dz * k
        else:
            # 钳形：绕敌轴左右各偏 flank_deg，奇数/偶数连交替，形成两翼
            side = 1.0 if flank_i % 2 == 0 else -1.0
            flank_i += 1
            ang = math.radians(float(doc["flank_deg"]) * side)
            ca, sa = math.cos(ang), math.sin(ang)
            rx = dx * ca - dz * sa
            rz = dx * sa + dz * ca
            rl = math.hypot(rx, rz) or 1.0
            step = min(float(doc["flank_range"]), dist)
            tx, tz = px + rx / rl * step, pz + rz / rl * step
            order = "FlankL" if side > 0 else "FlankR"

        tx, tz = clamp(tx, half, m), clamp(tz, half, m)
        if math.hypot(tx - px, tz - pz) < float(doc["min_move"]) and order != "Hold":
            order, tx, tz = "Hold", px, pz

        out.append({"order": order, "x": round(tx, 1), "z": round(tz, 1)})
        notes.append("%d:%s(%d,%d)s%d%s" % (c.get("id", -1), order, tx, tz, strength,
                                            "C" if contact else "-"))
    return out, notes


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    log_path = None
    doctrine_path = None

    def log_message(self, *a):
        pass  # 静音默认访问日志

    def _send(self, code, payload):
        raw = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(raw)
        self.close_connection = True

    def do_GET(self):
        self._send(200, {"ok": True, "service": "steel-front llm commander"})

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n) if n else b""
        try:
            req = json.loads(raw.decode("utf-8", errors="replace"))
        except Exception as e:
            print("BAD REQUEST: %s" % e, flush=True)
            self._send(400, {"error": "bad json"})
            return

        # 最新一条 user 消息就是态势
        situation_txt = ""
        for msg in reversed(req.get("messages") or []):
            if msg.get("role") == "user":
                situation_txt = msg.get("content") or ""
                break

        doc = load_doctrine(self.doctrine_path)
        try:
            situation = json.loads(situation_txt)
        except Exception as e:
            print("SITUATION PARSE FAIL: %s | %.200s" % (e, situation_txt), flush=True)
            self._send(200, {"choices": [{"message": {"content": "{}"}}]})
            return

        cmds, notes = decide(situation, doc)
        content = json.dumps({"companies": cmds}, ensure_ascii=False)
        side = situation.get("side", "?")
        print("[%s] %s | %s" % (time.strftime("%H:%M:%S"), side, " ".join(notes)), flush=True)

        # 落盘：态势 + 决策 + 条令，供复盘（每行一条 JSON）
        try:
            os.makedirs(os.path.dirname(self.log_path), exist_ok=True)
            with open(self.log_path, "a", encoding="utf-8") as f:
                f.write(json.dumps({
                    "t": time.time(), "side": side,
                    "situation": situation, "commands": cmds, "doctrine": doc,
                }, ensure_ascii=False) + "\n")
        except Exception as e:
            print("log write failed: %s" % e, flush=True)

        self._send(200, {"choices": [{"message": {"role": "assistant", "content": content}}]})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--log", default=os.path.join(REPO, "data", "llm_server.jsonl"))
    ap.add_argument("--doctrine", default=os.path.join(REPO, "data", "llm_doctrine.json"))
    a = ap.parse_args()

    Handler.log_path = a.log
    Handler.doctrine_path = a.doctrine
    srv = ThreadingHTTPServer(("127.0.0.1", a.port), Handler)
    print("commander listening on 127.0.0.1:%d  log=%s  doctrine=%s" % (a.port, a.log, a.doctrine), flush=True)
    print("doctrine now: %s" % json.dumps(load_doctrine(a.doctrine), ensure_ascii=False), flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()


if __name__ == "__main__":
    sys.exit(main())
