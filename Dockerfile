FROM rust:latest AS rust-build

RUN mkdir /usr/src/docker-container-update
WORKDIR /usr/src/docker-container-update
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./src ./src
RUN cargo build --release




FROM docker:cli

WORKDIR /docker-container-update
COPY --from=rust-build /usr/src/docker-container-update/target/release/dcu /usr/local/bin/dcu
CMD dcu