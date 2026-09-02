# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
# 找到已生成的错误行并替换为 raw 字符串
s = s.replace('let glslang = "C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";', 'let glslang = r"C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";')
# 万一转义不同：
if 'VulkanSDK' in s and 'unknown' in s:
    pass
io.open('build.rs', 'w', encoding='utf-8', newline='\n').write(s)
print('raw fixed via python')
