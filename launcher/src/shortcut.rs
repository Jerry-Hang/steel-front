//! 桌面快捷方式：PowerShell WScript.Shell COM（系统自带，零依赖）
use std::os::windows::process::CommandExt;

pub fn create_desktop_shortcut(exe: &str, work_dir: &str, name: &str) -> Result<(), String> {
    let desktop = std::env::var("USERPROFILE")
        .map(|p| format!("{}\\Desktop\\{}.lnk", p, name))
        .unwrap_or_else(|_| format!("{}.lnk", name));
    let ps = format!(
        concat!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');",
            "$s.TargetPath='{}';",
            "$s.WorkingDirectory='{}';",
            "$s.Save()",
        ),
        desktop.replace('\\', "\\\\"),
        exe.replace('\\', "\\\\"),
        work_dir.replace('\\', "\\\\")
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("PowerShell 调用失败: {}", e))?;
    if status.success() { Ok(()) } else { Err("快捷方式创建失败".to_string()) }
}