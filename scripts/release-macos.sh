#!/usr/bin/env bash
# Builds Studio for macOS as a self-contained app (Studio.app) and disk image (Studio.dmg) in
# target/packages. FFmpeg and x264 are compiled from source and linked into the app, so it needs
# nothing installed on the Macs it runs on, and it runs on macOS 13 and later.
#
# Needs, on the machine that builds releases: Xcode's command-line tools, git, pkg-config and
# cargo-packager (`cargo install cargo-packager --locked`). The first run downloads and compiles
# x264, libvpx, Opus and FFmpeg, which takes several minutes; later runs reuse them.
#
# Signing and notarizing (so the app opens without Gatekeeper warnings) use your Apple Developer ID:
# put its name in apps/studio/Cargo.toml (`signing-identity` under [package.metadata.packager.macos]),
# and for notarizing set APPLE_ID, APPLE_PASSWORD (an app-specific password) and APPLE_TEAM_ID.
# Without them the app gets an ad-hoc signature, which opens after a one-time approval.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# Every C and Rust object is built for macOS 13 and later.
export MACOSX_DEPLOYMENT_TARGET=13.0

# x264, pinned to one commit of its stable branch so release builds are reproducible.
x264_commit=b35605ace3ddf7c1a5d67a2eb553f034aef41d55
# x264's and FFmpeg's build tools break on paths with spaces, and this repository usually lives in
# one ("Omina Studio/Omina Motion"), so the release is compiled in a folder without any and the
# finished binary is copied back into target/release for packaging.
build_root="${STUDIO_BUILD_DIR:-$HOME/Library/Caches/omina-motion-release}"
case "$build_root" in
    *[[:space:]]*) echo "error: STUDIO_BUILD_DIR must not contain spaces: $build_root" >&2; exit 1 ;;
esac
export CARGO_TARGET_DIR="$build_root/target"
deps="$build_root/static-deps"
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
# libvpx (VP9, for WebM), pinned to a release tag. Its configure is a plain shell script, so it
# needs nothing beyond Xcode's tools on Apple silicon.
vpx_tag=v1.15.0
if [ ! -f "$deps/lib/libvpx.a" ] || [ "$(cat "$deps/vpx-tag" 2>/dev/null)" != "$vpx_tag" ]; then
    echo "Building libvpx (static)…"
    rm -rf "$deps/src/libvpx"
    mkdir -p "$deps/src"
    git clone --quiet --depth 1 --branch "$vpx_tag" https://chromium.googlesource.com/webm/libvpx "$deps/src/libvpx"
    # libvpx's configure only recognises macOS versions it was released alongside, and on a
    # newer one falls back to plain C with none of the processor's fast paths, so the target is
    # named outright (the newest it knows; the code it builds runs on later versions too).
    case "$(uname -m)" in
        arm64) vpx_target=arm64-darwin23-gcc ;;
        *) vpx_target=x86_64-darwin23-gcc ;;
    esac
    (
        cd "$deps/src/libvpx"
        ./configure --target="$vpx_target" --prefix="$deps" --enable-static --disable-shared --enable-pic --enable-vp9 \
            --disable-examples --disable-tools --disable-docs --disable-unit-tests \
            --extra-cflags="-mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"
        make -j"$(sysctl -n hw.ncpu)"
        make install
    )
    echo "$vpx_tag" > "$deps/vpx-tag"
fi
# Opus (WebM's sound), from its release tarball, which comes with a ready-made configure. The
# download is checked against the checksums Xiph.Org publishes for its releases.
opus_version=1.5.2
if [ ! -f "$deps/lib/libopus.a" ] || [ "$(cat "$deps/opus-version" 2>/dev/null)" != "$opus_version" ]; then
    echo "Building Opus (static)…"
    rm -rf "$deps/src/opus-$opus_version"
    mkdir -p "$deps/src"
    (
        cd "$deps/src"
        curl -fsSLO "https://downloads.xiph.org/releases/opus/opus-$opus_version.tar.gz"
        curl -fsSL https://downloads.xiph.org/releases/opus/SHA256SUMS.txt | grep " opus-$opus_version.tar.gz\$" | shasum -a 256 -c -
        tar xzf "opus-$opus_version.tar.gz"
        cd "opus-$opus_version"
        ./configure --prefix="$deps" --enable-static --disable-shared --with-pic --disable-doc --disable-extra-programs \
            CFLAGS="-O2 -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"
        make -j"$(sysctl -n hw.ncpu)"
        make install
    )
    echo "$opus_version" > "$deps/opus-version"
fi
# FFmpeg's configure finds x264, libvpx and Opus here first; only their static libraries are
# installed, so they are linked in.
export PKG_CONFIG_PATH="$deps/lib/pkgconfig"

# Check for Updates reads the feed named here; without it the menu item is turned off.
if [ -z "${STUDIO_UPDATE_URL:-}" ]; then
    echo "warning: STUDIO_UPDATE_URL is not set, so this build can't check for updates." >&2
fi

echo "Building Studio with FFmpeg built in…"
cargo build --release -p studio --features bundled-ffmpeg
mkdir -p target/release
cp "$CARGO_TARGET_DIR/release/studio" target/release/studio

# The app must not reach for libraries that only exist on this Mac.
if otool -L target/release/studio | grep -E '/opt/homebrew|/usr/local|@rpath' >/dev/null; then
    echo "error: the release binary still links libraries outside macOS:" >&2
    otool -L target/release/studio | grep -E '/opt/homebrew|/usr/local|@rpath' >&2
    exit 1
fi

# The notices inside the app list exactly what this build contains.
python3 scripts/third-party-notices.py

echo "Packaging…"
CARGO_TARGET_DIR="$root/target" cargo packager --release -p studio --formats app
# The disk image is made separately so the whole app is signed first; see that script.
./scripts/package-macos-dmg.sh "Omina Motion"

# The GPL asks that the source of a release be available: record exactly which FFmpeg, x264,
# libvpx and Opus went into it, beside the release.
ffmpeg_source="$(find "$CARGO_TARGET_DIR/release/build" -maxdepth 3 -type d -path '*ffmpeg-sys-next-*/out/ffmpeg-*' | head -n 1)"
{
    echo "Studio $(cargo pkgid -p studio | sed 's/.*[#@]//') for macOS"
    echo "Studio source: this repository at commit $(git rev-parse HEAD)"
    echo "FFmpeg: https://github.com/FFmpeg/FFmpeg at commit $(git -C "$ffmpeg_source" rev-parse HEAD)"
    echo "x264: https://code.videolan.org/videolan/x264 at commit $x264_commit"
    echo "libvpx: https://chromium.googlesource.com/webm/libvpx at tag $vpx_tag"
    echo "Opus: https://downloads.xiph.org/releases/opus/opus-$opus_version.tar.gz"
} > target/packages/SOURCES.txt
echo "Done: target/packages"
