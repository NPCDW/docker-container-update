//! 端到端测试：通过 `--init` 生成的配置驱动扫描，覆盖深度与黑白名单。

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// 造一棵目录树，返回 (可执行文件目录, 扫描目录)。
///
/// 树形结构固定为：
/// stacks/{db,nginx/{api,web},foo/bar/baz,.hidden}
fn setup() -> (TempDir, PathBuf, PathBuf) {
    let root = TempDir::new().unwrap();
    let bindir = root.path().join("bin");
    let stacks = root.path().join("stacks");
    for dir in ["db", "nginx/api", "nginx/web", "foo/bar/baz", ".hidden"] {
        fs::create_dir_all(stacks.join(dir)).unwrap();
    }
    for file in [
        "db/docker-compose.yml",
        "nginx/api/docker-compose.yml",
        "nginx/web/compose.yaml",
        "foo/bar/baz/docker-compose.yml",
        ".hidden/docker-compose.yml",
    ] {
        fs::write(stacks.join(file), "services: {}\n").unwrap();
    }
    fs::create_dir_all(&bindir).unwrap();
    (root, bindir, stacks)
}

/// 在已有 compose 文件的目录里再造一个子目录与子 compose 文件，
/// 用于验证「目录已有 compose 文件时不再扫描下一级目录」。
fn nest_inside_stack(stacks: &Path) {
    fs::create_dir_all(stacks.join("db/backup")).unwrap();
    fs::write(
        stacks.join("db/backup/docker-compose.yml"),
        "services: {}\n",
    )
    .unwrap();
}

/// 调用被测二进制；工作目录交给调用方通过 env 控制。
fn run(bindir: &Path, args: &[&str], envs: &[(&str, &str)]) -> (i32, String, String) {
    let exe = env!("CARGO_BIN_EXE_docker-container-update");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .arg("--config")
        .arg(bindir.join("docker-container-update.yaml"))
        .current_dir("/");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// 生成配置文件并写入扫描目录与深度。
fn write_config(bindir: &Path, stacks: &Path, extra: &str) {
    let config = format!(
        "compose:\n  dir: {}\n  max_depth: 2\n{extra}\n",
        stacks.display()
    );
    fs::write(bindir.join("docker-container-update.yaml"), config).unwrap();
}

fn listed(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| {
            line.trim_start()
                .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '.')
        })
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn scans_two_levels_by_default() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, stderr) = run(&bindir, &["list"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        listed(&stdout),
        vec![
            "db/docker-compose.yml",
            "nginx/api/docker-compose.yml",
            "nginx/web/compose.yaml",
        ],
        "第 3 层与隐藏目录都不应出现: {stdout}"
    );
}

#[test]
fn max_depth_env_extends_scan() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, _) = run(&bindir, &["list"], &[("DCU_COMPOSE_MAX_DEPTH", "3")]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("foo/bar/baz/docker-compose.yml"),
        "{stdout}"
    );
}

#[test]
fn whitelist_limits_scan() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, stderr) = run(
        &bindir,
        &["list"],
        &[
            ("DCU_WHITELIST_ENABLE", "true"),
            ("DCU_WHITELIST_DIRS", "nginx"),
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        listed(&stdout),
        vec!["nginx/api/docker-compose.yml", "nginx/web/compose.yaml"],
        "{stdout}"
    );
}

#[test]
fn whitelist_glob_matches_relative_path() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, _) = run(
        &bindir,
        &["list"],
        &[
            ("DCU_WHITELIST_ENABLE", "true"),
            ("DCU_WHITELIST_DIRS", "nginx/*"),
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(
        listed(&stdout),
        vec!["nginx/api/docker-compose.yml", "nginx/web/compose.yaml"],
        "{stdout}"
    );
}

#[test]
fn blacklist_skips_matched_subtree() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, _) = run(
        &bindir,
        &["list"],
        &[
            ("DCU_BLACKLIST_ENABLE", "true"),
            ("DCU_BLACKLIST_DIRS", "nginx,db"),
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(listed(&stdout), Vec::<String>::new(), "{stdout}");
}

#[test]
fn config_subcommand_reflects_env() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let (code, stdout, _) = run(
        &bindir,
        &["config"],
        &[
            ("DCU_COMPOSE_MAX_DEPTH", "5"),
            ("DCU_COMPOSE_PULL", "false"),
        ],
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("max_depth: 5"), "{stdout}");
    assert!(stdout.contains("pull: false"), "{stdout}");
}

#[test]
fn does_not_descend_into_existing_stack() {
    let (_root, bindir, stacks) = setup();
    nest_inside_stack(&stacks);
    write_config(&bindir, &stacks, "");
    let (code, stdout, stderr) = run(&bindir, &["list"], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("db/docker-compose.yml"),
        "栈根项目本身仍应被扫描: {stdout}"
    );
    assert!(
        !stdout.contains("db/backup/docker-compose.yml"),
        "栈内部子目录不应被扫描: {stdout}"
    );
}

#[test]
fn descends_when_directory_has_no_compose_file() {
    let (_root, bindir, stacks) = setup();
    nest_inside_stack(&stacks);
    // 把 db 的 compose 文件换成不认识的扩展名，db 就不再是栈根，
    // 此时应当继续下探到 db/backup。
    fs::remove_file(stacks.join("db/docker-compose.yml")).unwrap();
    write_config(&bindir, &stacks, "");
    let (code, stdout, _) = run(&bindir, &["list"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("db/backup/docker-compose.yml"), "{stdout}");
}

/// 把 compose 命令换成会打印工作目录与参数的假实现，用于观测调用方式。
///
/// 脚本把每个参数打印成 `arg=<参数>`，最后打印 `cwd=$PWD`。
fn fake_compose(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let script = dir.join("fake-compose.sh");
    fs::write(
        &script,
        "#!/bin/sh\nfor arg in \"$@\"; do echo \"arg=$arg\"; done\necho \"cwd=$PWD\"\n",
    )
    .unwrap();
    // 需要可执行位，才能被 Command 直接当作程序调用。
    let mut perms = fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).unwrap();
    script
}

/// 一次 compose 调用：假 compose 收到的参数，以及它所在的目录。
#[derive(Debug, PartialEq, Eq)]
struct Call {
    args: Vec<String>,
    cwd: String,
}

/// 解析假 compose 的输出：每个参数一行，调用结束时打印 `cwd=`。
fn calls(stdout: &str) -> Vec<Call> {
    let mut calls: Vec<Call> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(arg) = line.strip_prefix("arg=") {
            args.push(arg.to_string());
        } else if let Some(cwd) = line.strip_prefix("cwd=") {
            calls.push(Call {
                args: std::mem::take(&mut args),
                cwd: cwd.to_string(),
            });
        }
    }
    calls
}

/// 从 verbose 日志里取出 `$ (cwd=...) <命令>` 行。
fn shown_commands(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|line| line.trim_start())
        .filter(|line| line.starts_with("$ "))
        .map(|line| line.trim_end().to_string())
        .collect()
}

/// `docker compose` 命令必须带绝对 `-f`，且在 compose 文件所在目录执行。
///
/// 否则 `.env` 与相对 bind mount 会相对错误的目录解析——
/// 这是本工具唯一关心的「compose 工作目录」语义。
#[test]
fn compose_runs_in_compose_file_directory() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let fake = fake_compose(&bindir);
    let api_dir = stacks.join("nginx/api");
    let api_file = api_dir.join("docker-compose.yml");

    let (code, stdout, stderr) = run(
        &bindir,
        &["--verbose"],
        &[
            ("DCU_COMPOSE_COMMAND", fake.to_str().unwrap()),
            // 只扫 nginx 下两个项目，断言时不必关心 db
            ("DCU_WHITELIST_ENABLE", "true"),
            ("DCU_WHITELIST_DIRS", "nginx/*"),
        ],
    );
    assert_eq!(code, 0, "{stderr}");

    // 假 compose 的 ps 没有输出，即容器不存在：只执行 pull 与两次 ps 探测。
    // ps 的输出被工具解析成容器 ID，不会回显，它的调用方式只能从 verbose 日志里看。
    let expected = |sub: &[&str]| {
        let mut args = vec!["-f".to_string(), api_file.display().to_string()];
        args.extend(sub.iter().map(|s| s.to_string()));
        args
    };
    let api_calls: Vec<Call> = calls(&stdout)
        .into_iter()
        .filter(|call| call.cwd == api_dir.display().to_string())
        .collect();
    assert_eq!(
        api_calls.iter().map(|c| c.args.clone()).collect::<Vec<_>>(),
        vec![
            expected(&["pull"]),
            expected(&["up", "-d", "--remove-orphans"])
        ],
        "ps 的结果被用于探测，不会回显；pull 与 up 的输出都会回显: {stdout}"
    );

    // verbose 日志覆盖全部三类命令，且都带绝对 -f 与 (cwd=...) 前缀。
    let shown = shown_commands(&stdout).join("\n");
    for sub in ["pull", "-f", "ps -q", "ps --status=running -q"] {
        assert!(shown.contains(sub), "日志应包含 {sub}: {stdout}");
    }
    assert!(!shown.contains("cwd=/\n"), "工作目录不应是根目录: {stdout}");
    let marker = format!("-f {}", api_file.display());
    assert!(
        shown.matches(&marker).count() >= 3,
        "每条命令都应带绝对 -f: {stdout}"
    );
    let cwd_marker = format!("(cwd={})", api_dir.display());
    assert!(
        shown.matches(&cwd_marker).count() >= 3,
        "每条命令都应打印工作目录: {stdout}"
    );
}

/// dry-run 只打印，但打印出的命令要能看出工作目录。
#[test]
fn dry_run_shows_working_directory() {
    let (_root, bindir, stacks) = setup();
    write_config(&bindir, &stacks, "");
    let project_dir = stacks.join("nginx/api");

    let (code, stdout, stderr) = run(
        &bindir,
        &["--dry-run"],
        &[
            ("DCU_WHITELIST_ENABLE", "true"),
            ("DCU_WHITELIST_DIRS", "nginx/*"),
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains(&format!("(cwd={})", project_dir.display())),
        "dry-run 应打印工作目录: {stdout}"
    );
}
