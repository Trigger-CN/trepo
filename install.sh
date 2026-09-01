#!/usr/bin/env sh
set -eu

repository="Trigger-CN/trepo"
install_dir="${TREPO_INSTALL_DIR:-${HOME}/.local/bin}"
base_url="https://github.com/${repository}/releases/latest/download"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64|Linux-amd64) platform="linux-x86_64" ;;
  Darwin-x86_64) platform="macos-x86_64" ;;
  Darwin-arm64|Darwin-aarch64) platform="macos-aarch64" ;;
  *) echo "trepo does not publish an asset for $(uname -s)/$(uname -m)." >&2; exit 1 ;;
esac

command -v curl >/dev/null 2>&1 || { echo "curl is required." >&2; exit 1; }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl -fL --retry 3 --proto '=https' --tlsv1.2 "${base_url}/SHA256SUMS" -o "${tmp}/SHA256SUMS"
asset="$(awk -v platform="$platform" '{ name=$2; sub(/^\*/, "", name); sub(/^\.\//, "", name); if (name ~ ("^trepo-v[^/]+-" platform "\\.tar\\.gz$")) print name }' "${tmp}/SHA256SUMS")"
[ -n "$asset" ] && [ "$(printf '%s\n' "$asset" | wc -l | tr -d ' ')" -eq 1 ] || { echo "Could not select one ${platform} asset from SHA256SUMS." >&2; exit 1; }
curl -fL --retry 3 --proto '=https' --tlsv1.2 "${base_url}/${asset}" -o "${tmp}/${asset}"
expected="$(awk -v asset="$asset" '{ name=$2; sub(/^\*/, "", name); sub(/^\.\//, "", name); if (name == asset) print tolower($1) }' "${tmp}/SHA256SUMS")"
actual="$(if command -v sha256sum >/dev/null 2>&1; then sha256sum "${tmp}/${asset}" | awk '{print $1}'; else shasum -a 256 "${tmp}/${asset}" | awk '{print $1}'; fi)"
[ "$actual" = "$expected" ] || { echo "Checksum mismatch for ${asset}." >&2; exit 1; }
tar -xzf "${tmp}/${asset}" -C "$tmp"
package="${asset%.tar.gz}"
[ -f "${tmp}/${package}/trepo" ] || { echo "Archive does not contain ${package}/trepo." >&2; exit 1; }
mkdir -p "$install_dir"
install -m 0755 "${tmp}/${package}/trepo" "${install_dir}/trepo"
echo "Installed trepo to ${install_dir}/trepo"
case ":${PATH}:" in *":${install_dir}:"*) ;; *) echo "Add ${install_dir} to PATH." ;; esac
