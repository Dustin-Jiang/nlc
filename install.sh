#!/bin/sh
# Install nlc: prebuilt binary from GitHub Releases, cargo from git as fallback.
# Install dir: $NLCS_INSTALL_DIR (default ~/.local/bin)
set -eu

REPO="Dustin-Jiang/nlc"
DEST="${NLCS_INSTALL_DIR:-$HOME/.local/bin}"

need_cmd() { command -v "$1" >/dev/null 2>&1; }

if need_cmd nlc; then
  echo "nlc already installed: $(command -v nlc)"
  exit 0
fi

# Map this machine to a release target; empty when we have no prebuilt asset.
target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Linux)
      case "$arch" in
        x86_64) echo "x86_64-unknown-linux-musl" ;;
        *) return 1 ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) return 1 ;;
      esac
      ;;
    MINGW* | MSYS* | CYGWIN*)
      echo "x86_64-pc-windows-msvc"
      ;;
    *)
      return 1
      ;;
  esac
}

if T=$(target) && need_cmd curl; then
  case "$T" in
    *windows*) pkg="nlc-$T.zip" ;;
    *) pkg="nlc-$T.tar.gz" ;;
  esac
  url="https://github.com/$REPO/releases/latest/download/$pkg"
  tmp=$(mktemp -d)
  echo "downloading $url"
  if curl -fL --retry 3 -o "$tmp/pkg" "$url"; then
    tar xf "$tmp/pkg" -C "$tmp"
    bin=nlc
    case "$T" in *windows*) bin=nlc.exe ;; esac
    mkdir -p "$DEST"
    install -m 0755 "$tmp/$bin" "$DEST/$bin"
    rm -rf "$tmp"
    echo "installed $DEST/$bin"
    case ":$PATH:" in
      *":$DEST:"*) ;;
      *) echo "note: $DEST is not on your PATH, add it to use nlc" ;;
    esac
    exit 0
  fi
  rm -rf "$tmp"
  echo "no prebuilt binary for this platform, falling back to cargo"
fi

if need_cmd cargo; then
  exec cargo install --git "https://github.com/$REPO" nlc
fi

echo "no prebuilt binary for this platform and no cargo found."
echo "build from source instead:"
echo "  git clone https://github.com/$REPO && cd nlc && cargo build --release"
exit 1
