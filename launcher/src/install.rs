//! 安装向导：绿色版复制（game/ 文件夹）或 zip 解压（game.zip，系统 tar）

/// 启动器旁的游戏包源：优先 game/ 文件夹，其次 game.zip
pub fn install_sources() -> Vec<String> {
    let dir = crate::config::launcher_dir();
    let mut srcs = Vec::new();
    if std::path::Path::new(&format!("{}\\game\\steel-front.exe", dir)).exists() {
        srcs.push(format!("{}\\game", dir));
    }
    if std::path::Path::new(&format!("{}\\game.zip", dir)).exists() {
        srcs.push(format!("{}\\game.zip", dir));
    }
    srcs
}

/// 复制目录（递归）
pub fn copy_dir_all_public(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all_public(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// 安装到目标路径。返回错误信息。
pub fn install_to(dest: &str) -> Result<(), String> {
    let dest_path = std::path::Path::new(dest);
    if !dest_path.exists() {
        std::fs::create_dir_all(dest_path).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    for src in install_sources() {
        if src.ends_with(".zip") {
            // tar -xf（Windows 10 1803+ 自带，支持 zip）
            let status = std::process::Command::new("tar")
                .args(["-xf", &src, "-C", dest])
                .status()
                .map_err(|e| format!("tar 调用失败: {}", e))?;
            if status.success() {
                return Ok(());
            } else {
                return Err("zip 解压失败（tar 返回错误）".to_string());
            }
        } else {
            // 复制 game/ 文件夹内容到目标
            copy_dir_all_public(std::path::Path::new(&src), dest_path)
                .map_err(|e| format!("复制失败: {}", e))?;
            return Ok(());
        }
    }
    Err("未找到安装源（启动器旁需有 game/ 文件夹或 game.zip）".to_string())
}

/// 检查目标路径是否已可玩
pub fn verify_install(dest: &str) -> bool {
    std::path::Path::new(&format!("{}\\steel-front.exe", dest)).exists()
}