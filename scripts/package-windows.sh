#!/usr/bin/env bash
# Packs the Windows cross-build into the portable zip the in-app updater installs (it looks for a "-portable.zip"
# release asset, and needs eve-maj-preview.exe and config.exe at the zip's root).
#
#   cargo build --release            # default target is x86_64-pc-windows-gnu (see .cargo/config.toml)
#   scripts/package-windows.sh v2.1.0
#
# Writes dist/eve-maj-rusty-<version>-windows-x86_64-portable.zip and its .sha256.
set -euo pipefail

version="${1:?usage: $0 <version, e.g. v2.1.0>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
release="$root/target/x86_64-pc-windows-gnu/release"
name="eve-maj-rusty-${version}-windows-x86_64-portable"

for exe in eve-maj-preview.exe eve-maj-config.exe; do
    [[ -f "$release/$exe" ]] || { echo "missing $release/$exe; run 'cargo build --release' first" >&2; exit 1; }
done
# config.exe's WebView2 loader (Microsoft's redistributable, shipped inside the webview2-com-sys crate). On the GNU
# toolchain it is loaded as a DLL, so it has to sit next to config.exe.
# Newest copy; cargo puts build output in build/<crate>-<hash>/ or, in newer versions, build/<crate>/<hash>/.
loader="$(find "$release/build" -path '*webview2-com-sys*/out/x64/WebView2Loader.dll' -printf '%T@ %p\n' 2>/dev/null | sort -rn | head -n1 | cut -d' ' -f2- || true)"
[[ -n "$loader" ]] || { echo "WebView2Loader.dll not found under $release/build" >&2; exit 1; }

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp "$release/eve-maj-preview.exe" "$stage/eve-maj-preview.exe"
# The installer script and the tray both expect the Zig build's name.
cp "$release/eve-maj-config.exe" "$stage/config.exe"
cp "$loader" "$stage/WebView2Loader.dll"
cp "$root/README.md" "$root/LICENSE" "$root/CascadiaCode-LICENSE.txt" "$stage/"

mkdir -p "$root/dist"
rm -f "$root/dist/$name.zip"
(cd "$stage" && zip -q -9 -X "$root/dist/$name.zip" ./*)
(cd "$root/dist" && sha256sum "$name.zip" > "$name.zip.sha256")
echo "$root/dist/$name.zip"
unzip -l "$root/dist/$name.zip"
