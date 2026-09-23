//! 目录扫描：按配置的深度与黑白名单找出所有 compose 项目。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{Config, FilterState};

/// 一个 compose 项目：以 compose 文件所在目录标识。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeProject {
    /// compose 文件所在目录（绝对路径）。
    pub dir: PathBuf,
    /// 目录下的 compose 文件名。
    pub file: String,
}

impl ComposeProject {
    /// 组装 `-f` 参数；使用 compose 自己的默认文件名时无需显式指定。
    pub fn file_args(&self) -> Vec<String> {
        if self.file == "docker-compose.yml" || self.file == "compose.yaml" {
            return Vec::new();
        }
        vec!["-f".to_string(), self.file.clone()]
    }

    /// 展示用路径；`base` 非空时显示相对 `base` 的路径。
    pub fn display(&self, base: &str) -> String {
        let dir = if base.is_empty() {
            self.dir.display().to_string()
        } else {
            match self.dir.strip_prefix(base) {
                Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
                Ok(rel) => rel.display().to_string(),
                Err(_) => self.dir.display().to_string(),
            }
        };
        format!("{dir}/{}", self.file)
    }
}

/// 扫描出全部 compose 项目，按目录排序。
pub fn scan(config: &Config) -> Result<Vec<ComposeProject>> {
    let base = &config.compose.base_dir;
    if !base.is_dir() {
        anyhow::bail!("目录不存在或不是目录: {}", base.display());
    }

    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    walk(
        base,
        base,
        0,
        config,
        &mut dirs,
        &mut seen,
        FilterState::default(),
    )
    .with_context(|| format!("扫描目录失败: {}", base.display()))?;
    dirs.sort();

    Ok(dirs
        .into_iter()
        .filter_map(|dir| detect_compose(dir, config))
        .collect())
}

/// 递归收集候选目录（含 base 自身）。
fn walk(
    base: &Path,
    dir: &Path,
    depth: usize,
    config: &Config,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    state: FilterState,
) -> Result<()> {
    if !seen.insert(dir.to_path_buf()) {
        return Ok(());
    }
    out.push(dir.to_path_buf());

    if depth >= config.compose.max_depth {
        return Ok(());
    }

    let entries =
        std::fs::read_dir(dir).with_context(|| format!("读取目录失败: {}", dir.display()))?;
    let mut children: Vec<(PathBuf, FilterState)> = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // 隐藏目录（含 .git）与软链接目录一律跳过，避免扫到无关或循环的目录。
        if name.starts_with('.') || entry.file_type()?.is_symlink() {
            continue;
        }
        if !config.includes_scan_dir(base, dir, &name, state) {
            continue;
        }
        let child_state = config.filter_state(base, dir, &name, state);
        children.push((path, child_state));
    }
    children.sort_by(|a, b| a.0.cmp(&b.0));

    for (child, child_state) in children {
        walk(base, &child, depth + 1, config, out, seen, child_state)?;
    }
    Ok(())
}

/// 在目录中按 `file_names` 的顺序找出第一个存在的 compose 文件。
fn detect_compose(dir: PathBuf, config: &Config) -> Option<ComposeProject> {
    config
        .compose
        .file_names
        .iter()
        .find(|file| dir.join(file).is_file())
        .map(|file| ComposeProject {
            dir,
            file: file.clone(),
        })
}
