#!/usr/bin/env bash
# Build mdtui and install it, together with its mermaid-cli prerequisite.
#
#   ./install.sh                 # installs into ~/.local/bin
#   PREFIX=/usr/local ./install.sh
#
# Steps:
#   1. check Rust (cargo) and Node.js >= 22.13 (needed by mermaid-cli 12)
#   2. install / update @mermaid-js/mermaid-cli globally with npm
#   3. write an `mmdc` launcher into $PREFIX/bin that pins the Node it was installed with,
#      so diagrams render even in shells where nvm isn't loaded
#   4. smoke-test mmdc (handles Ubuntu's Chromium sandbox restriction)
#   5. cargo build --release and install the mdtui binary
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
cd "$(dirname "$0")"

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- 1. prerequisites -----------------------------------------------------------------
say "checking prerequisites"
have cargo || die "cargo not found — install Rust from https://rustup.rs"

# load nvm if present so we pick up a modern Node
if ! have node || [ "$(node -e 'process.stdout.write(String(+process.versions.node.split(".")[0] >= 22))' 2>/dev/null)" != "true" ]; then
  export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
  # shellcheck disable=SC1091
  [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" && nvm use --silent 22 >/dev/null 2>&1 || true
fi
have node && have npm || die "Node.js/npm not found — install Node >= 22.13 (e.g. 'nvm install 22')"
node -e '
  const [a, b] = process.versions.node.split(".").map(Number);
  if (a < 22 || (a === 22 && b < 13)) process.exit(1);
' || die "Node $(node --version) is too old for mermaid-cli 12 — need >= 22.13 (try 'nvm install 22')"
echo "    cargo $(cargo --version | cut -d' ' -f2), node $(node --version), npm $(npm --version)"

# --- 2. mermaid-cli ---------------------------------------------------------------------
say "installing @mermaid-js/mermaid-cli globally (npm)"
npm install -g --no-fund --no-audit @mermaid-js/mermaid-cli@latest >/dev/null
NODE_BIN="$(command -v node)"
MMDC_JS="$(readlink -f "$(npm config get prefix)/bin/mmdc")"
[ -f "$MMDC_JS" ] || die "mermaid-cli install failed (no $MMDC_JS)"

# --- 3. nvm-independent launcher -----------------------------------------------------------
install -d "$BIN"
cat > "$BIN/mmdc" <<EOF
#!/bin/sh
# mermaid-cli launcher: pins Node $(node --version) so mmdc works even when nvm isn't loaded.
# Regenerate with $(pwd)/install.sh.
exec "$NODE_BIN" "$MMDC_JS" "\$@"
EOF
chmod 755 "$BIN/mmdc"
echo "    mmdc $("$BIN/mmdc" --version) → $BIN/mmdc"

# --- 4. smoke test -----------------------------------------------------------------------
say "testing mmdc (first run starts headless Chromium, may take a few seconds)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
printf 'flowchart LR\n  A --> B\n' > "$tmp/t.mmd"
if "$BIN/mmdc" -q -i "$tmp/t.mmd" -o "$tmp/t.png" >/dev/null 2>"$tmp/err"; then
  echo "    ok"
else
  printf '{"args":["--no-sandbox"]}' > "$tmp/p.json"
  if grep -q "sandbox" "$tmp/err" && "$BIN/mmdc" -q -p "$tmp/p.json" -i "$tmp/t.mmd" -o "$tmp/t.png" >/dev/null 2>&1; then
    echo "    ok with --no-sandbox (Chromium sandbox is blocked by AppArmor; mdtui retries this automatically)"
  else
    warn "mmdc is installed but could not render: $(head -c 300 "$tmp/err")"
    warn "mdtui will still draw flowchart/sequence/state/pie diagrams as text"
  fi
fi

# --- 5. mdtui ------------------------------------------------------------------------------
say "building mdtui (release)"
cargo build --release --locked -p mdtui
install -m 755 target/release/mdtui "$BIN/mdtui"
say "installed $("$BIN/mdtui" --version) → $BIN/mdtui"

case ":$PATH:" in
  *":$BIN:"*) ;;
  *) warn "$BIN is not on your PATH — add: export PATH=\"$BIN:\$PATH\"" ;;
esac
