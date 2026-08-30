#![allow(unused)]
fn main() {
    let glslang = "C:\\VulkanSDK\\1.4.357.0\\Bin\\glslangValidator.exe";
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    for name in ["fp32", "fp16", "fp8", "fp4"] {
        let glsl = src.join("shaders").join(format!("{name}.comp"));
        let spv = out.join(format!("{name}.spv"));
        println!("cargo:rerun-if-changed={}", glsl.display());
        let status = std::process::Command::new(&glslang).args([glsl.to_str().unwrap(), "-V", "-o", spv.to_str().unwrap()]).status();
        match status { Ok(s) if s.success() => println!("compile {name}: OK"), other => println!("compile {name}: {:?}", other) }
    }
    let rtspv = src.join("pt_ref3.spv");
    if rtspv.exists() { let _ = std::fs::copy(&rtspv, out.join("rt.spv")); println!("rt.spv copied"); }
}
