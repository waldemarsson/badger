# .devcontainer/Dockerfile
# Ubuntu 24.04 Noble — Rust, Node.js, Claude Code, and Copilot CLI via devcontainer features.
FROM mcr.microsoft.com/devcontainers/base:noble

# Tauri v2 system dependencies (GTK, WebKit, pkg-config)
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libglib2.0-dev \
        libgtk-3-dev \
        libwebkit2gtk-4.1-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*
