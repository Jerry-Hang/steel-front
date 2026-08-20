//! 自动更新：GitHub releases API 检查 + curl 下载 + 解压替换
use std::os::windows::process::CommandExt;
use std::ffi::c_void;

const REPO: &str = "Jerry-Hang/steel-front";

/// 检查最新版本。返回 (tag, zip_url)。错误返回 Err。
pub fn check_latest() -> Result<(String, Option<String>), String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let tmp = std::env::temp_dir().join("sf_release.json");
    let cmd = format!(
        "curl.exe -sL -o \"{}\" -H \"User-Agent: steel-front-launcher\" \"{}\"",
        tmp.display(),
        url
    );
    let status = std::process::Command::new("cmd")
        .args(["/C", &cmd])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("curl 调用失败: {}", e))?;
    if !status.success() {
        return Err("curl 下载失败".to_string());
    }
    let text = std::fs::read_to_string(&tmp).map_err(|_| "无法读取更新信息".to_string())?;
    // 解析 tag_name 与 assets 下载链接
    let mut tag = String::new();
    let mut zip_url: Option<String> = None;
    if let Some(i) = text.find("\"tag_name\":\"") {
        let rest = &text[i + 13..];
        tag = rest.chars().take_while(|c| *c != '"').collect();
    }
    // assets 里的 .zip 下载 URL（browser_download_url）
    for m in text.match_indices("browser_download_url\":\"") {
        let rest = &text[m.0 + 22..];
        let url: String = rest.chars().take_while(|c| *c != '"').collect();
        if url.ends_with(".zip") || url.ends_with(".exe") {
            zip_url = Some(url);
            break;
        }
    }
    if tag.is_empty() {
        return Err("仓库暂无发布版本".to_string());
    }
    Ok((tag, zip_url))
}

/// 下载并替换游戏。zip_url 为 releases 资产 zip；解压到游戏目录（备份旧文件）。
pub fn download_and_apply(zip_url: &str, game_dir: &str) -> Result<(), String> {
    let tmp_zip = std::env::temp_dir().join("sf_update.zip");
    let tmp_extract = std::env::temp_dir().join("sf_update_extract");
    let _ = std::fs::remove_dir_all(&tmp_extract);
    let _ = std::fs::create_dir_all(&tmp_extract);
    let cmd = format!(
        "curl.exe -sL -o \"{}\" -H \"User-Agent: steel-front-launcher\" \"{}\"",
        tmp_zip.display(),
        zip_url
    );
    let status = std::process::Command::new("cmd")
        .args(["/C", &cmd])
        .creation_flags(0x08000000)
        .status()
        .map_err(|e| format!("下载失败: {}", e))?;
    if !status.success() || !tmp_zip.exists() {
        return Err("下载失败".to_string());
    }
    // 解压
    let status = std::process::Command::new("tar")
        .args(["-xf", tmp_zip.to_str().unwrap_or(""), "-C", tmp_extract.to_str().unwrap_or("")])
        .status()
        .map_err(|e| format!("解压失败: {}", e))?;
    if !status.success() {
        return Err("更新包解压失败".to_string());
    }
    // 找到解压出的 steel-front.exe（可能嵌套一层目录）
    let mut exe_src: Option<std::path::PathBuf> = None;
    for entry in std::fs::read_dir(&tmp_extract).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.join("steel-front.exe").exists() {
            exe_src = Some(p);
            break;
        } else if p.ends_with("steel-front.exe") {
            exe_src = Some(p.parent().map(|x| x.to_path_buf()).unwrap_or(p.clone()));
            break;
        }
    }
    let exe_src = exe_src.ok_or("更新包中未找到游戏")?;
    // 备份当前游戏（防回滚）
    let backup = std::path::Path::new(game_dir).join("backup_pre_update");
    let _ = std::fs::remove_dir_all(&backup);
    let _ = crate::install::copy_dir_all_public(std::path::Path::new(game_dir), &backup);
    // 复制新文件（覆盖）
    let _ = crate::install::copy_dir_all_public(&exe_src, std::path::Path::new(game_dir));
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}