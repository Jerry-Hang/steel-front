# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
old = """        if stale {
            // 2026-09-01：GLSL 更新时自动重编（glslangValidator），消除手跑 compile_pt.ps1
            let glslang = r"C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";
            let ok = std::process::Command::new(glslang)
                .args(["-V", glsl_path.to_str().unwrap(), "-o", spv_path.to_str().unwrap()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                println!("cargo:warning=PT GLSL 编译失败（glslangValidator 不可用？），使用旧 SPV");
            }
        }"""
new = """        if stale {
            // 2026-09-01：GLSL 更新时自动重编（glslangValidator 只认 .comp/.vert 等扩展名！
            // 复制为临时 .comp 再编译，消除手跑 compile_pt.ps1）
            let glslang = r"C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";
            let temp_comp = dir.join("pt_panorama.comp");
            let _ = std::fs::copy(&glsl_path, &temp_comp);
            let ok = std::process::Command::new(glslang)
                .args(["-V", temp_comp.to_str().unwrap(), "-o", spv_path.to_str().unwrap()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let _ = std::fs::remove_file(&temp_comp);
            if !ok {
                println!("cargo:warning=PT GLSL 编译失败（glslangValidator 不可用？），使用旧 SPV");
            }
        }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open('build.rs', 'w', encoding='utf-8', newline='\n').write(s)
    print('build comp copy')
else:
    print('miss')
