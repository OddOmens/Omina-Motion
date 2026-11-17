#!/usr/bin/env bash
# Turns the packaged app in target/packages into a disk image beside it, with the app and a
# shortcut to Applications to drag it onto.
#
# Before that it makes sure the whole app is signed, not only the program inside it. The linker
# signs the program on its own, but without a signature over the whole app (its Info.plist and
# resources too) macOS calls a downloaded copy "damaged" and refuses to open it. An app already
# signed properly, for example with a Developer ID set as `signing-identity` in Cargo.toml, keeps
# its signature; otherwise it gets an ad-hoc one, which needs no Apple account.
#
# Usage: scripts/package-macos-dmg.sh "Omina Name"   (the app's product-name from Cargo.toml)
set -euo pipefail

product_name="${1:?usage: $0 \"<product name>\"}"
packages="$(cd "$(dirname "$0")/.." && pwd)/target/packages"
app="$packages/$product_name.app"
dmg="$packages/$product_name.dmg"

if [ ! -d "$app" ]; then
    echo "error: $app not found; run cargo packager --formats app first" >&2
    exit 1
fi

if codesign --verify --deep --strict "$app" 2>/dev/null; then
    echo "$product_name.app is already signed."
else
    echo "Signing $product_name.app (ad-hoc)…"
    codesign --force --deep --sign - "$app"
    codesign --verify --deep --strict "$app"
fi

# The disk image's contents are put together in a temporary folder, removed when the script ends.
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
ditto "$app" "$staging/$product_name.app"
ln -s /Applications "$staging/Applications"

rm -f "$packages"/*.dmg
# hdiutil's own guess at how big the image must be can come out too small for a large app
# ("No space left on device"), so the size is given: what the files take up, plus a fifth and
# 20 MB for the file system's own bookkeeping. The compressed image ends up much smaller.
size_mb=$(( $(du -sm "$staging" | cut -f1) * 6 / 5 + 20 ))
hdiutil create -volname "$product_name" -srcfolder "$staging" -size "${size_mb}m" -ov -format UDZO "$dmg"
echo "Done: $dmg"
