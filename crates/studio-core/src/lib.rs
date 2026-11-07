//! Studio's engine-side core: the project data model, frame-exact time math,
//! keyframe animation, playback timing and project edits. It has no UI or I/O
//! dependencies, so the desktop app and a headless render node share it.

pub mod align;
pub mod animation;
pub mod audio;
pub mod bake;
pub mod captions;
pub mod copy;
pub mod cutout;
pub mod cut;
pub mod edit;
pub mod media;
pub mod effects;
pub mod elements;
pub mod enhance;
pub mod folders;
pub mod gradient;
pub mod grade;
pub mod import;
pub mod library;
pub mod magic_move;
pub mod mockups;
pub mod model;
pub mod morph;
pub mod motion_blur;
pub mod nesting;
pub mod package;
pub mod path;
pub mod playback;
pub mod presets;
pub mod project_file;
pub mod property;
pub mod resize;
pub mod sample;
pub mod silence;
pub mod slots;
pub mod templates;
pub mod text_animation;
pub mod snap;
pub mod speed;
pub mod time;
pub mod tracks;
pub mod transform;
pub mod trim;
pub mod trim_tools;
pub mod transitions;
