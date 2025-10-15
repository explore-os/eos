# Devcontainer Template

- **Custom Dockerfile**: Builds from `mcr.microsoft.com/devcontainers/base:ubuntu` and sets up a `vscode` user.
- **Source Bind Mount**: Your host project folder (`${localWorkspaceFolder}`) is bind-mounted into `/workspaces/${localWorkspaceFolderBasename}`.
- **Internal Volume Mount**: A named Docker volume `devcontainer-internal` is mounted at `/opt/shared` inside the container.

## Usage
1. Place the `.devcontainer` folder into the root of your project (or use this as a starting template).
2. Open the project in VS Code and run **Dev Containers: Reopen in Container**.
