
# -*- coding: utf-8 -*-
import json, urllib.request
def probe(extra):
    body = {
        "model": "local",
        "temperature": 0.4,
        "max_tokens": 600,
        "messages": [
            {"role": "system", "content": "You are a modern infantry battalion commander. Output STRICT JSON only with a companies array; each item has order (Assault/Hold/FlankL/FlankR/Regroup) and x,z coordinates in [-270,270]. Number of companies must match the situation. No thinking, no explanation."},
            {"role": "user", "content": json.dumps({"battle": "128v128", "side": "red", "map_half": 270, "enemy": {"x": -110, "z": 90}, "companies": [{"id": 0, "strength": 36, "x": 106, "z": -125, "contact": True, "current": "Assault"}, {"id": 1, "strength": 36, "x": 156, "z": -33, "contact": True, "current": "Assault"}, {"id": 2, "strength": 36, "x": 149, "z": 64, "contact": True, "current": "Assault"}]})}
        ]
    }
    body.update(extra)
    req = urllib.request.Request("http://127.0.0.1:8080/v1/chat/completions", data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    raw = urllib.request.urlopen(req, timeout=120).read().decode("utf-8", errors="replace")
    m = json.loads(raw)["choices"][0]["message"]
    print("content:", repr(m.get("content"))[:300])
    return m.get("content") or ""
try:
    print("== thinking=false ==")
    probe({"chat_template_kwargs": {"thinking": False}})
except Exception as e:
    print("ERR1", e)
try:
    print("== stream=false, no_think ==")
    probe({"no_think": True})
except Exception as e:
    print("ERR2", e)
