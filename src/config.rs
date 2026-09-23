//! 配置加载：YAML 文件 + 环境变量覆盖。
//!
//! 「注释与配置项在一起」由 `--init` 生成的模板保证：
//! 模板里每个配置项的正上方就是它的说明注释。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::env::Scope;

/// 默认配置文件名，位于可执行文件同目录。
pub const DEFAULT_CONFIG_FILE: &str = "dcu.yaml";

/// 顶层配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// docker-compose 相关配置
    pub compose: ComposeConfig,
    /// 白名单：启用后只扫描白名单命中的目录
    pub whitelist: ListConfig,
    /// 黑名单：启用后跳过黑名单命中的目录
    pub blacklist: ListConfig,
}

/// `compose` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComposeConfig {
    /// docker-compose 文件所在目录，相对路径按程序所在目录解析
    pub dir: String,
    /// 递归扫描的最大层级，0 表示只扫描 dir 本身
    pub max_depth: usize,
    /// 视为 compose 文件的文件名，按顺序优先匹配
    pub file_names: Vec<String>,
    /// docker compose 命令，可换成 `docker-compose`
    pub command: String,
    /// 是否执行 pull
    pub pull: bool,
    /// 是否执行 up -d
    pub up: bool,
    /// up 时是否附带 --remove-orphans
    pub remove_orphans: bool,
    /// 全部更新完成后是否执行 docker image prune -f
    pub prune: bool,
    /// 追加到 up 之后的额外参数
    pub up_args: Vec<String>,

    /// `dir` 解析后的绝对路径，由 `resolve_paths` 填充，不参与序列化
    #[serde(skip)]
    pub base_dir: PathBuf,
}

/// 黑白名单段。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListConfig {
    /// 是否启用该名单
    pub enable: bool,
    /// 目录模式，相对 `compose.dir` 书写
    pub dirs: Vec<String>,
}

impl Default for ComposeConfig {
    fn default() -> Self {
        ComposeConfig {
            dir: ".".to_string(),
            max_depth: 2,
            file_names: [
                "docker-compose.yml",
                "docker-compose.yaml",
                "compose.yml",
                "compose.yaml",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            command: "docker compose".to_string(),
            pull: true,
            up: true,
            remove_orphans: true,
            prune: false,
            up_args: Vec::new(),
            base_dir: PathBuf::new(),
        }
    }
}

impl ComposeConfig {
    /// 把 command 拆成可执行的参数前缀，如 `docker compose` -> ["docker", "compose"]。
    pub fn command_args(&self) -> Vec<String> {
        self.command
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    }
}

impl Config {
    /// 加载配置：文件（不存在则用默认值）→ 环境变量覆盖 → 解析路径 → 校验。
    pub fn load(path: &Path, workdir: &Path) -> Result<Config> {
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
            serde_yaml::from_str(&content)
                .with_context(|| format!("解析 YAML 失败: {}", path.display()))?
        } else {
            Config::default()
        };

        apply_env(&mut config);
        config.resolve_paths(workdir);
        config.validate()?;
        Ok(config)
    }

    /// 相对 `compose.dir` 解析为绝对路径。
    fn resolve_paths(&mut self, workdir: &Path) {
        let dir = Path::new(&self.compose.dir);
        let resolved = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            workdir.join(dir)
        };
        self.compose.base_dir = normalize(&resolved);
    }

    fn validate(&self) -> Result<()> {
        if self.compose.file_names.is_empty() {
            bail!("compose.file_names 不能为空");
        }
        if self.whitelist.enable && compile(&self.whitelist.dirs).is_empty() {
            bail!("whitelist.enable 为 true 时，whitelist.dirs 不能为空");
        }
        Ok(())
    }

    /// 该子目录是否应进入扫描队列。
    ///
    /// `state` 记录从根到 `parent` 这一路上名单命中的情况：
    /// 白名单采用「一旦命中，子孙放行」的语义，否则 `dirs: [nginx]`
    /// 会把 `nginx/api` 一起挡掉；黑名单则是命中即整棵子树跳过。
    pub fn includes_scan_dir(
        &self,
        base: &Path,
        parent: &Path,
        name: &str,
        state: FilterState,
    ) -> bool {
        let probe = Probe::new(base, parent, name);

        if self.whitelist.enable {
            // 命中白名单的目录，其子孙一律放行。
            return state.whitelist_hit
                || self.whitelist.matched(&probe)
                || self.whitelist.is_prefix_of_match(&probe);
        }
        if self.blacklist.enable && self.blacklist.matched(&probe) {
            return false;
        }
        true
    }

    /// 计算子目录被纳入扫描后的名单命中状态，供其子孙继续使用。
    pub fn filter_state(
        &self,
        base: &Path,
        parent: &Path,
        name: &str,
        state: FilterState,
    ) -> FilterState {
        let probe = Probe::new(base, parent, name);
        FilterState {
            whitelist_hit: state.whitelist_hit || self.whitelist.matched(&probe),
        }
    }
}

/// 从扫描根到候选目录的命中选择状态。
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterState {
    whitelist_hit: bool,
}

/// 建好相对路径的候选目录，避免每次匹配都重算。
///
/// `segments` 是候选目录相对扫描根的各段名字，末段即目录名本身。
struct Probe<'a> {
    segments: Vec<String>,
    name: &'a str,
}

impl<'a> Probe<'a> {
    fn new(base: &Path, parent: &Path, name: &'a str) -> Probe<'a> {
        let rel_parent = parent.strip_prefix(base).unwrap_or(parent);
        let mut segments: Vec<String> = rel_parent
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        segments.push(name.to_string());
        Probe { segments, name }
    }
}

/// 编译后的目录模式。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pattern {
    /// 单段名字，如 `nginx`：任意层级的同名目录都命中
    Name(String),
    /// 相对扫描根的模式，如 `nginx/*`、`stacks/**/prod`
    Glob(Vec<String>),
}

impl Pattern {
    /// 解析一行配置；空串或纯斜杠返回 `None`。
    fn parse(raw: &str) -> Option<Pattern> {
        let cleaned = raw.trim().trim_matches('/');
        if cleaned.is_empty() || cleaned == "." {
            return None;
        }
        let segments: Vec<String> = cleaned
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(|s| s.to_string())
            .collect();
        match segments.len() {
            0 => None,
            // 单段且无通配符时不限定位置，任意层级的同名目录都算命中。
            1 if !segments[0].contains('*') => Some(Pattern::Name(segments[0].clone())),
            _ => Some(Pattern::Glob(segments)),
        }
    }

    /// 候选目录本身是否命中该模式。
    fn matched(&self, probe: &Probe) -> bool {
        match self {
            Pattern::Name(name) => name == probe.name,
            // 多段模式以扫描根为锚点，要求路径被完整消耗。
            Pattern::Glob(segments) => walk_glob(segments, &probe.segments) == GlobResult::Matched,
        }
    }

    /// 候选目录是否只是命中路径的开头，仍需继续下探。
    fn is_prefix_of_match(&self, probe: &Probe) -> bool {
        match self {
            Pattern::Name(_) => false,
            Pattern::Glob(segments) => walk_glob(segments, &probe.segments) == GlobResult::Prefix,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobResult {
    /// 路径已经完全命中模式
    Matched,
    /// 路径是命中路径的前缀，还需要往下走
    Prefix,
    /// 不可能命中
    No,
}

/// 按段匹配路径与模式，区分「已命中」与「只是前缀」。
///
/// 模式与路径都从头开始消耗；路径多出的部分（如模式 `nginx`、路径
/// `nginx/api`）记为前缀，说明还需要继续下探。
fn walk_glob(pattern: &[String], path: &[String]) -> GlobResult {
    let Some(head) = pattern.first() else {
        // 模式已耗尽：路径也空了才算完全命中。
        return if path.is_empty() {
            GlobResult::Matched
        } else {
            GlobResult::Prefix
        };
    };

    if head == "**" {
        let rest = &pattern[1..];
        if rest.is_empty() {
            // 末尾的 `**` 命中当前位置及其下任意层。
            return GlobResult::Matched;
        }
        // 让 `**` 吃掉 0 到多段后继续匹配剩余模式，取最强结果。
        let mut best = GlobResult::No;
        for skip in 0..=path.len() {
            match walk_glob(rest, &path[skip..]) {
                GlobResult::Matched => return GlobResult::Matched,
                GlobResult::Prefix => best = GlobResult::Prefix,
                GlobResult::No => {}
            }
        }
        return best;
    }

    let Some((target, rest_path)) = path.split_first() else {
        // 模式还有段要匹配、路径却已耗尽：当前路径是命中路径的前缀，
        // 它的子目录仍可能命中，所以要继续下探而不是整段剪掉。
        return GlobResult::Prefix;
    };
    if !single_match(head, target) {
        return GlobResult::No;
    }
    walk_glob(&pattern[1..], rest_path)
}

/// 单段通配匹配，`*` 匹配任意长度字符。
fn single_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

impl ListConfig {
    fn matched(&self, probe: &Probe) -> bool {
        compile(&self.dirs).iter().any(|p| p.matched(probe))
    }

    fn is_prefix_of_match(&self, probe: &Probe) -> bool {
        compile(&self.dirs)
            .iter()
            .any(|p| p.is_prefix_of_match(probe))
    }
}

fn compile(raw: &[String]) -> Vec<Pattern> {
    raw.iter().filter_map(|s| Pattern::parse(s)).collect()
}

/// 用 `DCU_` 前缀的环境变量覆盖配置。
pub fn apply_env(config: &mut Config) {
    let root = Scope::new();
    let compose = root.with("compose");

    if let Some(v) = compose.string("dir") {
        config.compose.dir = v;
    }
    if let Some(v) = compose.usize("max_depth") {
        config.compose.max_depth = v;
    }
    if let Some(v) = compose.list("file_names") {
        config.compose.file_names = v;
    }
    if let Some(v) = compose.string("command") {
        config.compose.command = v;
    }
    if let Some(v) = compose.bool("pull") {
        config.compose.pull = v;
    }
    if let Some(v) = compose.bool("up") {
        config.compose.up = v;
    }
    if let Some(v) = compose.bool("remove_orphans") {
        config.compose.remove_orphans = v;
    }
    if let Some(v) = compose.bool("prune") {
        config.compose.prune = v;
    }
    if let Some(v) = compose.list("up_args") {
        config.compose.up_args = v;
    }

    for (key, target) in [
        ("whitelist", &mut config.whitelist),
        ("blacklist", &mut config.blacklist),
    ] {
        let scope = root.with(key);
        if let Some(v) = scope.bool("enable") {
            target.enable = v;
        }
        if let Some(v) = scope.list("dirs") {
            target.dirs = v;
        }
    }
}

/// 去掉 `.` / `..`，纯字符串处理，不访问文件系统，保证路径可比对。
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 生成带注释的默认配置文件；已存在时不覆盖。
pub fn write_default_config(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("配置文件已存在，未覆盖: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }
    std::fs::write(path, default_config_template())
        .with_context(|| format!("写入配置文件失败: {}", path.display()))?;
    Ok(())
}

/// 默认模板：注释就写在对应配置项的上一行。
pub fn default_config_template() -> &'static str {
    r#"# docker-container-update 配置文件
#
# 所有配置项都可以用环境变量覆盖，规则是 DCU_ 前缀 + 配置路径的大写下划线形式：
#   DCU_COMPOSE_DIR=/opt/stacks
#   DCU_COMPOSE_MAX_DEPTH=3
#   DCU_WHITELIST_ENABLE=true
#   DCU_WHITELIST_DIRS=nginx/api,nginx/web
# 列表用英文逗号分隔。

compose:
  # docker-compose 文件所在目录；相对路径按「程序所在目录」解析
  dir: .
  # 递归扫描的最大层级，0 表示只扫描 dir 本身，默认最多 2 层
  max_depth: 2
  # 视为 compose 文件的文件名，按顺序优先匹配
  file_names:
    # compose v1 的默认文件名
    - docker-compose.yml
    # compose v1 的默认文件名
    - docker-compose.yaml
    # compose v2 的默认文件名
    - compose.yml
    # compose v2 的默认文件名
    - compose.yaml
  # compose 命令，可换成 `docker-compose`；带子命令时一并写上（如 `docker compose`）
  command: docker compose
  # 是否执行 pull
  pull: true
  # 是否执行 up -d
  up: true
  # up 时是否附带 --remove-orphans
  remove_orphans: true
  # 全部更新完成后是否执行 docker image prune -f
  prune: false
  # 追加到 up 之后的额外参数，如 ["--wait"]
  up_args: []

# 白名单：enable 为 true 时「只扫描」命中的目录
whitelist:
  # 是否启用白名单
  enable: false
  # 目录模式，相对 compose.dir 书写；单段名字表示任意层级的同名目录，
  # 支持 * 与 **，如 [nginx, apps/*, stacks/**/prod]
  dirs: []

# 黑名单：enable 为 true 时扫描「除命中目录之外」的其他目录
blacklist:
  # 是否启用黑名单
  enable: false
  # 目录模式，相对 compose.dir 书写，写法同白名单
  dirs: []
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Pattern {
        Pattern::parse(s).unwrap()
    }

    fn probe(path: &str) -> Probe<'static> {
        let segs: Vec<String> = path.split('/').map(|s| s.to_string()).collect();
        Probe {
            name: Box::leak(segs.last().unwrap().clone().into_boxed_str()),
            segments: segs,
        }
    }

    #[test]
    fn name_pattern_matches_any_depth() {
        assert!(p("nginx").matched(&probe("nginx")));
        assert!(p("nginx").matched(&probe("a/b/nginx")));
        assert!(!p("nginx").matched(&probe("nginxx")));
    }

    #[test]
    fn glob_matches_and_prefix() {
        let pat = p("nginx/*");
        assert_eq!(
            walk_glob(&pat_segments(&pat), &["nginx".into()]),
            GlobResult::Prefix
        );
        assert_eq!(
            walk_glob(&pat_segments(&pat), &["nginx".into(), "api".into()]),
            GlobResult::Matched
        );
        assert_eq!(
            walk_glob(&pat_segments(&pat), &["db".into()]),
            GlobResult::No
        );
    }

    #[test]
    fn glob_double_star() {
        let pat = p("foo/**/baz");
        let segs = pat_segments(&pat);
        assert_eq!(walk_glob(&segs, &["foo".into()]), GlobResult::Prefix);
        assert_eq!(
            walk_glob(&segs, &["foo".into(), "bar".into()]),
            GlobResult::Prefix
        );
        assert_eq!(
            walk_glob(&segs, &["foo".into(), "bar".into(), "baz".into()]),
            GlobResult::Matched
        );
        assert_eq!(
            walk_glob(&segs, &["foo".into(), "baz".into()]),
            GlobResult::Matched
        );
    }

    fn pat_segments(pat: &Pattern) -> Vec<String> {
        match pat {
            Pattern::Glob(segs) => segs.clone(),
            Pattern::Name(n) => vec![n.clone()],
        }
    }
}
