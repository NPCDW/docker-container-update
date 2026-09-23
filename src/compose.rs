//! 对单个 compose 项目执行 pull / up。

use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::scanner::ComposeProject;

/// 对给定项目执行一次更新；`dry_run` 为真时只打印命令不执行。
pub fn update_project(
    project: &ComposeProject,
    config: &Config,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let mut base = config.compose.command_args();
    // `-f` 是 compose 的全局参数，必须排在子命令之前。
    base.extend(project.file_args());

    if config.compose.pull {
        let mut args = base.clone();
        args.push("pull".to_string());
        run(&args, project, dry_run, verbose)?;
    }

    if config.compose.up {
        let mut args = base;
        args.push("up".to_string());
        args.push("-d".to_string());
        if config.compose.remove_orphans {
            args.push("--remove-orphans".to_string());
        }
        args.extend(config.compose.up_args.iter().cloned());
        run(&args, project, dry_run, verbose)?;
    }
    Ok(())
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
