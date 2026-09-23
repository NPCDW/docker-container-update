FROM rust:latest AS rust-build

RUN mkdir /usr/src/docker-container-update
WORKDIR /usr/src/docker-container-update
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src
RUN cargo build --release


FROM docker:cli

# busybox 自带 crond，用它做守护进程，不再额外装 cron 包
RUN apk add --no-cache tzdata busybox-extras \
    && mkdir -p /var/spool/cron/crontabs /var/log/docker-container-update

# 定时任务：每天凌晨 2:20 执行一次更新，日志追加到文件
RUN echo '20 2 * * * /usr/local/bin/docker-container-update >> /var/log/docker-container-update/run.log 2>&1' \
        > /var/spool/cron/crontabs/root \
    && chmod 600 /var/spool/cron/crontabs/root

WORKDIR /docker-container-update
COPY --from=rust-build /usr/src/docker-container-update/target/release/docker-container-update /docker-container-update/docker-container-update
RUN ln -s /docker-container-update/docker-container-update /usr/local/bin/docker-container-update

# crond 需要前台运行（-f）且日志打到 stderr（-d 8），才能作为容器的 PID 1
CMD ["crond", "-f", "-d", "8", "-c", "/var/spool/cron/crontabs"]
