FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release -p eos --features docker,_setup
RUN mv target/release/eos target/release/setup
RUN cargo build --release -p eos --features docker

FROM mcr.microsoft.com/vscode/devcontainers/base:debian AS runtime

RUN apt-get update && \
        apt-get install -y --no-install-recommends git ca-certificates curl fish bash sudo && \
        rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/eos /usr/local/bin
COPY --from=builder /app/target/release/setup /

RUN mkdir /ext
COPY teleplot-eos.vsix /ext/teleplot-eos.vsix
COPY install-private-vsix.sh /ext/install-private-vsix.sh
RUN chmod +x /ext/install-private-vsix.sh && chown -R vscode:vscode /ext
RUN echo '%sudo ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers

RUN mkdir /explore
COPY --from=builder /app/examples /explore/examples
COPY --from=builder /app/demos /explore/demos
RUN chmod +x /explore/demos/*

COPY mount-eos.sh /
COPY --from=builder /app/docker-entrypoint.sh /
RUN chmod +x /docker-entrypoint.sh

RUN mkdir -p /etc/fish/completions && \
        /setup /etc/fish/completions && \
        rm /setup

RUN mkdir -p /explore/system
RUN chown -R vscode:vscode /explore

WORKDIR /explore

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["/usr/local/bin/eos", "serve", "/explore/systea"]
