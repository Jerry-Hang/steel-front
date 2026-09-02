# -*- coding: utf-8 -*-
import io
p = 'build.rs'
s = io.open(p, encoding='utf-8').read()
old = """        if stale {
            println!("cargo:warning=PT GLSL 比 SPV 新，请跑 scripts/compile_pt.ps1 重新编译 pt_panorama.spv");
        }"""
new = """        if stale {
            // 2026-09-01：自动编译 GLSL→SPIR-V（不再手跑 compile_pt.ps1！）
            let out_spv = spv_path.clone();
            let glslang = r"C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";
            let ok = std::process::Command::new(glslang)
                .args([glsl_path.to_str().unwrap(), "-V", "-o", out_spv.to_str().unwrap()]).status()
                .map(|s| s.success()).unwrap_or(false);
            println!("cargo:rerun-if-changed=assets/rt/pt_panorama.spv");
            if !ok {
                println!("cargo:warning=PT GLSL 编译失败（glslangValidator），使用旧 SPV");
            }
        }"""
if old in s:
    s = s.replace(old, new, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('build auto-compile')
else:
    print('miss build')
