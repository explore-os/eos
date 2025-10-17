FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --all

FROM mcr.microsoft.com/vscode/devcontainers/base:debian AS runtime

RUN apt-get update && \
        apt-get install -y --no-install-recommends git ca-certificates curl fish bash sudo && \
        rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/supervisor /usr/local/bin
COPY --from=builder /app/target/release/eos /usr/local/bin
COPY --from=builder /app/target/release/script-actor /usr/local/bin

RUN mkdir /eos
COPY --from=builder /app/examples /eos
COPY --from=builder /app/README.md /eos/README.md
RUN chown -R vscode:vscode /eos

ENTRYPOINT ["/usr/local/bin/supervisor"]
