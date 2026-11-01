# Studio

One app for compositing and editing, built in Rust: After Effects-style layers,
keyframes and effects with Premiere-style editing, for macOS, Windows and Linux.

## Layout

```
crates/studio-core   Data model, frame-exact time, keyframes, playback timing, edits, alignment, and the
                     `.studio` project file format (JSON; loaded files are validated before use).
                     No UI or I/O, so the desktop app and a future headless render node share it.
crates/studio-render GPU compositor (wgpu: Metal, Vulkan, DirectX 12). Renders a composition frame without a
                     window: transforms and parenting, blend modes, track mattes, mask layers, background
                     removal masks, adjustment layers, effects (WGSL shaders), text with real fonts, and
                     shapes drawn from signed distances (smooth edges at any size, rounded corners, solid or
                     gradient fills, centered strokes). Pen paths are rasterized with tiny-skia in `path.rs`
                     at their on-screen size. Used by the viewer, export and render nodes.
crates/studio-media  Media probing and video decoding with FFmpeg (hardware or software, whichever measures
                     faster per clip), a frame server that decodes on background threads, and proxies:
                     all-intra ProRes Proxy copies built in the background so scrubbing is instant.
                     Also audio: decoding and resampling, a mixer for a composition's sound, and playback
                     through the sound card (cpal: CoreAudio, WASAPI, ALSA). And movie writing: H.264 + AAC
                     (.mp4) and ProRes 422 HQ + 24-bit PCM (.mov), converted to BT.709 YUV in `yuv.rs`.
crates/studio-captions Speech to timed words with OpenAI's Whisper (tiny), run locally with Candle (Metal on
                     macOS). Reads the model from disk; the app downloads it once, after asking.
crates/studio-cutout Background removal with MobileSAM, run locally with Candle (Metal on macOS): clicks pick
                     out a subject, and video follows it frame to frame. Reads the model from disk; the app
                     downloads it once (40 MB, checked by SHA-256), after asking.
crates/studio-export Exports a composition to a movie: GPU render, audio mix and encode, written under a
                     temporary name and renamed when done. No UI, so it also runs on a headless render machine.
apps/studio          Desktop app (GPUI + gpui-kit). UI only: panels render EditorState and forward input.
```

Inside `apps/studio/src`:

- `state/`: every editing action (playback, selection, keyframes, viewer drags, undo, opening and saving),
  unit-tested without a window. `state/history.rs` is undo and redo.
- `panels/timeline/tracks.rs`: the timeline's tracks (CapCut style): one row per track, several clips
  per row, thumbnails and waveforms, and each clip's keyframe rows when it is opened.
- `panels/toolbar.rs`: the top bar (where you are, and background jobs' progress) and the rail on the left:
  at the top the side panel (Media, Libraries, Effects) and the tools (Select, Cut, Text, Shape, Pen); at
  the bottom New Comp, Import, Export and Settings. Holding the Shape button opens the shape menu.
- Pen paths: the model and geometry are in `studio-core/src/path.rs`; drawing and point dragging are in
  `state/viewer.rs` and `panels/viewer.rs`. Masks (`Layer::mask`) let any layer show or hide another.
- `panels/cutout.rs`, `state/cutout.rs`: Remove Background (pick clicks, the mask job and tracking). Masks
  are grayscale PNGs per source frame under Studio's cache folder, in `Cutouts`.
- `panels/color_picker.rs`: the color picker every color chip opens: a saturation/brightness square, hue and
  opacity sliders, hex and opacity fields, the project's colors, paint type for shapes, and a Libraries tab.
- `panels/libraries.rs`, `state/library.rs`, `library_store.rs`: Libraries (brands holding shared colors and
  text styles, linked wherever they are used; the model and syncing are in `studio-core/src/library.rs`).
  They are stored in Studio's settings folder as `libraries.json`, with backups in `Library Backups`.
- `panels/captions.rs`, `state/captions.rs`: Generate Captions (download the model, transcribe in the
  background) and caption layers; their timing and animations are in `studio-core/src/captions.rs`.
- `state/copy.rs`: copy, paste and duplicate (Cmd+C, Cmd+V, Cmd+D), on top of `studio-core/src/copy.rs`.
- `workspace.rs`: the window, keyboard shortcuts, playback loop and panel layout.
- `sound.rs`: keeps the sound in step with playback (start, stop, mute changes, heard time).
- `project_files.rs`: Save, Save As, Open, asking about unsaved changes, reopening the last project, and the
  recovery copy of unsaved work.
- `app_settings.rs`: what Studio remembers between runs (last project, window placement, recovery copy, theme,
  accent color, proxy cache limit). `panels/settings.rs` is the Settings dialog (Cmd+,).
- `theme.rs`: the palettes. Panels name colors by `theme::color` constants and `theme::hsla` swaps in the
  active theme's (Dark, Black, Light) and accent's color, so new panels follow the theme for free.
- `panels/graph_editor.rs`: the Easings popover: easing families (In / Out / Both) and a curve editor.
- `panels/`: one module per panel. `panels/views.rs` wraps each panel in a cached GPUI view.
- `assets.rs`: the bundled Lucide icons. Add any new `IconName` here; a test fails if one is missing.
- `ui_tests.rs`: headless UI tests that click, drag and type into the real window offscreen.
  Elements a test drives need an id and `.test_support()` (a no-op outside tests).

## Requirements

FFmpeg's libraries and `pkg-config` (macOS: `brew install ffmpeg pkg-config`;
Debian/Ubuntu: `apt install libavcodec-dev libavformat-dev libswscale-dev libswresample-dev libasound2-dev pkg-config clang`).
H.264 export uses x264, which Homebrew's and most Linux distributions' FFmpeg include; without it, the
system's hardware H.264 encoder is used.

## Commands

```bash
cargo run -p studio            # run the editor (debug build)
cargo run -p studio --release  # run it at full speed (slower to compile)
cargo test --workspace         # run all tests
cargo clippy --workspace --all-targets
cargo build -p studio --release
cargo test -p studio frame_time --release -- --ignored --nocapture   # measure frame cost
cargo run -p studio-render --release --example render_frame -- out.png 100   # render a frame to PNG
cargo run -p studio-media --release --example decode_speed -- clip.mov         # decoding speed, software vs hardware
cargo run -p studio-media --release --example build_proxy -- clip.mov out.mov  # proxy build speed
cargo run -p studio-media --release --example scrub_speed -- clip.mov          # scrubbing, with and without a proxy
cargo run -p studio-media --release --example audio_clock -- "" 5               # sound start delay and clock (plays silence)
cargo run -p studio-export --release --example export -- project.studio out.mp4  # export without the editor (.mp4 or .mov)
```

## Conventions

- Time is whole frames; frame rates are exact fractions (24000/1001, not 23.976).
- A layer's `in_frame`/`out_frame` are where it shows; `start_frame` is where its source's first frame
  plays. Trimming moves in/out, slipping moves `start_frame`, moving shifts all three. Anything that reads
  footage or audio at a moment must go through `start_frame` (see `studio-core/src/trim.rs`).
- Track mattes pair a layer with the one directly above it, so anything that inserts or reorders layers must
  keep those pairs together (see how `studio-core/src/cut.rs` splits a layer with its matte).
- Every layer is on a track (`studio-core/src/tracks.rs`): 0 is the main track, picture tracks count up, sound
  tracks down, and clips on one track never overlap. The layer list stays the render order, kept sorted by
  track by `tracks::normalize`; call it after anything that adds, removes or re-tracks layers. With the
  magnetic main track on, the editor packs track 0 after each edit (in `record_history`).
- Playback position comes from a clock, never from counting screen refreshes: the time actually heard
  from the sound card while playing (so picture follows sound), or the wall clock when there is no sound
  device. The sound card's callback never blocks: it only copies from a lock-free queue.
- All project changes go through `studio-core::edit` / `EditorState`, never directly from UI code.
- Undo needs nothing from new edits: the workspace records a step whenever the project changed and no drag
  is in progress (a whole drag is one step). Changes the app makes by itself, such as fitting a text box to
  its text, must go through `History::absorb` so they do not become undo steps.
- Project files are written to a temporary file and then renamed over the old one, so a crash or full disk
  never leaves a half-written project. Missing media is looked for next to the project file on opening.
- Panels are cached views: an edit (`cx.notify()` on the workspace) refreshes every panel, while
  playback refreshes only the Viewer, Timeline and Properties. Keep dense rows free of heavy
  components (use plain elements, as the timeline does) and never query the OS during render.
- Every visible control either works or is shown disabled. Nothing is decorative.
- Rendering works in 16-bit float linear light with premultiplied alpha. Effects not drawn by the
  GPU yet are listed in `studio-render/src/effects.rs` and left out of the UI. Effects that would do nothing
  on a kind of layer (colour correction on text, anything on audio) are decided in `studio_core::effects::suits`.
- `unsafe` is forbidden workspace-wide, with one audited exception: `studio-media/src/hardware.rs`, which
  calls FFmpeg's hardware-decoding API directly. Every unsafe block there explains why it is sound.
- Proxies live in the system cache folder (macOS: `~/Library/Caches/Studio/Proxies`) and are named from the
  source's path, size and modification time; deleting them is always safe.
- GPUI is pinned to an exact version; upgrade deliberately.
