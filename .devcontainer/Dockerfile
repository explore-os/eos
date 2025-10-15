FROM mcr.microsoft.com/vscode/devcontainers/rust:bullseye

RUN apt-get update && \
        apt-get install -y --no-install-recommends git ca-certificates curl fish bash sudo && \
        rm -rf /var/lib/apt/lists/*

RUN mkdir -p /var/actors && chown vscode:vscode -R /var/actors

WORKDIR /workspaces
