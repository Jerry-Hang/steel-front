# -*- coding: utf-8 -*-
import struct, zlib, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
# 用 PIL 不可用——读 PNG 用简单方式：直接让 PowerShell/读像素
# 改用 python 读已存 PNG（仅取中心几像素值，用 zlib unfilter 太重）——改用 cv2 不可用
# 简化：用 win32 截图另存？不——用 Blender 已有？——用 PowerShell 的 System.Drawing
