use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(super) fn discover_plugins_from_dirs(dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut discovered = Vec::new();
    let mut dedupe = BTreeSet::new();
    for dir in dirs {
        collect_plugin_dirs(dir, &mut discovered)?;
    }
    discovered.retain(|path| dedupe.insert(path.to_string_lossy().to_string()));
    Ok(discovered)
}

fn collect_plugin_dirs(root: &Path, discovered: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("读取插件路径元信息失败: {}", root.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("plugins.dirs 必须是目录: {}", root.display());
    }
    if root.join("plugin.json").is_file() {
        discovered.push(
            root.canonicalize()
                .with_context(|| format!("解析插件目录失败: {}", root.display()))?,
        );
        return Ok(());
    }

    let entries =
        std::fs::read_dir(root).with_context(|| format!("读取插件目录失败: {}", root.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("读取目录条目失败: {}", root.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_plugin_dirs(&path, discovered)?;
        }
    }
    Ok(())
}
