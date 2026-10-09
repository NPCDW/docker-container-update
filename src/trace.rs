//! 命令回显：在调用外部命令之前，按 shell 提示符的风格打印它。
//!
//! 输出形如 `+ root@host:/opt/stacks/nginx$ docker compose -f a.yml pull`，
//! 便于在一堆日志里一眼找出「到底执行了哪些命令、在哪个目录执行」。

use std::path::Path;

/// 打印一条即将执行的命令。
///
/// 目录取绝对路径：日志常被贴到别处排查，只有绝对路径才能确定命令跑在哪。
pub fn print(dir: &Path, args: &[String]) {
    println!(
        "+ {}@{}:{}$ {}",
        username(),
        hostname(),
        dir.display(),
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

    #[test]
    fn env_var_of_skips_blank() {
        assert_eq!(env_var_of("  "), None);
        assert_eq!(env_var_of("value"), Some("value".to_string()));
    }
}
