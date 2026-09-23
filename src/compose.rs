//! 对单个 compose 项目执行 pull / up。

use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::scanner::ComposeProject;

/// 对给定项目执行一次更新；`dry_run` 为真时只打印命令不执行。
///
/// 只要会执行 `up`，就先确认项目的容器「存在且正在运行」：
/// 容器不存在说明是首次部署，容器已停止说明是人为停掉的，
/// 这两种情况都不在本工具的更新职责内，直接跳过 `up` 以免意外拉起。
pub fn update_project(
    project: &ComposeProject,
    config: &Config,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let mut base = config.compose.command_args();
    // `-f` 是 compose 的全局参数，必须排在子命令之前。
    base.extend(project.file_args());

    // pull / up 都失败时仍继续跑另一条命令，最后统一报错。
    let mut first_error: Option<anyhow::Error> = None;

    if config.compose.pull {
        let mut args = base.clone();
        args.push("pull".to_string());
        if let Err(err) = run(&args, project, dry_run, verbose) {
            first_error.get_or_insert(err);
        }
    }

    if config.compose.up {
        match ensure_running(project, &base, dry_run, verbose)? {
            UpDecision::Proceed => {
                let mut args = base;
                args.push("up".to_string());
                args.push("-d".to_string());
                if config.compose.remove_orphans {
                    args.push("--remove-orphans".to_string());
                }
                args.extend(config.compose.up_args.iter().cloned());
                if let Err(err) = run(&args, project, dry_run, verbose) {
                    first_error.get_or_insert(err);
                }
            }
            UpDecision::Skip(reason) => {
                println!("  跳过 up: {reason}");
            }
        }
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// `up` 前的检查结论。
enum UpDecision {
    /// 容器存在且正在运行，可以更新
    Proceed,
    /// 容器不存在或已停止，跳过更新，附带原因
    Skip(String),
}

/// 判断项目的容器是否存在且正在运行。
///
/// 先 `ps -q` 拿到项目名下的容器 ID：一个都没有就是「不存在」；
/// 再 `ps --status running -q` 拿到运行中的容器：为空就是「已停止」。
/// 只凭 ID 数量比较，不依赖 `docker inspect` 的文本解析。
fn ensure_running(
    project: &ComposeProject,
    base: &[String],
    dry_run: bool,
    verbose: bool,
) -> Result<UpDecision> {
    if dry_run {
        let shown = probe_args(base, &["ps", "-q"]).join(" ");
        println!("  [dry-run] {shown}  # 检查容器是否存在且正在运行");
        return Ok(UpDecision::Proceed);
    }

    let all = probe_ids(project, base, &["ps", "-q"], verbose)?;
    if all.is_empty() {
        return Ok(UpDecision::Skip("容器不存在".to_string()));
    }
    let running = probe_ids(project, base, &["ps", "--status", "running", "-q"], verbose)?;
    if running.is_empty() {
        return Ok(UpDecision::Skip("容器已停止".to_string()));
    }
    Ok(UpDecision::Proceed)
}

/// 用 compose 的 `ps` 子命令列出容器 ID。
fn probe_ids(
    project: &ComposeProject,
    base: &[String],
    sub: &[&str],
    verbose: bool,
) -> Result<Vec<String>> {
    let args = probe_args(base, sub);
    let (program, rest) = args.split_first().context("命令不能为空")?;
    let shown = args.join(" ");
    if verbose {
        println!("  $ {shown}");
    }

    let output = Command::new(program)
        .args(rest)
        .current_dir(&project.dir)
        .output()
        .with_context(|| format!("无法执行 `{shown}`，请确认已安装 {program}"))?;
    if !output.status.success() {
        print_stream(&output.stderr);
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "被信号终止".to_string());
        bail!("`{shown}` 失败，退出码 {code}");
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

/// 在全局参数之后拼上子命令，如 `docker compose -f x.yml ps -q`。
fn probe_args(base: &[String], sub: &[&str]) -> Vec<String> {
    let mut args = base.to_vec();
    args.extend(sub.iter().map(|s| s.to_string()));
    args
}

/// 清理悬空镜像，整个流程结束后只调用一次。
pub fn prune(config: &Config, dry_run: bool, verbose: bool) -> Result<()> {
    let args: Vec<String> = ["docker", "image", "prune", "-f"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let shown = args.join(" ");
    if dry_run {
        println!("[prune] [dry-run] {shown}");
        return Ok(());
    }
    if verbose {
        println!("[prune] $ {shown}");
    }
    let (program, rest) = args.split_first().context("命令不能为空")?;
    let output = Command::new(program)
        .args(rest)
        .output()
        .with_context(|| format!("无法执行 `{shown}`"))?;
    if verbose {
        print_stream(&output.stdout);
    }
    if !output.status.success() {
        print_stream(&output.stderr);
        bail!("`{shown}` 失败");
    }
    let _ = config;
    Ok(())
}

fn run(args: &[String], project: &ComposeProject, dry_run: bool, verbose: bool) -> Result<()> {
    let (program, rest) = args.split_first().context("命令不能为空")?;
    let shown = args.join(" ");
    if dry_run {
        println!("  [dry-run] {shown}");
        return Ok(());
    }
    if verbose {
        println!("  $ {shown}");
    }

    let output = Command::new(program)
        .args(rest)
        // compose 会读取当前目录的 .env 等文件，因此切换到 compose 文件所在目录执行。
        .current_dir(&project.dir)
        .output()
        .with_context(|| format!("无法执行 `{shown}`，请确认已安装 {program}"))?;

    // pull / up 的进度信息都走 stderr，失败时用于报错。
    if verbose || !output.status.success() {
        print_stream(&output.stderr);
    }
    if verbose {
        print_stream(&output.stdout);
    }

    if !output.status.success() {
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "被信号终止".to_string());
        bail!("`{shown}` 失败，退出码 {code}");
    }
    Ok(())
}

fn print_stream(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        println!("    {line}");
    }
}
