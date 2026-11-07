#!/usr/bin/env python3
"""Writes THIRD_PARTY_NOTICES.md: what Studio's app is built from, beyond its own code, and under
which licenses. The libraries and models are listed by hand below; the Rust crates come from
`cargo metadata`, so the list matches Cargo.lock. Run from anywhere:

    python3 scripts/third-party-notices.py
"""

import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent

HEADER = """# Third-party notices

Studio is free software under the GNU General Public License, version 3 or later (see LICENSE).
Its release builds also contain the software below. Each keeps its own license; the full texts of
the licenses named here are at https://spdx.org/licenses/.

## Built into the app

- **FFmpeg** (https://ffmpeg.org): decoding, encoding and scaling video and sound. Built from its
  `release/9.0` branch with `--enable-gpl --enable-version3`, statically: GPL-3.0-or-later. The exact
  commit used for a release is written to `SOURCES.txt` beside the release.
- **x264** (https://www.videolan.org/developers/x264.html): H.264 encoding. Built from commit
  `b35605ace3ddf7c1a5d67a2eb553f034aef41d55` of its stable branch: GPL-2.0-or-later.
- **libvpx** (https://chromium.googlesource.com/webm/libvpx): VP9 encoding for WebM. Built from
  the release tag written to `SOURCES.txt`: BSD-3-Clause.
- **Opus** (https://opus-codec.org): sound for WebM. Built from the release written to
  `SOURCES.txt`: BSD-3-Clause.
- **Lucide icons** (https://lucide.dev), through gpui-kit-assets: ISC.
- **RNNoise** (Xiph.Org), in its Rust port nnnoiseless, with its model: BSD-3-Clause.
- **DeepFilterNet3** (Hendrik Schröter), through deep-filter-rt, with its model: MIT OR Apache-2.0.

## Downloaded when first used (after asking)

- **Whisper tiny** (OpenAI), from huggingface.co/openai/whisper-tiny: Apache-2.0.
- **MobileSAM** (Chaoning Zhang et al., Apache-2.0 upstream), from huggingface.co/lmz/candle-sam.

## Source

Studio's source, and the build script that fetches and builds FFmpeg, x264, libvpx and Opus
(`scripts/release-macos.sh`), come with every release, as the GPL requires.

## Rust crates

"""


def crates():
    """Every crate in the build except Studio's own, as (name, version, license)."""
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout)
    workspace = set(metadata["workspace_members"])
    resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
    found = []
    for package in metadata["packages"]:
        if package["id"] in workspace or package["id"] not in resolved:
            continue
        license_name = package.get("license") or "see its license file"
        found.append((package["name"], package["version"], license_name))
    return sorted(found, key=lambda crate: (crate[0].lower(), crate[1]))


def main():
    rows = "\n".join(f"| {name} | {version} | {license_name} |" for name, version, license_name in crates())
    table = "| Crate | Version | License |\n| --- | --- | --- |\n" + rows + "\n"
    (ROOT / "THIRD_PARTY_NOTICES.md").write_text(HEADER + table, encoding="utf-8")
    print(f"Wrote THIRD_PARTY_NOTICES.md ({rows.count(chr(10)) + 1} crates)")


if __name__ == "__main__":
    main()
