FROM rust:latest AS rust-build

RUN mkdir /usr/src/docker-container-update
WORKDIR /usr/src/docker-container-update
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src
RUN cargo build --release


FROM docker:cli

# supercronic：为容器设计的 crontab 任务运行器（静态编译，Alpine 可直接用）
ARG SUPERCRONIC_VERSION=v0.2.49
ARG SUPERCRONIC_SHA1SUM=e63c11a9726b775a6a11801e81af4f3fb926aa68
RUN apk add --no-cache tzdata \
    && curl -fsSL -o /usr/local/bin/supercronic \
        "https://github.com/aptible/supercronic/releases/download/${SUPERCRONIC_VERSION}/supercronic-linux-amd64" \
    && echo "${SUPERCRONIC_SHA1SUM}  /usr/local/bin/supercronic" | sha1sum -c - \
    && chmod +x /usr/local/bin/supercronic

WORKDIR /docker-container-update
COPY --from=rust-build /usr/src/docker-container-update/target/release/docker-container-update /docker-container-update/docker-container-update
RUN ln -s /docker-container-update/docker-container-update /usr/local/bin/docker-container-update

# 定时任务：每天凌晨 2:20 执行一次更新。
# supercronic 把任务输出直接打到容器 stdout/stderr，不再需要自己重定向到文件。
RUN printf '# 每天凌晨 2:20 执行一次更新\n20 2 * * * /usr/local/bin/docker-container-update\n' \
        > /etc/docker-container-update.crontab

# supercronic 作为 PID 1 前台运行：容器环境变量直传任务、SIGTERM 优雅退出
CMD ["supercronic", "-passthrough-logs", "/etc/docker-container-update.crontab"]
