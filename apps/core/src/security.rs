use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fs2::FileExt;
use rand::Rng;

pub(crate) fn resolve_data_dir(data_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = data_dir {
        return Ok(path);
    }
    let cwd = std::env::current_dir().context("读取当前目录失败")?;
    Ok(cwd.join(".subforge"))
}

pub(crate) fn ensure_data_dir(data_dir: &Path) -> Result<()> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("创建数据目录失败: {}", data_dir.display()))?;
    set_owner_only_directory_permissions(data_dir)?;
    Ok(())
}

pub(crate) fn load_or_create_admin_token_with_override(
    data_dir: &Path,
    configured: Option<&str>,
) -> Result<String> {
    let token_path = data_dir.join("admin_token");
    if let Some(configured_token) = configured {
        let token = configured_token.trim();
        if token.is_empty() {
            return Err(anyhow!("server.admin_token 不能为空字符串"));
        }
        fs::write(&token_path, format!("{token}\n"))
            .with_context(|| format!("写入 admin_token 失败: {}", token_path.display()))?;
        set_owner_only_file_permissions(&token_path)?;
        return Ok(token.to_string());
    }

    if token_path.exists() {
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("读取 admin_token 失败: {}", token_path.display()))?;
        let token = token.trim().to_string();
        if !token.is_empty() {
            set_owner_only_file_permissions(&token_path)?;
            return Ok(token);
        }
    }

    let token = generate_admin_token();
    fs::write(&token_path, format!("{token}\n"))
        .with_context(|| format!("写入 admin_token 失败: {}", token_path.display()))?;
    set_owner_only_file_permissions(&token_path)?;
    Ok(token)
}

pub(crate) fn acquire_single_instance_lock(data_dir: &Path) -> Result<File> {
    let lock_path = data_dir.join("subforge.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("创建锁文件失败: {}", lock_path.display()))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| anyhow!("另一个 Core 实例已在运行"))?;
    Ok(lock_file)
}

fn generate_admin_token() -> String {
    let mut bytes = [0_u8; 32];
    let mut rng = rand::rng();
    rng.fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    matches!(
        normalize_host(host).as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn normalize_host(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub(crate) fn admin_token_config_permission_warning(
    path: &Path,
    has_admin_token: bool,
) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(path).ok()?.permissions().mode() & 0o777;
        build_admin_token_config_permission_warning(path, has_admin_token, mode)
    }

    #[cfg(not(unix))]
    {
        build_admin_token_config_permission_warning(path, has_admin_token, 0o600)
    }
}

fn build_admin_token_config_permission_warning(
    path: &Path,
    has_admin_token: bool,
    mode: u32,
) -> Option<String> {
    if !has_admin_token || (mode & 0o077 == 0) {
        return None;
    }
    Some(format!(
        "WARNING: 配置文件 {} 包含 server.admin_token 且权限为 {:o}，建议收敛为 600（仅当前用户可读写）。",
        path.display(),
        mode
    ))
}

pub(crate) fn set_owner_only_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("设置文件权限失败: {}", path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(path, false)?;
    }

    Ok(())
}

fn set_owner_only_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("设置目录权限失败: {}", path.display()))?;
    }

    #[cfg(windows)]
    {
        apply_windows_owner_only_acl(path, true)?;
    }

    Ok(())
}

#[cfg(windows)]
fn apply_windows_owner_only_acl(path: &Path, is_directory: bool) -> Result<()> {
    let username = std::env::var("USERNAME").context("读取当前用户名失败，无法设置 ACL")?;
    let target = path.to_string_lossy().into_owned();
    let permission = if is_directory { "(OI)(CI)F" } else { "(R,W)" };
    let grant = format!("{username}:{permission}");

    run_icacls(&target, &["/inheritance:r"])?;
    run_icacls(&target, &["/grant:r", &grant])?;

    Ok(())
}

#[cfg(windows)]
fn run_icacls(target: &str, args: &[&str]) -> Result<()> {
    use std::process::Command;

    let output = Command::new("icacls")
        .arg(target)
        .args(args)
        .output()
        .with_context(|| format!("执行 icacls 失败: icacls {target} {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "icacls 执行失败: icacls {target} {}，stdout: {}，stderr: {}",
            args.join(" "),
            stdout.trim(),
            stderr.trim()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::build_admin_token_config_permission_warning;

    #[test]
    fn warns_when_admin_token_exists_and_permissions_are_too_open() {
        let warning = build_admin_token_config_permission_warning(
            Path::new("/tmp/subforge.toml"),
            true,
            0o644,
        );
        assert!(warning.is_some());
    }

    #[test]
    fn no_warning_when_permissions_are_owner_only() {
        let warning = build_admin_token_config_permission_warning(
            Path::new("/tmp/subforge.toml"),
            true,
            0o600,
        );
        assert!(warning.is_none());
    }

    #[test]
    fn no_warning_without_admin_token_even_if_permissions_are_open() {
        let warning = build_admin_token_config_permission_warning(
            Path::new("/tmp/subforge.toml"),
            false,
            0o666,
        );
        assert!(warning.is_none());
    }
}
