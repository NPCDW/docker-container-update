# docker-container-update

一个用 Rust 写的命令行工具：递归扫描指定目录下的 `docker-compose` 文件，并对每个项目执行 `pull` 与 `up -d`。

## 特性

- **配置目录固定为程序所在目录**：不管在哪个路径调用，配置文件与相对路径都相对可执行文件本身解析。
- **compose 命令在 compose 文件所在目录执行**：`ps` / `pull` / `up` 都先切到 compose 文件所在目录，
  并用绝对路径的 `-f` 指定文件，`.env` 与相对 bind mount 都相对该目录解析。
- **YAML 配置**：所有配置项都可以用环境变量覆盖。
- **递归扫描**：可配置最大递归层级，默认最多 2 层；目录里出现 compose 文件即视为栈根，不再下探其子目录。
- **白名单 / 黑名单**：启用白名单时只扫描命中的目录；启用黑名单时扫描除命中目录以外的其他目录。
- **支持 `*` 与 `**` 通配**：如 `nginx/*`、`stacks/**/prod`。
- **安全更新**：执行 `up -d` 前先确认容器「存在且正在运行」，容器不存在或已停止时跳过 `up`；
  `pull` 没拉到新内容时也跳过 `up`。
- **`--dry-run`**：只打印将要执行的命令，不实际执行。
- **前后置钩子**：`compose.pre_command` 在更新前、`compose.post_command` 在更新后执行自定义 shell 命令。

## 安装

```bash
cargo build --release
# 产物: target/release/docker-container-update
```

把 `docker-container-update` 放到目标机任意目录即可，配置文件和它同级。

## 容器用法

镜像内置 [supercronic](https://github.com/aptible/supercronic)（为容器设计的 crontab 任务运行器），
容器启动即由它作为 PID 1 前台运行，每天凌晨 2:20 执行一次 `docker-container-update`。

```bash
docker run -d --name docker-container-update --restart unless-stopped \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /usr/local/soft:/usr/local/soft \
  -e TZ=Asia/Shanghai \
  npcdw/docker-container-update
```

- 定时任务写在镜像内 `/etc/docker-container-update.crontab`：`20 2 * * *`。
- 任务输出（含 supercronic 的执行记录）直接用 `docker logs -f docker-container-update` 看。
- 容器时区由 `TZ` 决定，「凌晨 2:20」按该时区计算，默认 `Asia/Shanghai`。
- 容器环境变量（`TZ`、`DCU_*` 等）会原样传给定时任务，不会像系统 cron 那样被清掉。
- 想临时手动跑一次：`docker exec docker-container-update docker-container-update --dry-run`。
- 要改时间，挂载自定义 crontab 覆盖即可：`-v "$PWD/crontab:/etc/docker-container-update.crontab:ro"`。
- 改完 crontab 想热加载：`docker kill -s USR2 docker-container-update`。

## 快速开始

下文的 `docker-container-update` 指可执行文件本体。

```bash
# 1. 在可执行文件所在目录生成带注释的默认配置
./docker-container-update --init

# 2. 编辑 docker-container-update.yaml，把 compose.dir 指向你的 docker-compose 目录

# 3. 查看会扫描到哪些文件
./docker-container-update list

# 4. 先演练一遍
./docker-container-update --dry-run

# 5. 正式执行
./docker-container-update
```

## 命令

| 命令 | 说明 |
| --- | --- |
| `update`（可省略） | 扫描并更新容器（默认行为） |
| `list`（别名 `ls`） | 仅列出扫描到的 docker-compose 文件 |
| `config` | 打印生效配置（含环境变量覆盖后的结果） |
| `--init` | 生成带注释的默认配置文件 |

选项：`-c/--config <FILE>`、`-n/--dry-run`、`-v/--verbose`。

## 配置

配置文件名固定为 `docker-container-update.yaml`，位于可执行文件同目录；也可以用 `-c` 指定别处。

每个配置项的正上方就是它的说明注释，完整示例见 [`docker-container-update.yaml`](docker-container-update.yaml)：

```yaml
compose:
  # docker-compose 文件所在目录；相对路径按「程序所在目录」解析
  dir: .
  # 递归扫描的最大层级，0 表示只扫描 dir 本身，默认最多 2 层；
  # 目录里一旦出现 compose 文件就视为栈根，不再扫描它的下一级目录
  max_depth: 2
  # 视为 compose 文件的文件名，按顺序优先匹配
  file_names:
    - docker-compose.yml
    - docker-compose.yaml
    - compose.yml
    - compose.yaml
  # compose 命令，可换成 `docker-compose`；带子命令时一并写上
  command: docker compose
  # 是否执行 pull
  pull: true
  # 是否执行 up -d；仅在容器「存在且正在运行」时才执行，
  # 容器不存在或已停止时跳过 up，只做 pull
  up: true
  # up 时是否附带 --remove-orphans
  remove_orphans: true
  # 全部更新完成后是否执行 docker image prune -f
  prune: false
  # 追加到 up 之后的额外参数
  up_args: []
  # 扫描开始前执行的 shell 命令（用 /bin/sh -c 执行），留空表示不执行
  pre_command: ''
  # 更新完成后执行的 shell 命令，留空表示不执行
  post_command: ''

# 白名单：enable 为 true 时「只扫描」命中的目录
whitelist:
  # 是否启用白名单
  enable: false
  # 目录模式，相对 compose.dir 书写；单段名字表示任意层级的同名目录
  dirs: []

# 黑名单：enable 为 true 时扫描「除命中目录之外」的其他目录
blacklist:
  # 是否启用黑名单
  enable: false
  # 目录模式，相对 compose.dir 书写，写法同白名单
  dirs: []
```

### 黑白名单

- **只扫描白名单**：`whitelist.enable: true` 时，只有命中 `whitelist.dirs` 的目录及其子目录会被扫描。
- **扫描除黑名单外的目录**：`blacklist.enable: true` 时，命中 `blacklist.dirs` 的目录整棵子树都会被跳过。
- 两者都启用时，**白名单优先**。

目录模式写法：

| 写法 | 含义 |
| --- | --- |
| `nginx` | 任意层级下名为 `nginx` 的目录 |
| `nginx/api` | 相对 `dir` 的 `nginx/api` |
| `nginx/*` | `nginx` 下的任意一层子目录 |
| `stacks/**/prod` | `stacks` 与 `prod` 之间可隔任意层 |

以 `.` 开头的目录（如 `.git`）与符号链接目录一律跳过。

### 扫描终止

一个目录里出现 `file_names` 中的任意文件时，该目录就被当作一个**栈根**：

- 这个目录本身会被扫描出来；
- 它的下一级目录**不再扫描**，避免把栈内部用于管理数据的 compose 文件也当成独立项目。

目录里没有任何 compose 文件时，才继续按 `max_depth` 往下扫。

## 环境变量

规则：`DCU_` 前缀 + 配置路径的大写下划线形式。列表用英文逗号分隔。

```bash
DCU_COMPOSE_DIR=/opt/stacks
DCU_COMPOSE_MAX_DEPTH=3
DCU_COMPOSE_PULL=false
DCU_COMPOSE_UP_ARGS='--wait,--quiet-pull'
DCU_COMPOSE_PRE_COMMAND='./backup.sh'
DCU_WHITELIST_ENABLE=true
DCU_WHITELIST_DIRS=nginx/api,nginx/web
```

| 环境变量 | 配置项 |
| --- | --- |
| `DCU_COMPOSE_DIR` | `compose.dir` |
| `DCU_COMPOSE_MAX_DEPTH` | `compose.max_depth` |
| `DCU_COMPOSE_FILE_NAMES` | `compose.file_names` |
| `DCU_COMPOSE_COMMAND` | `compose.command` |
| `DCU_COMPOSE_PULL` | `compose.pull` |
| `DCU_COMPOSE_UP` | `compose.up` |
| `DCU_COMPOSE_REMOVE_ORPHANS` | `compose.remove_orphans` |
| `DCU_COMPOSE_PRUNE` | `compose.prune` |
| `DCU_COMPOSE_UP_ARGS` | `compose.up_args` |
| `DCU_COMPOSE_PRE_COMMAND` | `compose.pre_command` |
| `DCU_COMPOSE_POST_COMMAND` | `compose.post_command` |
| `DCU_WHITELIST_ENABLE` | `whitelist.enable` |
| `DCU_WHITELIST_DIRS` | `whitelist.dirs` |
| `DCU_BLACKLIST_ENABLE` | `blacklist.enable` |
| `DCU_BLACKLIST_DIRS` | `blacklist.dirs` |

布尔值接受 `1/true/yes/on` 与 `0/false/no/off`。

## 行为说明

- 每个命中的目录执行 `docker compose pull` 与 `docker compose up -d`，
  命令的工作目录一律是 compose 文件所在目录（这样 `.env` 才会被读取），
  同时把 `PWD` 环境变量也设为该目录，兼容自身不做该处理的 compose 实现（如 `docker-compose`）。
- 命令统一带 `-f <compose 文件绝对路径>`，文件名不是默认值时同样成立。
- `-v` 与 `--dry-run` 打印的命令都带 `(cwd=<目录>)` 前缀，便于确认工作目录。
- **`pull` 没拉到新内容就跳过 `up`**：对比 `pull` 的输出，若只是 `Image is up to date` /
  各层 `Already exists`（本地镜像已是最新），说明没有任何更新，`up -d` 只会空跑，直接跳过。
  跳过时打印 `跳过 up: pull 未拉取到新内容`，且不算失败。
  `--dry-run` 无法预知 pull 结果，不做此判断，仍照常打印 `pull` 与 `up`。
- **`up -d` 前先检查容器状态**（探测命令与 `pull`/`up` 在同一个工作目录下执行）：
  1. `docker compose ps --all -q` 为空 → 容器不存在，跳过 `up`；`--all` 不能省，
     因为 `ps` 默认只列运行中的容器，少了它已停止的容器会被误判成「不存在」；
  2. `docker compose ps --status=running -q` 为空 → 容器已停止，跳过 `up`；
  3. 两者都非空才执行 `up -d`。

  跳过时打印 `跳过 up: 容器不存在` / `跳过 up: 容器已停止`，且不算失败。
  这样只更新「本来就在跑」的容器，不会把人为停掉的、或从未部署过的容器悄悄拉起来。
- 任意一个项目失败，进程以非 0 退出码结束，并继续处理其余项目。

### 前后置钩子

`compose.pre_command` 与 `compose.post_command` 是给单个项目加的自定义 shell 命令，
适合「更新前备份数据卷」「更新后清理旧镜像」这类 compose 本身管不到的动作。

- **执行方式**：`/bin/sh -c <命令>`，可以用管道、`&&` 等 shell 语法；
  工作目录与 `PWD` 都是该项目的 compose 文件所在目录，与 `pull` / `up` 一致。
- **执行时机**：
  1. `pre_command`（扫描前）→ `pull` → 容器状态检查 → `up -d` → `post_command`（更新完成后）；
  2. `pre_command` 必须成功，`pull` 与 `up` 才会执行 —— 前置命令失败说明依赖没准备好，
     继续拉镜像并重建容器只会得到一个起不来的栈；
  3. `post_command` 无论前面成功还是失败都会执行，保证收尾动作不被跳过。
- **失败处理**：失败与非 0 退出码都算项目失败，命令输出照常打印；
  `pre_command` / `post_command` 自带日志，可以在命令里把输出重定向到文件。
- **`--dry-run`**：只打印命令，包括 `[dry-run]` 前缀与工作目录，不实际执行。

```yaml
compose:
  # 更新前先把数据目录打包备份
  pre_command: 'tar czf /backup/$(basename $PWD)-$(date +%F).tgz data/'
  # 更新后删掉本次替换下来的旧镜像，并打一行日志
  post_command: 'docker image prune -f >/dev/null && echo "$(basename $PWD) 更新完成"'
```

钩子按项目执行，不是整轮执行一次；多个项目会各跑一遍。

## 开发

```bash
cargo build
cargo test
```
