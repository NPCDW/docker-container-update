FROM rust:latest AS rust-build

RUN apt-get update && apt-get install -y musl-tools gcc-aarch64-linux-gnu \
    && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

ARG TARGETARCH
RUN if [ "$TARGETARCH" = "amd64" ]; then \
        RUST_TARGET="x86_64-unknown-linux-musl"; \
    else \
        RUST_TARGET="aarch64-unknown-linux-musl"; \
    fi \
    && mkdir /usr/src/docker-container-update \
    && cp -r /usr/src/docker-container-update /tmp/build-src || true

WORKDIR /usr/src/docker-container-update
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src

RUN RUST_TARGET=$([ "$TARGETARCH" = "amd64" ] && echo "x86_64-unknown-linux-musl" || echo "aarch64-unknown-linux-musl") \
    && cargo build --release --target "$RUST_TARGET" \
    && cp "target/$RUST_TARGET/release/docker-container-update" /docker-container-update


FROM docker:cli

RUN apk add --no-cache -f tzdata supercronic

WORKDIR /docker-container-update
COPY --from=rust-build /docker-container-update /docker-container-update

RUN ln -s /docker-container-update/docker-container-update /usr/local/bin/docker-container-update

RUN printf '20 2 * * * /usr/local/bin/docker-container-update\n' \
        > /etc/docker-container-update.crontab

CMD ["supercronic", "-passthrough-logs", "/etc/docker-container-update.crontab"]