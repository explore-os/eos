FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release -p supervisor
RUN cargo build --release -p eos
RUN cargo build --release -p script-actor

FROM mcr.microsoft.com/vscode/devcontainers/universal AS runtime

RUN apt-get update && \
        apt-get install -y --no-install-recommends git ca-certificates curl fish bash sudo && \
        rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/supervisor /usr/local/bin
COPY --from=builder /app/target/release/eos /usr/local/bin
COPY --from=builder /app/target/release/script-actor /usr/local/bin
CMD ["/usr/local/bin/supervisor"]
