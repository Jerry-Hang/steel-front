use std::path::PathBuf;
fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let rtspv = src.join("pt_ref3.spv");
    if rtspv.exists() { let _ = std::fs::copy(&rtspv, out.join("rt.spv")); }
    println!("cargo:rerun-if-changed={}", rtspv.display());
}
