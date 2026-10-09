//! 跨平台运行期配置读取：环境变量优先，Android 上再回退到系统属性。
//!
//! 为什么需要它（2026-10-09 实机）：Android 应用进程拿不到自定义环境变量，
//! `setprop` + `__system_property_get` 是不重编就能改运行参数的唯一通道。
//! 桌面（Windows / Linux）行为不变：只看环境变量。

/// 读一个配置项。
///
/// 查找顺序：
/// 1. 环境变量 `<name>`（例如 `RV3D_CPU_PRIME`）
/// 2. Android：系统属性 `debug.sf.<name 去掉 RV3D_ 前缀、转小写>`
///    （例如 `debug.sf.cpu_prime`）
pub fn cfg(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    #[cfg(target_os = "android")]
    {
        let prop = format!(
            "debug.sf.{}",
            name.trim_start_matches("RV3D_").to_lowercase()
        );
        if let Some(v) = android_getprop(&prop) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// 读配置并判断是否等于 `"1"`。
pub fn flag(name: &str) -> bool {
    cfg(name).map(|v| v == "1").unwrap_or(false)
}

/// Android 系统属性读取（libc 的 `__system_property_get`，libc 本来就链着，零新依赖）。
#[cfg(target_os = "android")]
fn android_getprop(name: &str) -> Option<String> {
    use std::ffi::CString;
    extern "C" {
        fn __system_property_get(
            name: *const std::os::raw::c_char,
            value: *mut std::os::raw::c_char,
        ) -> i32;
    }
    let cname = CString::new(name).ok()?;
    let mut buf = [0 as std::os::raw::c_char; 160];
    let n = unsafe { __system_property_get(cname.as_ptr(), buf.as_mut_ptr()) };
    if n <= 0 {
        return None;
    }
    let bytes: Vec<u8> = buf[..n as usize].iter().map(|&b| b as u8).collect();
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_reads_env_var() {
        std::env::set_var("RV3D_TEST_FLAG_UNIT", "1");
        assert!(flag("RV3D_TEST_FLAG_UNIT"));
        std::env::remove_var("RV3D_TEST_FLAG_UNIT");
        assert!(!flag("RV3D_TEST_FLAG_UNIT"));
    }

    #[test]
    fn cfg_ignores_blank_env() {
        std::env::set_var("RV3D_TEST_BLANK_UNIT", "   ");
        assert!(cfg("RV3D_TEST_BLANK_UNIT").is_none());
        std::env::remove_var("RV3D_TEST_BLANK_UNIT");
    }

    #[test]
    fn cfg_reads_env_var_value() {
        std::env::set_var("RV3D_TEST_VALUE_UNIT", "4-6");
        assert_eq!(cfg("RV3D_TEST_VALUE_UNIT").as_deref(), Some("4-6"));
        std::env::remove_var("RV3D_TEST_VALUE_UNIT");
    }
}