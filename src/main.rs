//! docker-container-update：递归扫描目录下的 docker-compose 文件，并逐个执行更新。
//!
//! 用法：`docker-container-update [选项] [子命令]`，不带子命令时等价于 `docker-container-update update`。

mod compose;
mod config;
mod env;
mod scanner;

use std::env::current_exe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::scanner::ComposeProject;

#[derive(Debug, Parser)]
#[command(
    name = "docker-container-update",
    version,
    about = "扫描 docker-compose 目录并更新容器",
    long_about = "递归扫描指定目录下的 docker-compose 文件，对每个项目执行 docker compose pull 与 up -d。\n工作目录固定为程序自身所在目录，与在哪个路径调用无关。"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// 配置文件路径，默认取程序所在目录下的 docker-container-update.yaml
    #[arg(short = 'c', long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// 在配置文件位置生成带注释的默认配置后退出
    #[arg(long, global = true)]
    init: bool,

    /// 只打印将执行的命令，不实际执行
    #[arg(short = 'n', long, global = true)]
    dry_run: bool,

    /// 打印每条命令及其完整输出
    #[arg(short = 'v', long, global = true)]
    verbose: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 扫描并更新容器（默认行为）
    Update,
    /// 仅列出扫描到的 docker-compose 文件
    #[command(alias = "ls")]
    List,
    /// 打印生效配置（含环境变量覆盖后的结果）
    Config,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("错误: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    // 工作目录固定为程序所在目录，保证从任意位置调用行为一致。
    let workdir = exe_dir()?;
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| workdir.join(config::DEFAULT_CONFIG_FILE));

    if cli.init {
        config::write_default_config(&config_path)?;
        println!("已生成默认配置文件: {}", config_path.display());
        return Ok(ExitCode::SUCCESS);
    }

    let config = Config::load(&config_path, &workdir)?;

    match cli.command.unwrap_or(Command::Update) {
        Command::Config => {
            print_yaml(&config)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::List => {
            let projects = scanner::scan(&config)?;
            print_projects(&projects, &config, true);
            Ok(ExitCode::SUCCESS)
        }
        Command::Update => {
            let projects = scanner::scan(&config)?;
            if projects.is_empty() {
                println!(
                    "未发现 docker-compose 文件: {}",
                    config.compose.base_dir.display()
                );
                return Ok(ExitCode::SUCCESS);
            }
            print_projects(&projects, &config, false);
            let failed = run_updates(&projects, &config, cli.dry_run, cli.verbose);
            // 清理放在全部项目更新之后，避免每个项目都跑一次。
            if config.compose.prune && !failed {
                if let Err(err) = compose::prune(&config, cli.dry_run, cli.verbose) {
                    eprintln!("清理悬空镜像失败: {err:#}");
                    return Ok(ExitCode::FAILURE);
                }
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

/// 返回可执行文件所在目录。
///
/// 优先用 `/proc/self/exe`：即使程序本身是从 `$PATH` 中调用的软链接，
/// 它也能给出真实路径，从而定位到真正的配置目录。
fn exe_dir() -> Result<PathBuf> {
    if let Ok(real) = std::fs::read_link("/proc/self/exe") {
        if let Some(parent) = real.parent() {
            return Ok(parent.to_path_buf());
        }
    }
    let exe = current_exe().context("无法定位可执行文件路径")?;
    exe.parent()
        .map(Path::to_path_buf)
        .context("无法定位可执行文件所在目录")
}

fn print_projects(projects: &[ComposeProject], config: &Config, relative: bool) {
    let base = if relative {
        config.compose.base_dir.display().to_string()
    } else {
        String::new()
    };
    println!("共发现 {} 个 docker-compose 文件:", projects.len());
    for project in projects {
        println!("  {}", project.display(&base));
    }
}

fn run_updates(projects: &[ComposeProject], config: &Config, dry_run: bool, verbose: bool) -> bool {
    let mut failed = false;
    for (index, project) in projects.iter().enumerate() {
        println!(
            "[{}/{}] 更新 {}",
            index + 1,
            projects.len(),
            project.dir.display()
        );
        if let Err(err) = compose::update_project(project, config, dry_run, verbose) {
            eprintln!("  失败: {err:#}");
            failed = true;
        }
    }
    failed
}

fn print_yaml(config: &Config) -> Result<()> {
    print!("{}", serde_yaml::to_string(config)?);
    println!("compose.base_dir: {}", config.compose.base_dir.display());
    Ok(())
}
