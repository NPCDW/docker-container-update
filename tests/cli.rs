//! 端到端测试：通过 `--init` 生成的配置驱动扫描，覆盖深度与黑白名单。

use std::fs;
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
    let exe = env!("CARGO_BIN_EXE_dcu");
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .arg("--config")
        .arg(bindir.join("dcu.yaml"))
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
    fs::write(bindir.join("dcu.yaml"), config).unwrap();
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
