#!/usr/bin/env bash
# .devcontainer/postCreate.sh
set -euo pipefail

echo ""
echo "🔧 Running post-create setup..."
echo ""

# ── Prepare Copilot bind-mount directories ────────────────────────────────────
for dir in "$HOME/.copilot/skills" "$HOME/.copilot/agents" "$HOME/.agents" "$HOME/.agents/skills"; do
    mkdir -p "$dir"
done
for dir in "$HOME/.copilot/skills" "$HOME/.copilot/agents" "$HOME/.agents/skills"; do
    if [ ! -w "$dir" ]; then
        echo "⚠️  $dir is not writable by $(whoami); fix host ownership instead of changing from inside the container"
    fi
done
echo "✅ Copilot directories ready"

# ── Set yolo aliases for Claude and CoPilot ───────────────────────────────────

grep -q 'yolopilot' "$HOME/.bashrc" 2>/dev/null \
    || echo "alias yolopilot='copilot --yolo --experimental'" >> "$HOME/.bashrc"


grep -q 'yoloclaude' "$HOME/.bashrc" 2>/dev/null \
    || echo "alias yoloclaude='claude --dangerously-skip-permissions'" >> "$HOME/.bashrc"


# ── Fix volume ownership ──────────────────────────────────────────────────────
echo "🔑 Fixing volume ownership..."
sudo mkdir -p \
    "$HOME/.cargo/registry" \
    "$HOME/.npm" \
    "$HOME/.claude" \
    "$HOME/.copilot"
sudo chown -R vscode:vscode \
    "$HOME/.cargo" \
    "$HOME/.npm" \
    "$HOME/.claude" \
    "$HOME/.copilot" \
    2>/dev/null || true
echo "   ✅ Done"

# ── Migrate Claude Code to native installer ───────────────────────────────────
echo "🤖 Installing Claude Code (native)..."
claude install 2>/dev/null || true
echo "   ✅ Done"

# ── Install npm dependencies ──────────────────────────────────────────────────
echo "📦 Installing npm dependencies..."
cd /workspace && npm install
echo "   ✅ Done"

# ── Drop NOPASSWD sudo rule ───────────────────────────────────────────────────
# All privileged setup is complete. Removing this means AI agents in auto-approve
# mode cannot escalate to root.
echo "🔒 Hardening: removing NOPASSWD sudo rule..."
sudo rm -f /etc/sudoers.d/vscode
echo "   ✅ sudo disabled for vscode user"

# ── Verification ─────────────────────────────────────────────────────────────
echo ""
echo "══════════════════════════════════════════════════════════"
echo "  Environment Verification"
echo "══════════════════════════════════════════════════════════"
echo "  Rust           : $(rustc --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Cargo          : $(cargo --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Node.js        : $(node --version 2>/dev/null || echo 'NOT FOUND')"
echo "  npm            : $(npm --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Claude Code    : $(claude --version 2>/dev/null || echo 'NOT FOUND')"
echo "  Copilot CLI    : $(copilot --version 2>/dev/null || echo 'run: copilot')"
echo "  Copilot skills : $(ls "$HOME/.copilot/skills" 2>/dev/null | wc -l) file(s)"
echo "  Copilot agents : $(ls "$HOME/.copilot/agents" 2>/dev/null | wc -l) file(s)"
echo "══════════════════════════════════════════════════════════"
echo ""
echo "📋 Quick Start"
echo "  Dev server:    npm run dev"
echo "  Tauri dev:     npm run tauri dev"
echo "  Check types:   npm run check"
echo "  Claude Code:   claude"
echo "  Copilot CLI:   copilot auth login"
echo ""
