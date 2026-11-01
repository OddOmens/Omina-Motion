//! Studio's engine-side core: the project data model, frame-exact time math,
//! keyframe animation, playback timing and project edits. It has no UI or I/O
//! dependencies, so the desktop app and a headless render node share it.

pub mod align;
pub mod animation;
pub mod audio;
pub mod copy;
pub mod cut;
pub mod edit;
pub mod media;
pub mod effects;
pub mod model;
pub mod playback;
pub mod project_file;
pub mod property;
pub mod sample;
pub mod snap;
pub mod time;
pub mod tracks;
pub mod transform;
pub mod trim;
