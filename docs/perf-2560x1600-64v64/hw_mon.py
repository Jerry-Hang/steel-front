#!/usr/bin/env python3
"""hw_mon.py — 每秒采样 WSL CPU(每核)/内存 + NVIDIA GPU(util/vram/temp/power) + Windows 内存。
用法: python3 /tmp/hw_mon.py [间隔秒=1] [次数=120] > /tmp/hw2560.log"""
import time, sys, subprocess, re

INTERVAL = float(sys.argv[1]) if len(sys.argv) > 1 else 1.0
COUNT = int(sys.argv[2]) if len(sys.argv) > 2 else 120

def read_stat():
    d = {}
    with open('/proc/stat') as f:
        for line in f:
            if line.startswith('cpu'):
                parts = line.split()
                name = parts[0]
                if name == 'cpu':
                    continue
                vals = [int(x) for x in parts[1:]]
                d[name] = (vals[3] + vals[4], sum(vals))
    return d

def cpu_usage(prev, cur):
    usages = []
    for name in cur:
        if name in prev:
            di = cur[name][0] - prev[name][0]
            dt = cur[name][1] - prev[name][1]
            usages.append(100.0 * max(0.0, dt - di) / max(1, dt))
    return usages

def mem_gb():
    d = {}
    with open('/proc/meminfo') as f:
        for line in f:
            if ':' in line:
                k, v = line.split(':', 1)
                d[k] = int(v.split()[0])  # kB
    total = d.get('MemTotal', 0) / 1048576.0
    avail = d.get('MemAvailable', d.get('MemFree', 0)) / 1048576.0
    return total, total - avail

def nv_gpu():
    try:
        out = subprocess.check_output(
            ['/usr/lib/wsl/lib/nvidia-smi',
             '--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw',
             '--format=csv,noheader,nounits'],
            text=True, timeout=4)
        p = [x.strip() for x in out.strip().split(',')]
        return p[0], p[1], p[2], p[3], p[4]
    except Exception:
        return None

def win_ram():
    try:
        out = subprocess.check_output(
            ['powershell.exe', '-NoProfile', '-NonInteractive', '-Command',
             "$a=(Get-Counter '\\Memory\\Available MBytes' -MaxSamples 1).CounterSamples[0].CookedValue;"
             "$c=(Get-Counter '\\Memory\\% Committed Bytes In Use' -MaxSamples 1).CounterSamples[0].CookedValue;"
             "Write-Output ('{0:N0} {1:N1}' -f $a, $c)"],
            text=True, timeout=6)
        nums = out.strip().replace('\r', '').split('\n')
        vals = [x for x in nums if x.strip()]
        a, c = vals[-1].split()
        return float(a) / 1024.0, float(c)
    except Exception:
        return None

prev = read_stat()
for i in range(COUNT):
    time.sleep(INTERVAL)
    cur = read_stat()
    usages = cpu_usage(prev, cur)
    prev = cur
    cpu_avg = sum(usages) / len(usages) if usages else 0.0
    cpu_max = max(usages) if usages else 0.0
    mtotal, mused = mem_gb()
    nv = nv_gpu()
    wr = win_ram()
    ts = time.strftime('%H:%M:%S')
    nv_s = 'nv_util=%s%% vram=%s/%sGB temp=%sC power=%sW' % nv if nv else 'nv=NA'
    wr_s = 'win_avail=%.1fGB win_commit=%.1f%%' % wr if wr else 'win_ram=NA'
    print('[%s] cpu_avg=%5.1f%% cpu_max=%5.1f%% wsl_mem=%.2f/%.2fGB %s %s' % (ts, cpu_avg, cpu_max, mused, mtotal, nv_s, wr_s), flush=True)
