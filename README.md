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
                     A clip with a proxy plays from it alone (decoded ahead), and building waits
                     while anything plays.
                     Also audio: decoding and resampling, a mixer for a composition's sound, and playback
                     through the sound card (cpal: CoreAudio, WASAPI, ALSA). And movie writing: H.264 + AAC
                     (.mp4) and ProRes 422 HQ + 24-bit PCM (.mov), converted to BT.709 YUV in `yuv.rs`.
crates/studio-captions Speech to timed words with OpenAI's Whisper (tiny), run locally with Candle (Metal on
                     macOS). Reads the model from disk; the app downloads it once, after asking. Whisper's
                     phrase times are loose, so words are timed to where speech is heard in the sound,
                     with pauses matched to sentence ends. `CAPTIONS_MODEL=<folder> cargo test -p
                     studio-captions --test whisper -- --ignored` checks timing on real speech (macOS).
crates/studio-cutout Background removal with MobileSAM, run locally with Candle (Metal on macOS): clicks pick
                     out a subject, and video follows it frame to frame. Reads the model from disk; the app
                     downloads it once (40 MB, checked by SHA-256), after asking.
crates/studio-enhance Speech cleanup, like Adobe Podcast's Enhance Speech: AI noise removal (RNNoise "Light" or
                     DeepFilterNet3 "Studio", both built in), hum notches, voice EQ, de-essing, compression and
                     -16 LUFS loudness (BS.1770) with a peak limiter. Offline, on whole recordings.
                     `ENHANCE_CLEAN=… ENHANCE_NOISY=… ENHANCE_OUT=… cargo test -p studio-enhance --release --test
                     real_speech -- --ignored --nocapture` measures it on a recording.
crates/studio-export Exports a composition to a movie (H.264, H.265, ProRes, WebM), an animated GIF or a PNG
                     sequence, at the composition's frame rate or another: GPU render, audio mix and encode,
                     written under a temporary name and renamed when done. No UI, so it also runs on a
                     headless render machine.
apps/studio          Desktop app (GPUI + gpui-kit). UI only: panels render EditorState and forward input.
```

Inside `apps/studio/src`:

- `state/`: every editing action (playback, selection, keyframes, viewer drags, undo, opening and saving),
  unit-tested without a window. `state/history.rs` is undo and redo.
- `panels/timeline/tracks.rs`: the timeline's tracks (CapCut style): one row per track, several clips
  per row, thumbnails and waveforms, and each clip's keyframe rows when it is opened.
- `panels/toolbar.rs`: the top bar ("Studio" and the editing tools beside it as small icons named in
  tooltips; on the right the project and composition, and background jobs' progress) and the rail on the left: at the top the side panel (Media,
  Libraries, Effects and so on); at the bottom New Comp, Import, Export and Settings. Tools that share a
  button (Select / Track Select, Cut / Ripple / Roll / Slip / Slide, Hand / Zoom), the Shape button and the
  Eyedropper open a menu under them when held or right-clicked.
- Premiere's trim tools (ripple, roll, slide, moving a group) are in `studio-core/src/trim_tools.rs`;
  `state/layer_time.rs` applies each drag afresh to the timing as it was when the drag began.
- Pen paths: the model and geometry are in `studio-core/src/path.rs`; drawing and point dragging are in
  `state/viewer.rs` and `panels/viewer.rs`. Masks (`Layer::mask`) let any layer show or hide another.
- `panels/cutout.rs`, `state/cutout.rs`: Remove Background (pick clicks, the mask job and tracking). Masks
  are grayscale PNGs per source frame under Studio's cache folder, in `Cutouts`. The model only looks at
  frames the composition shows that have changed (others link the mask before them), edges are refined
  against the picture (`studio_cutout::Mask::refine`), and Settings > AI Models > Run cooler rests the GPU
  between frames. `CUTOUT_MODEL=<folder> cargo test -p studio-cutout --release --test speed -- --ignored
  --nocapture` times each stage.
- The Export dialog (⌘M, `panels/export_dialog.rs`): a preview of the frames being exported, with a scrubber
  across exactly that range and frame-step arrows, over a checkerboard when the format keeps transparency;
  a summary of the file (size, frame rate, length, sound, and its size for a fixed bitrate); and grouped
  settings: format (H.264, H.265 / HEVC, five ProRes flavors, WebM with transparency, animated GIF, PNG
  sequence), quality and speed for H.264, H.265 and WebM, size, frame rate (faster rates repeat frames, slower
  ones skip them; GIFs stop at 50), work area or whole composition, and sound with its AAC (or, for WebM, Opus)
  bitrate.
- Quick Export (a clip's right-click menu, `panels/export.rs`): renders one layer alone (`studio-core/src/bake.rs`)
  to ProRes 4444 with transparency in an Exports folder beside the project; Export and Replace imports it
  into the project's Exports folder and puts it in the layer's place.
- The export queue (`panels/export_queue.rs`, File ▸ Export Queue…): the Export dialog's Add to Queue… saves an
  export with the project as it is then; Start Queue runs them one after another.
- File ▸ Package… (`panels/package.rs`, `package_writer.rs`, `studio-core/src/package.rs`), like After
  Effects' Collect Files: the project and every file it links to (footage, images, sound, LUT files) in one
  zip. Inside a folder named after the package, the media is copied into folders that mirror the Project
  panel's (media at the top level goes in `Media`, LUTs in `LUTs`, clashing names get " 2", " 3"), and the
  packaged `.studio` points at the copies by paths relative to itself. Missing files are left out and listed
  in `Missing files.txt`. It runs in the background with progress and Cancel in the top bar.
- `panels/color_picker.rs`: the color picker every color chip opens: a saturation/brightness square, hue and
  opacity sliders, hex and opacity fields, the project's colors, the picked and saved colors, paint type for
  shapes, and a Libraries tab.
- The right panel's tabs are Properties, Effects (the selected layer's effects, keyframable, with an Add
  Effect menu and sliders for settings with a modest range) and Color. All three show their groups as the
  same folding cards (`chrome::card`), with the same heading buttons for effects (on/off, reset, remove).
- The top bar's New Layer button and the Layer menu add a solid (in the eyedropper's current color, ⌘Y), an
  adjustment layer (⌥⌘Y) or a null object (⇧⌥⌘Y, a layer that draws nothing, for parenting), and hold Add or
  Remove Marker (M), Sequence Layers and Stagger Layers (`studio-core/src/edit.rs`). The Rate Stretch tool (R,
  with Cut, Ripple and Roll) drags a clip's end to change its speed (`speed::stretch_to`). The viewer's grid
  button opens a guides menu (' shows or hides them): thirds and the center on any frame, title and action
  safe on 16:9, and the TikTok, Instagram Reels and YouTube Shorts safe zones on 9:16 (`panels/guides.rs`;
  the social insets are approximate, from the apps' common 1080×1920 templates).
- Drops: Media items and files from the Finder can be dropped on the Media panel (import only), on a track
  (clips there, one after another) or on the viewer (`EditorState::add_items_at`: from the playhead on a new
  top track, centered where let go; full-frame pictures stay centered).
- Media slots (`studio-core/src/slots.rs`, `EditorState::drop_items_at`): a photo or video dragged over the viewer
  outlines the slot under the pointer, looking inside nested compositions. A slot is a "Drop your … here"
  placeholder (as templates, mockups and elements make; `slots::placeholder_name`) or a nested composition showing
  one photo or video. Dropping on a placeholder turns it into a composition of its size ("Photo slot") holding the
  media, scaled to cover and cropped to the placeholder's shape by a hidden mask; the layer keeps its id,
  animation, effects and timing. Dropping on a filled slot swaps the media inside, scaling every Scale keyframe so
  a pan or zoom carries on, and every place showing that composition updates. Other files dropped at the same
  time are added as layers as usual. A video's sound plays through its slot.
- Typing text in the viewer (`panels/viewer.rs`): the Type tool's click on empty space starts a text layer, and
  on a text layer edits it with the caret there (as does a double-click with the Selection tool). The picture
  shows the text as it will look while typing (`with_typed_text`), with a blinking caret and the selection
  drawn on the real glyphs from the layout (`studio_render::text::TextGeometry`); the field taking the keys is
  hidden. Return starts a new line; ⌘Return, Escape or a click off the text finishes. Click and drag on the
  text to place the caret or select; double-click selects a word.
- Text animators (`studio-core/src/text_animation.rs`, Motion ▸ Text Animation): an In and an Out
  animation per text layer that moves its characters, words or lines one after another (Fade, Rise, Drop,
  Slide, Pop, Zoom, Spin, Typewriter), in order, reverse, from the center or shuffled, with an easing, a
  duration for the whole and how much the pieces overlap. Nothing is keyframed: the renderer works out each
  piece's pose per frame and draws it as its own part of the text's raster (`draw_text_pieces`).
- Gradients (`studio-core/src/gradient.rs`): two to eight color stops and a from/to line in fractions of the
  layer's box. `panels/gradient_editor.rs` is the bar under a shape's fill or stroke (click to add a stop, drag
  to move, click a stop for its color, Location, Delete, Reverse); the viewer shows the line with a handle on
  each end to drag (`state/gradient.rs`). Files with the older two-color gradients still open.
- `panels/color_grade.rs`, `state/grade.rs`: the Color tab (like Lumetri): grade the selected clip (to grade
  the whole video, select an adjustment layer over it). Like the Properties tab it is a list of cards, one
  per correction: those the clip has come first, in the order they apply, with turn off, reset and remove in
  their headings; the rest are folded with a + to add them. Each is a card for one of the effects Basic Correction
  (grouped into White Balance with a picker, Light and Color), Apply LUT, Color Wheels (sized to the panel,
  each with a brightness slider), Tone Curve, Levels and Vignette. Values are sliders on colored tracks,
  with a typed value and double-click to reset; the same effects keyframe in the Effects tab. LUTs are `.cube` files or the 31 built-in looks in five groups
  (`studio-core/src/grade.rs`), drawn by `shaders/lut.wgsl` from a 3D texture. Imported LUTs are copied
  into Studio's settings folder, in `LUTs` (`lut_library.rs`), and listed in the Effects panel, where pointing at one previews it in the viewer (on the selected clips, or the whole picture). The Color
  tab's LUT browser and the Effects panel show each LUT on the current frame (`panels/lut_previews.rs`:
  the frame is drawn small without the LUT being chosen, then each table is applied on the CPU). `GRADE_SHEET=/path.png cargo test -p studio-render --test render grade_sheet -- --ignored`
  renders every look into one picture to check.
- `panels/color_well.rs`: the Eyedropper tool (I) keeps a current color, like Photoshop's foreground color:
  click the viewer to pick it (Shift-click also fills the selected shapes, text and solids). Holding or
  right-clicking the top bar's Eyedropper button opens it, to apply it, save it as a swatch or into a library, or
  reuse a recent pick. Recent picks and swatches are kept in Settings, for every project.
- `panels/libraries.rs`, `state/library.rs`, `library_store.rs`: Libraries (brands holding shared colors and
  text styles, linked wherever they are used; the model and syncing are in `studio-core/src/library.rs`).
  They are stored in Studio's settings folder as `libraries.json`, with backups in `Library Backups`.
- `panels/analysis.rs`: background jobs that study a clip first: Remove Silences (`studio-core/src/silence.rs`),
  Mark Beats (`studio-media/src/beats.rs`; markers snap), Track Motion (`studio-media/src/track.rs`) and Auto
  Reframe (follows the subject with the background-removal model). Speed, reverse, ramps and freeze frames
  (Insert Freeze Frame, a clip's right-click menu) are in `studio-core/src/speed.rs`; Speed is also a
  keyframeable property (`PropertyPath::Speed`), which remaps time; the mixer plays sped-up sound, ducks music
  under a voice, and gates noise.
- Sound in nested compositions (`studio-core/src/audio.rs`, `composition_audio`): the sound inside a nested
  composition plays where its layer starts, heard only while that layer shows, at any depth. The composition
  layer's audio switch, solo, volume, fades, ducking and noise gate apply to everything inside it (its Audio
  card and the mute button on its clip; Remove Silences and Mark Beats are for single files, so they're hidden).
  Each nested clip gets a mixer id made from the chain of layers it plays through, so a composition used twice
  plays as two clips. Older files saved composition layers with sound switched off; they're switched on when
  opened (`switch_on_nested_sound`).
- `panels/templates.rs`: the Templates panel (with a Colors menu that makes a template in any of the template
  palettes, or in a Library brand's colors and font) and its View All gallery, which shows every template at once,
  grouped by category. Hovering over a card plays the template on it (15 pictures a second, built once into a
  scratch project); its + button adds it and posts a notification. The 61 templates (one for each of 61 animated layouts, so no two are the same design in
  other colors; each in 16:9, 4:5 and 9:16) are in `studio-core/src/templates/`; `catalog.rs` holds their words,
  colors and fonts and the named palettes (`LOOKS`), `social.rs` the intro and social layouts (profile card, Ask
  Me Anything, name reveal, story, comment reply, greeting, outlined words, vlog title), `ads.rs` the ad and UGC
  layouts (captions over a talking head, us versus them, problem and solution, feature callouts, a countdown
  offer, a star rating), and `editorial.rs` bold editorial ones with flat color and big type (a poster whose
  lines stretch in, sliding color blocks, a repeated word, stacking photo cards, a full-screen countdown, a title
  spinning out of a blur, shutters off a photo, gradient bars, words echoing over a photo, a bouncing full stop). Headlines come in word
  by word with text animators, everything leaves in a staggered exit at the end, and in 9:16 buttons and key words
  stay clear of the captions and buttons phone apps draw over the bottom of a video.
  `TEMPLATE_SHEET=/path.png cargo test -p studio-render --test render template_sheet -- --ignored` renders
  them all into one picture to check, and `TEMPLATE_STRIP=/path.png TEMPLATE_PICK=0,5 cargo test -p studio-render
  --test render template_strip -- --ignored` renders chosen ones across their running time to check how they move
  (with `TEMPLATE_FILL=/picture.png`, every placeholder is filled with that picture first).
- `panels/mockups.rs`: the Mockups panel. Device mockups for iPhone, iPhone Duo, iPad, Mac, Apple Watch (with bands) and
  a browser window (1920 × 1080 page, light or dark), drawn as shapes
  (`studio-core/src/mockups.rs`). A card's + opens the finishes that device comes in (`Device::finishes`, a list per
  device); picking one adds the device upright to the open composition as one layer, centered and scaled to fit
  (rotate the layer to turn it). Looking at the cards adds nothing. The layer is a see-through
  mockup composition sized to the device, showing a Screen composition at the device's own screen size (the selected
  video or picture fills it) through a rounded mask. `MOCKUP_SHEET=/path.png cargo test -p studio-render --test render
  mockup_sheet -- --ignored` renders them all into one picture.
- `panels/elements.rs`: the Elements panel (shapes, arrows, badges, frames, social buttons and emoji
  stickers, defined in `studio-core/src/elements.rs`). `ELEMENT_SHEET=/path.png cargo test -p studio-render
  --test render element_sheet -- --ignored` renders them all into one picture.
- The right panel's Motion tab (`panels/motion.rs`) holds everything about movement: the graph editor
  (`panels/graph_editor.rs`, Keyframe Easing) for the keyframes selected in the timeline, which the
  timeline's Easings button, a click on the line between two keyframes or a double-click on one opens;
  and the Animate and Text Animation sections. Each In and Out animation has its own easing (Smooth,
  Gentle, Snappy, Overshoot, Spring, Bounce, Linear, or its motion's default) and a curve whose handles drag.
- Shape morphing: `studio-core/src/morph.rs`. A shape layer's `morph` holds outline keyframes (and,
  after Morph To, fill and stroke keyframes). Between two keyframes the outlines are cut into the same
  number of points along their length (`resample`, which keeps the exact shape), the second is started
  and run the way that moves its points least (`matched`), then blended point by point; a closed outline
  morphing into an open one is opened at its first point. Drawn shapes become equivalent pen paths
  (`outline`, rounded corners included) before they morph. The Motion tab's Morph card
  (`panels/morph.rs`) has the Path stopwatch and keyframe buttons, and dragging a morphing path's points
  in the viewer keyframes the outline at the playhead. Morph To… (a layer's right-click menu, the Layer
  menu, or the Morph card with two shapes selected) keyframes the first selected shape into the second's
  outline, colors, position, scale, rotation and opacity over a chosen length, then hides the second.
  Style edits on a morphing shape apply across the whole morph. `MORPH_SHEET=/path.png cargo test -p
  studio-render --test render morph_sheet -- --ignored` renders frames across three morphs.
- Magic Move (Composition ▸ Magic Move…, `panels/magic_move.rs`, `studio-core/src/magic_move.rs`): a new
  composition that plays the open one, changes it into another of the same size and frame rate, then
  plays that. During the change, short-lived "bridge" copies of the layers are keyframed from the first's
  last frame to the second's first: layers with the same name morph (two shapes), move into place (same
  content) or move while crossfading (different content); the rest fade out or in. Picture tracks start
  at 1, leaving the magnetic main track empty so nothing slides out of time. `MAGIC_MOVE_SHEET=/path.png
  cargo test -p studio-render --test render magic_move_sheet -- --ignored` renders frames across one.
- Animation presets (In, Out and Loop, in the Motion tab's Animate section) are in
  `studio-core/src/presets.rs`; transitions between clips (the Effects panel's Transitions, dragged onto a
  cut) are in `studio-core/src/transitions.rs`.
- `panels/resize.rs`: Magic Resize (Composition menu): copies of the open composition in other sizes, with
  the layers rearranged by `studio-core/src/resize.rs`. A composition's right-click menu in the Project panel has
  Duplicate and Resize, which makes one such copy (9:16 to 4:5, 16:9 to 9:16 and so on) beside the original,
  named for its new shape, and opens it.
- Importing other apps' projects: `studio-core/src/import/` reads After Effects projects (`aep.rs`, from the
  RIFX chunks; the format isn't published) and Lottie animations (`lottie.rs`) into a `Scene`, which `build`
  turns into Studio folders, compositions and layers after checking it like a project file. Import (⌘I) and
  drag and drop take `.aep` and `.json` files. `IMPORT_FILE=… IMPORT_SHEET=sheet.png cargo test -p
  studio-render --test render import_sheet -- --ignored` renders an import's compositions for a look.
- `panels/enhance.rs`, `state/enhance.rs`: Enhance Speech (Properties): a cleaned copy of a clip's whole sound
  is made in the background, kept in Studio's cache under `Enhanced`, and played (and exported) instead of the
  original; settings and the copy are kept in step by `studio-core/src/enhance.rs`.
- `panels/captions.rs`, `state/captions.rs`: the Captions panel and Generate Captions (download the model, transcribe in the
  background) and caption layers; their timing and animations are in `studio-core/src/captions.rs`. In
  Properties, Sync shows every caption of a layer some frames later or earlier (kept when another look is
  applied), and each transcript phrase's start and end can be typed or dragged; its words stretch to fit
  without crossing their neighbors (`captions::retime_phrase`).
- `state/copy.rs`: copy, paste, Paste Attributes (Alt+Cmd+V, `panels/paste_attributes.rs`) and duplicate
  (Cmd+C, Cmd+V, Cmd+D), on top of `studio-core/src/copy.rs`, and Close Gaps (`studio-core/src/cut.rs`).
- `keymap.rs`: every command that can have a shortcut, the Studio, After Effects and Premiere Pro sets, and
  the person's own changes (Settings ▸ Keyboard Shortcuts). Tooltips and menu labels read keys from it, so
  they follow custom shortcuts.
- `versions.rs`, `panels/versions.rs`: earlier versions of a project, kept on each save and every few minutes
  of work under Studio's settings folder (`Versions`), listed and restored from File ▸ Browse Versions….
- `preview_cache.rs`: rendering ahead: a background thread with its own renderer renders the frames after the
  playhead and keeps them in memory (Settings ▸ Playback), so playback shows them; a green line on the ruler
  marks them. Render & Play (Shift+Space, the film button by the timeline's transport) renders the whole work
  area first, then plays it in a loop. The renderer also draws runs of simple layers (normal blending, no effects, matte or mask) in
  one pass. `cargo test -p studio --release stress_500_layers -- --ignored --nocapture` measures a 500-layer
  project; the timeline only draws the tracks near what can be seen.
- `workspace.rs`: the window, keyboard shortcuts, playback loop and panel layout. Cmd+[ / Cmd+] step a frame
  (with Shift, five); Cmd+= / Cmd+- zoom the viewer, and the scroll wheel over it zooms around the pointer
  (`panels/viewer.rs`, `zoom_to`).
- `sound.rs`: keeps the sound in step with playback (start, stop, mute changes, heard time).
- `project_files.rs`: Save, Save As, Open, Close Project (Cmd+W), asking about unsaved changes, what shows on
  launch, and the recovery copy of unsaved work.
- `panels/start_screen.rs`: the start screen, like After Effects' Home: shown on launch (Settings > General can
  reopen the last project instead), from File > Start Screen and after Close Project. It lists recent projects
  (filterable; right-click to show in Finder or remove), and starts a new project, opens one, or starts from a
  template (the template's composition becomes the new project's only one). File > Open Recent lists the same
  projects (`workspace::refresh_menus` rebuilds the menu bar when they change).
- `crash_log.rs`: a crash writes a report (error, place, call chain, version) to `Crash Reports` in the settings
  folder; the next launch says so once and offers to show it. Nothing is sent.
- `updates.rs`: Studio > Check for Updates, and a quiet check once a day at launch, read a small JSON feed (see
  Releasing). A newer version is offered as a download link; nothing installs itself.
- `app_settings.rs`: what Studio remembers between runs (recent projects, window placement, recovery copy, theme,
  accent color, Project panel columns, default folders and import sorting, cache folder, proxy settings).
  `panels/settings.rs` is the Settings dialog (Cmd+,), with sections in a menu on its left.
- `panels/project.rs`: the Project panel, with a preview that shows images and plays video and sound;
  `studio-core/src/folders.rs` adds default folders and sorts imports into them. Right-clicking an item opens
  its menu: Open, Composition Settings and Duplicate and Resize (compositions), Rename, Label, Reveal in Finder,
  Delete. Clicking an item's label swatch picks another label color. Double-clicking
  a name renames it (as on a timeline clip), and Cmd+D there duplicates the selected item
  (`studio-core/src/copy.rs`: a composition's copy gets copies of its layers).
- Right-clicking a layer, on its clip or in the viewer, opens `panels/layer_menu.rs`: Rename, Duplicate,
  Pre-compose… (`panels/precompose.rs`, with After Effects' trim-to-span and open options), Ungroup Composition
  (`studio-core/src/nesting.rs`: the nested composition's layers come back in its place, through a null when it
  was moved or scaled; the composition stays in the project), Flip Horizontal/Vertical, Reset Transform, Split,
  Delete and the exports. Holding the middle mouse button over the viewer pans it.
- Videos and images have Size above Scale in Layer Transform (`edit::set_media_size`): the pixels they are
  drawn at, so Scale can animate from 0 to 100% of that. Reset Transform puts them back at their own size.
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
system's hardware H.264 encoder is used. H.265 export likewise uses x265 when FFmpeg has it (Homebrew's
does) and the hardware encoder otherwise, which is what release builds use. WebM export uses libvpx (VP9,
with an alpha plane for transparency) and libopus, which Homebrew's FFmpeg includes; an FFmpeg without them
shows WebM greyed out in the Export dialog. GIF and PNG export need nothing extra.

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
  never leaves a half-written project. Media paths may be absolute or relative to the project file (as
  packages save them); missing media is looked for next to the project file on opening.
- Panels are cached views: an edit (`cx.notify()` on the workspace) refreshes every panel, while
  playback refreshes only the Viewer, Timeline and Properties. Keep dense rows free of heavy
  components (use plain elements, as the timeline does) and never query the OS during render.
- Every visible control either works or is shown disabled. Nothing is decorative.
- Rendering works in 16-bit float linear light with premultiplied alpha. Effects not drawn by the
  GPU yet would be listed in `studio-render/src/effects.rs` (none are now) and left out of the UI.
  `EFFECT_SHEET=/path.png cargo test -p studio-render --test render effect_sheet -- --ignored` renders every effect. Effects that would do nothing
  on a kind of layer (colour correction on text, anything on audio) are decided in `studio_core::effects::suits`.
- `unsafe` is forbidden workspace-wide, with two audited exceptions in `studio-media`: `hardware.rs` (FFmpeg's
  hardware-decoding API) and `color.rs` (telling FFmpeg's scaler a video's YUV formula). Every unsafe block
  explains why it is sound.
- Project files are untrusted input: `project_file::check` refuses anything the editor or renderer couldn't
  handle (ids, frames, sizes, text sizes and spacing, parent and nesting loops) with a "damaged" message. The
  renderer also guards itself (text metrics, path raster sizes, a budget of nested renders per frame, frames
  scaled down to fit 8192 pixels), and GPU errors fail the frame instead of panicking (release builds abort
  on panic).
- Proxies live in the system cache folder (macOS: `~/Library/Caches/Studio/Proxies`) and are named from the
  source's path, size and modification time; deleting them is always safe.
- GPUI is pinned to an exact version; upgrade deliberately.

## Privacy and security

- **Network.** Studio contacts only: Hugging Face, to download the captions (Whisper) and cut-out (MobileSAM)
  models, after asking or from Settings > AI Models (pinned files, each checked against its SHA-256 before use;
  `https` only); and, in builds given one, the update feed, once a day and from Check for Updates (`https`
  only, a plain request with no information about you). There is no telemetry or analytics, and crash
  reports stay on the Mac unless you send one.
- **On disk.** Settings, libraries (and up to 30 backups), downloaded models and the recovery copy of unsaved
  work live in `~/Library/Application Support/Studio`. Proxies, background-removal masks, cleaned speech and
  images from imported animations live in `~/Library/Caches/Studio` (or the folder chosen in Settings >
  Media Cache), and each can be cleared there.
- **Sharing projects.** A `.studio` file holds the full paths of its media (which include your user name) and
  of files in Studio's cache; it holds no settings or libraries. In a package (File ▸ Package…) the copied
  media's paths are relative, but the cache paths and the paths of missing files stay full. Exported movies carry
  only the encoder's tags: no project name, paths or dates.

## Releasing

`scripts/release-macos.sh` makes `target/packages/Studio.app`, a disk image and `SOURCES.txt`. It builds
x264, libvpx, Opus and FFmpeg from source (pinned) and links them into the app with the `bundled-ffmpeg` feature, so the
app needs nothing installed and runs on macOS 13 and later; it stops if the binary still links anything
outside macOS. Packaging is `cargo-packager` (configured in `apps/studio/Cargo.toml`), which can also make
Windows and Linux installers later. Everyday builds skip all this and use the system's FFmpeg
(`brew install ffmpeg`).

- **Needs** (release machine only): Xcode's command-line tools, git, pkg-config and
  `cargo install cargo-packager --locked`.
- **Signing and notarizing:** set `signing-identity` in `apps/studio/Cargo.toml` and, for notarizing,
  `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`. Unsigned builds work for testing.
- **Updates:** build with `STUDIO_UPDATE_URL=https://…/latest.json` to turn on Check for Updates. Publish that
  file with each release: `{"version": "0.2.0", "url": "https://…/Studio-0.2.0.dmg", "notes": "What's new."}`
  (the link must be `https`). Without the variable the menu item is off.
- **Icon:** `python3 scripts/make-icon.py` draws `assets/icon`.
- **Notices:** `python3 scripts/third-party-notices.py` rewrites `THIRD_PARTY_NOTICES.md` (the release
  script does this too); it ships inside the app with `LICENSE`.
- **Dependency advisories:** run `cargo install cargo-audit --locked && cargo audit` before each release.
- **Models:** Whisper tiny is Apache-2.0 on Hugging Face. MobileSAM is Apache-2.0 upstream, but the copy
  Studio downloads (`lmz/candle-sam`) states no license: confirm it, or download from a source that does.

## License

Studio is free software: you can redistribute it and/or modify it under the terms of the GNU General Public
License as published by the Free Software Foundation, either version 3 of the License, or (at your option)
any later version. See `LICENSE`. Release builds include FFmpeg and x264, also under the GPL; see
`THIRD_PARTY_NOTICES.md`.
