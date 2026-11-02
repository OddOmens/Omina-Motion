#!/usr/bin/env bash
# Builds Studio for macOS as a self-contained app (Studio.app) and disk image (Studio.dmg) in
# target/packages. FFmpeg and x264 are compiled from source and linked into the app, so it needs
# nothing installed on the Macs it runs on, and it runs on macOS 13 and later.
#
# Needs, on the machine that builds releases: Xcode's command-line tools, git, pkg-config and
# cargo-packager (`cargo install cargo-packager --locked`). The first run downloads and compiles
# x264 and FFmpeg, which takes several minutes; later runs reuse them.
#
# Signing and notarizing (so the app opens without Gatekeeper warnings) use your Apple Developer ID:
# put its name in apps/studio/Cargo.toml (`signing-identity` under [package.metadata.packager.macos]),
# and for notarizing set APPLE_ID, APPLE_PASSWORD (an app-specific password) and APPLE_TEAM_ID.
# Without them the app is built unsigned, which is fine for testing on this Mac.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# Every C and Rust object is built for macOS 13 and later.
export MACOSX_DEPLOYMENT_TARGET=13.0

# x264, pinned to one commit of its stable branch so release builds are reproducible.
x264_commit=b35605ace3ddf7c1a5d67a2eb553f034aef41d55
deps="$root/target/static-deps"
if [ ! -f "$deps/lib/libx264.a" ] || [ "$(cat "$deps/x264-commit" 2>/dev/null)" != "$x264_commit" ]; then
    echo "Building x264 (static)…"
    rm -rf "$deps/src/x264"
    mkdir -p "$deps/src"
    git clone --quiet https://code.videolan.org/videolan/x264.git "$deps/src/x264"
    git -C "$deps/src/x264" checkout --quiet "$x264_commit"
    (
        cd "$deps/src/x264"
        ./configure --prefix="$deps" --enable-static --enable-pic --disable-cli --disable-opencl \
            --extra-cflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"
        make -j"$(sysctl -n hw.ncpu)"
        make install
    )
    echo "$x264_commit" > "$deps/x264-commit"
fi
# FFmpeg's configure finds x264 here first; only the static library is installed, so it is linked in.
export PKG_CONFIG_PATH="$deps/lib/pkgconfig"

echo "Building Studio with FFmpeg built in…"
cargo build --release -p studio --features bundled-ffmpeg

# The app must not reach for libraries that only exist on this Mac.
if otool -L target/release/studio | grep -E '/opt/homebrew|/usr/local|@rpath' >/dev/null; then
    echo "error: the release binary still links libraries outside macOS:" >&2
    otool -L target/release/studio | grep -E '/opt/homebrew|/usr/local|@rpath' >&2
    exit 1
fi

# The notices inside the app list exactly what this build contains.
python3 scripts/third-party-notices.py

echo "Packaging…"
cargo packager --release -p studio --formats app,dmg

# The GPL asks that the source of a release be available: record exactly which FFmpeg and x264
# went into it, beside the release.
ffmpeg_source="$(find target/release/build -maxdepth 3 -type d -path '*ffmpeg-sys-next-*/out/ffmpeg-*' | head -n 1)"
{
    echo "Studio $(cargo pkgid -p studio | sed 's/.*[#@]//') for macOS"
    echo "Studio source: this repository at commit $(git rev-parse HEAD)"
    echo "FFmpeg: https://github.com/FFmpeg/FFmpeg at commit $(git -C "$ffmpeg_source" rev-parse HEAD)"
    echo "x264: https://code.videolan.org/videolan/x264 at commit $x264_commit"
} > target/packages/SOURCES.txt
echo "Done: target/packages"
