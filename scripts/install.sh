#!/usr/bin/env sh
set -eu

repo="${IMPACT_REPO:-AncientiCe/impact-rs}"
install_dir="${IMPACT_INSTALL_DIR:-$HOME/.local/bin}"
tmp_dir="$(mktemp -d)"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux) os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *) echo "Unsupported OS: $os" >&2; exit 1 ;;
  esac
  case "$arch" in
    x86_64|amd64) arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
  esac
  printf '%s-%s' "$arch_part" "$os_part"
}

checksum_verify() {
  file="$1"
  checksum_file="$2"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$file")" && sha256sum -c "$(basename "$checksum_file")")
  else
    (cd "$(dirname "$file")" && shasum -a 256 -c "$(basename "$checksum_file")")
  fi
}

target="$(detect_target)"

version_override="${IMPACT_VERSION:-}"
local_archive="${IMPACT_LOCAL_ARCHIVE:-}"

if [ "$version_override" = "local" ]; then
  if [ -z "$local_archive" ]; then
    echo "IMPACT_LOCAL_ARCHIVE is required when IMPACT_VERSION=local" >&2
    exit 1
  fi
  archive="$local_archive"
else
  if [ -n "$version_override" ]; then
    tag="$version_override"
  else
    tag="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  fi
  version="${tag#v}"
  asset="impact-$version-$target.tar.gz"
  archive="$tmp_dir/$asset"
  checksum="$tmp_dir/impact-$target.sha256"
  curl -fL "https://github.com/$repo/releases/download/$tag/$asset" -o "$archive"
  curl -fL "https://github.com/$repo/releases/download/$tag/impact-$target.sha256" -o "$checksum"
  checksum_verify "$archive" "$checksum"
fi

mkdir -p "$install_dir"
tar -xzf "$archive" -C "$tmp_dir"
binary="$(find "$tmp_dir" -type f -name impact | head -n 1)"
if [ -z "$binary" ]; then
  echo "Archive did not contain an impact binary" >&2
  exit 1
fi
cp "$binary" "$install_dir/impact"
chmod +x "$install_dir/impact"

case ":$PATH:" in
  *":$install_dir:"*) ;;
  *)
    echo "Add impact to PATH:"
    echo "  export PATH=\"$install_dir:\$PATH\""
    ;;
esac

echo "impact installed to $install_dir/impact"
echo "Next: impact index <project> && impact query <file>"
echo "Or register the MCP server: claude mcp add impact -- impact mcp"
