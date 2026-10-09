//! 命令回显：在调用外部命令之前，按 shell 提示符的风格打印它。
//!
//! 输出形如 `+ root@host:nginx$ docker compose -f a.yml pull`，
//! 便于在一堆日志里一眼找出「到底执行了哪些命令、在哪个目录执行」。

use std::path::Path;

/// 打印一条即将执行的命令。
pub fn print(dir: &Path, args: &[String]) {
    println!(
        "+ {}@{}:{}$ {}",
        username(),
        hostname(),
        dir_name(dir),
        args.join(" ")
    );
}

/// 当前用户：优先 `USER` / `LOGNAME`，都缺失时退回真实 UID（0 显示为 root）。
fn username() -> String {
    if let Some(name) = env_var("USER").or_else(|| env_var("LOGNAME")) {
        return name;
    }
    match real_uid() {
        Some(0) => "root".to_string(),
        Some(uid) => uid.to_string(),
        None => "unknown".to_string(),
    }
}

/// 主机名：`/proc` 里的始终是最新值，取不到再退回 `HOSTNAME` 环境变量。
fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .and_then(|s| env_var_of(s.trim()))
        .or_else(|| env_var("HOSTNAME"))
        .unwrap_or_else(|| "localhost".to_string())
}

/// 从 `/proc/self/status` 的 `Uid:` 行取真实 UID，如 `Uid:\t0\t0\t0\t0`。
fn real_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// 目录名，即路径的最后一段；根目录与空路径的兜底是 `/`。
fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.display().to_string())
}

fn env_var(key: &str) -> Option<String> {
    env_var_of(&std::env::var(key).ok()?)
}

/// 过滤掉空白值：变量存在但是空串时按「不存在」处理。
fn env_var_of(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn dir_name_is_last_segment() {
        assert_eq!(dir_name(&PathBuf::from("/opt/stacks/nginx")), "nginx");
        assert_eq!(dir_name(&PathBuf::from("/")), "/");
    }

    #[test]
    fn env_var_of_skips_blank() {
        assert_eq!(env_var_of("  "), None);
        assert_eq!(env_var_of("value"), Some("value".to_string()));
    }
}
