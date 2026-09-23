//! 对单个 compose 项目执行 pull / up。

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::scanner::ComposeProject;

/// 对给定项目执行一次更新；`dry_run` 为真时只打印命令不执行。
///
/// 只要会执行 `up`，就先确认项目的容器「存在且正在运行」：
/// 容器不存在说明是首次部署，容器已停止说明是人为停掉的，
/// 这两种情况都不在本工具的更新职责内，直接跳过 `up` 以免意外拉起。
///
/// `pull` 与 `up` 是绑定关系：`pull` 没拉到任何新镜像（本地已是最新）时，
/// 与其对应的 `up` 也没有意义，一并跳过，避免无谓地重建容器。
pub fn update_project(
    project: &ComposeProject,
    config: &Config,
    dry_run: bool,
    verbose: bool,
) -> Result<()> {
    let mut base = config.compose.command_args();
    // `-f` 是 compose 的全局参数，必须排在子命令之前。
    // 用绝对路径，保证即使命令没有在 compose 文件所在目录执行也指向同一个文件。
    base.push("-f".to_string());
    base.push(project.file_path().display().to_string());

    // pull / up 都失败时仍继续跑另一条命令，最后统一报错。
    let mut first_error: Option<anyhow::Error> = None;

    // pull 本次是否拉到了新镜像。pull 未执行或未拉到内容时为 false。
    let mut pulled_something = false;
    if config.compose.pull {
        let mut args = base.clone();
        args.push("pull".to_string());
        match run_capture(&args, project, dry_run, verbose) {
            Ok(output) => pulled_something = pull_updated(&output),
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
    }

    if config.compose.up {
        // pull 明确「什么都没拉到」时直接跳过 up：镜像没变，up 只会空跑。
        // 只有在 pull 实际执行过、且明确报告无更新时才跳过，
        // 未开启 pull 或 dry-run 场景仍按原逻辑推进。
        if config.compose.pull && !dry_run && !pulled_something {
            println!("  跳过 up: pull 未拉取到新内容");
        } else {
            match ensure_running(project, &base, dry_run)? {
                // 探测用的 ps 一定会执行，不需要在 verbose 下重复打印。
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
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// 判断 `docker compose pull` 的输出是否表示真的拉到了新镜像。
///
/// compose 在镜像已是最新时，各层只会报 `Already exists`（或 `Image is up to date`），
/// 没有任何下载/解压动作；只要出现下载、解压或明确的新镜像提示，就说明有更新。
fn pull_updated(output: &str) -> bool {
    const UPDATED_MARKERS: &[&str] = &[
        "Downloaded newer image",
        "Download complete",
        "Extracting",
        "Pull complete",
    ];
    const UPTODATE_MARKERS: &[&str] = &["Image is up to date"];
    if UPTODATE_MARKERS.iter().any(|m| output.contains(m)) {
        return false;
    }
    UPDATED_MARKERS.iter().any(|m| output.contains(m))
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
/// 先 `ps --all -q` 拿到项目名下的全部容器 ID（含已停止）：
/// 一个都没有就是「不存在」；再 `ps --status running -q` 拿到运行中的容器：
/// 为空就是「已停止」。`--all` 不能省：`docker compose ps` 默认只列运行中的容器，
/// 少了它，已停止的容器会被误判成「不存在」。
fn ensure_running(project: &ComposeProject, base: &[String], dry_run: bool) -> Result<UpDecision> {
    if dry_run {
        let shown = display(&project.dir, base, &["ps", "--all", "-q"]);
        println!("  [dry-run] {shown}  # 检查容器是否存在且正在运行");
        return Ok(UpDecision::Proceed);
    }

    let all = probe_ids(project, base, &["ps", "--all", "-q"])?;
    if all.is_empty() {
        return Ok(UpDecision::Skip("容器不存在".to_string()));
    }
    let running = probe_ids(project, base, &["ps", "--status=running", "-q"])?;
    if running.is_empty() {
        return Ok(UpDecision::Skip("容器已停止".to_string()));
    }
    Ok(UpDecision::Proceed)
}

/// 用 compose 的 `ps` 子命令列出容器 ID。
fn probe_ids(project: &ComposeProject, base: &[String], sub: &[&str]) -> Result<Vec<String>> {
    let args = probe_args(base, sub);
    let shown = display(&project.dir, base, sub);
    let (program, rest) = args.split_first().context("命令不能为空")?;
    println!("  $ {shown}");

    let output = Command::new(program)
        .args(rest)
        // 与 pull/up 保持一致：compose 命令一律在 compose 文件所在目录执行。
        .current_dir(&project.dir)
        .env("PWD", &project.dir)
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

/// 拼出带工作目录前缀的展示命令，如 `(cwd=/opt/stacks/nginx) docker compose ps -q`。
///
/// 日志里只看到 `docker compose ps -q` 无法判断在哪个目录执行，
/// 而目录正是决定「compose 项目是哪一个」的关键，所以统一打印出来。
fn display(dir: &Path, base: &[String], sub: &[&str]) -> String {
    format!(
        "(cwd={}) {}",
        dir.display(),
        probe_args(base, sub).join(" ")
    )
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

/// 执行命令并把 stderr/stdout 合并后返回，供调用方解析。
///
/// pull 的进度信息走 stderr，是否拉到新镜像只能从这份输出里判断，
/// 因此需要保留完整文本而不是直接丢弃。
fn run_capture(
    args: &[String],
    project: &ComposeProject,
    dry_run: bool,
    verbose: bool,
) -> Result<String> {
    let (program, rest) = args.split_first().context("命令不能为空")?;
    let shown = format!("(cwd={}) {}", project.dir.display(), args.join(" "));
    if dry_run {
        println!("  [dry-run] {shown}");
        // dry-run 不去假设结果，返回空串让上层按原逻辑推进。
        return Ok(String::new());
    }
    if verbose {
        println!("  $ {shown}");
    }

    let output = Command::new(program)
        .args(rest)
        // compose 会读取当前目录的 .env 等文件，因此切换到 compose 文件所在目录执行。
        .current_dir(&project.dir)
        // 显式设置工作目录，兼容自身未做该处理的 compose 实现（如 `docker-compose`）。
        .env("PWD", &project.dir)
        .output()
        .with_context(|| format!("无法执行 `{shown}`，请确认已安装 {program}"))?;

    // pull 的进度信息都走 stderr，失败时用于报错；二者合并后交给上层解析。
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
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
    Ok(combined)
}

fn run(args: &[String], project: &ComposeProject, dry_run: bool, verbose: bool) -> Result<()> {
    let (program, rest) = args.split_first().context("命令不能为空")?;
    let shown = format!("(cwd={}) {}", project.dir.display(), args.join(" "));
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
        // 显式设置工作目录，兼容自身未做该处理的 compose 实现（如 `docker-compose`）。
        .env("PWD", &project.dir)
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
