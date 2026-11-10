//! Sound processing building blocks for Studio. `filters` holds the biquad filters (from the
//! Audio EQ Cookbook) used by the EQ, hum notches and de-esser; `effects` holds the audio
//! effects a clip can have, which run block by block while the mix plays or exports.

mod effects;
mod filters;

pub use effects::{AudioEffect, ECHO_MAX_SECONDS};
pub use filters::Biquad;
