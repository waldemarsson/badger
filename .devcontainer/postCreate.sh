#!/usr/bin/env bash
# .devcontainer/postCreate.sh
set -euo pipefail

echo ""
echo "🔧 Running post-create setup..."
echo ""

# ── Load secrets ──────────────────────────────────────────────────────────────
# Source the bind-mounted .env for this script and wire it into .bashrc so every
# future terminal session picks it up too. No secret values are ever copied.
if [ -f /run/secrets/dev.env ]; then
    set -a
    # shellcheck source=/dev/null
    source /run/secrets/dev.env
    set +a
fi
grep -q 'dev.env' "$HOME/.bashrc" 2>/dev/null \
    || echo 'if [ -f /run/secrets/dev.env ]; then set -a; source /run/secrets/dev.env; set +a; fi' \
        >> "$HOME/.bashrc"

echo ""
echo "🔧 Running post-create setup..."
echo ""

# ── Trust .NET HTTPS development certificate ─────────────────────────────────
dotnet dev-certs https

# ── Prepare Copilot bind mounts ──────────────────────────────────────────────
# ~/.copilot is now a full host bind mount. Avoid recursive chown/chmod here,
# because those metadata changes can propagate back to the host filesystem.
for dir in "$HOME/.copilot" "$HOME/.agents" "$HOME/.agents/skills"; do
    mkdir -p "$dir"
done
for dir in "$HOME/.copilot" "$HOME/.agents/skills"; do
    if [ ! -w "$dir" ]; then
        echo "⚠️  $dir is not writable by $(whoami); fix host ownership or mode bits instead of changing them from inside the container"
    fi
done
echo "✅ Copilot directories ready"

# ── Fix volume ownership ──────────────────────────────────────────────────────
echo "🔑 Fixing volume ownership..."
sudo mkdir -p \
    "$HOME/.nuget" \
    "$HOME/.local/share/MicrosoftCredentialProvider" \
    "$HOME/.local/share/claude" \
    "$HOME/.local/state" \
    "$HOME/.claude" \
    "$HOME/.config/gcloud"
sudo chown -R vscode:vscode \
    "$HOME/.nuget" \
    "$HOME/.local" \
    "$HOME/.claude" \
    "$HOME/.config/gcloud" \
    2>/dev/null || true
echo "   ✅ Done"

# ── Migrate Claude Code to native installer ───────────────────────────────────
# The devcontainer feature installs the npm shim. Running `claude install`
# replaces it with the native binary as of early 2026.
echo "🤖 Installing Claude Code (native)..."
claude install 2>/dev/null || true
echo "   ✅ Done"

# ── Azure Artifacts Credential Provider ──────────────────────────────────────
CREDPROVIDER_DIR="$HOME/.nuget/plugins/netcore/CredentialProvider.Microsoft"
if [ ! -d "$CREDPROVIDER_DIR" ]; then
    echo "📦 Installing Azure Artifacts Credential Provider..."
    curl -fsSL https://aka.ms/install-artifacts-credprovider.sh | bash
    echo "   ✅ Installed to $CREDPROVIDER_DIR"
else
    echo "✅ Azure Artifacts Credential Provider already installed"
fi

# ── Wait for Docker daemon (DinD takes a few seconds to start) ───────────────
echo ""
echo "⏳ Waiting for Docker daemon..."
TRIES=0
MAX_TRIES=30
until docker info >/dev/null 2>&1; do
    TRIES=$((TRIES + 1))
    if [ "$TRIES" -ge "$MAX_TRIES" ]; then
        echo "   ⚠️  Docker daemon did not start within ${MAX_TRIES}s"
        break
    fi
    sleep 1
done
docker info >/dev/null 2>&1 && echo "   ✅ Docker daemon is running"

# ── Verification ─────────────────────────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════"
echo "  Environment Verification"
echo "══════════════════════════════════════════════════════════"
echo "  .NET SDK       : $(dotnet --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Docker         : $(docker --version 2>/dev/null || echo 'NOT RUNNING')"
echo "  Docker Compose : $(docker compose version 2>/dev/null || echo 'NOT FOUND')"
echo "  Node.js        : $(node --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Rust           : $(cargo --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Claude Code    : $(claude --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Copilot CLI    : $(copilot --version 2>/dev/null || echo 'run: copilot')"
echo "  CredProvider   : $([ -d "$CREDPROVIDER_DIR" ] && echo 'installed' || echo 'NOT FOUND')"
echo "  Copilot skills : $(ls "$HOME/.copilot/skills" 2>/dev/null | wc -l) file(s)"
echo "  Copilot agents : $(ls "$HOME/.copilot/agents" 2>/dev/null | wc -l) file(s)"
echo "  GSD files      : $(ls "$HOME/.copilot/get-shit-done" 2>/dev/null | wc -l) file(s)"
echo "══════════════════════════════════════════════════════════"
echo ""
echo "📋 Quick Start"
echo "  Verify DinD:           docker run --rm hello-world"
echo "  Azure Artifacts auth:  dotnet restore --interactive"
echo "  Claude Code:           claude"
echo "  Copilot CLI:           copilot auth login"
echo ""

# ── Rust toolchain (user-local via rustup) ───────────────────────────────────
if ! command -v cargo &>/dev/null && [ ! -f "$HOME/.cargo/bin/cargo" ]; then
    echo "🦀 Installing Rust toolchain via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --no-modify-path
    echo "   ✅ Rust installed: $("$HOME/.cargo/bin/rustc" --version)"
else
    echo "✅ Rust already installed: $(cargo --version 2>/dev/null || "$HOME/.cargo/bin/cargo" --version)"
fi
# Ensure cargo is on PATH for the rest of this script
export PATH="$HOME/.cargo/bin:$PATH"


# Drop the NOPASSWD:ALL rule injected by the devcontainer base image.
# All privileged setup is complete at this point. Removing this rule means
# AI agents running in yolo/auto-approve modes cannot escalate to root.
# To temporarily regain sudo during a terminal session, rebuild the container.
echo "🔒 Hardening: removing NOPASSWD sudo rule..."
sudo rm -f /etc/sudoers.d/vscode
echo "   ✅ sudo disabled for vscode user"
