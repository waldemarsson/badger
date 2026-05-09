# .devcontainer/Dockerfile
# .NET 10 on Ubuntu 24.04 Noble — GA as of December 2025.
# DinD, Node.js, Claude Code, and Copilot CLI are installed via devcontainer features.
FROM mcr.microsoft.com/devcontainers/dotnet:10.0-noble

# libsecret-1-0 : MSAL token cache persistence for Azure credential provider
# Tauri system deps : required to compile the Tauri v2 desktop app (GTK, WebKit, pkg-config)
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libsecret-1-0 \
        pkg-config \
        libglib2.0-dev \
        libgtk-3-dev \
        libwebkit2gtk-4.1-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*
