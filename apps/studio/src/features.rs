//! Tools and features that can be turned off (Settings ▸ Features), so Studio shows only what
//! someone uses. A feature that is off is hidden everywhere it would appear: the tool bar, the
//! rail, the menu bar, right-click menus and the Properties panel. Its keyboard shortcut does
//! nothing. Whatever it already made stays in the project and keeps working.
//!
//! A few things are always on, because Studio can't be used without them: New Composition,
//! Import, Settings, the Select tool, the Hand tool and the Media panel (where imports go).
//!
//! The four AI features keep their own switches in `AiFeatures`, so they read the same on the
//! AI Models page and here.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::app_settings::AppSettings;
use crate::workspace::{LeftPanel, Tool};

/// Something that can be turned off. The names are saved in settings, so they never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    // Tools in the top bar.
    TrackSelect,
    CutTool,
    RippleTool,
    RollTool,
    RateStretchTool,
    SlipTool,
    SlideTool,
    ZoomTool,
    TextTool,
    ShapeTool,
    PenTool,
    RotateTool,
    Eyedropper,
    NewLayer,
    // Panels on the rail.
    Libraries,
    Effects,
    Elements,
    Templates,
    Mockups,
    // Editing.
    Precompose,
    PasteAttributes,
    FreezeFrame,
    CloseGaps,
    SceneCuts,
    Speed,
    Stabilize,
    TrackMotion,
    Morph,
    MagicMove,
    MagicResize,
    Markers,
    SequenceLayers,
    GraphEditor,
    Guides,
    RenderAndPlay,
    // Getting work out.
    Export,
    ExportQueue,
    CaptionFiles,
    Package,
    // AI, stored in `AppSettings::ai`.
    Captions,
    RemoveBackground,
    AutoReframe,
    EnhanceSpeech,
}

/// A feature as Settings lists it.
pub struct FeatureInfo {
    pub feature: Feature,
    pub name: &'static str,
    /// Where it shows up, so it's clear what turning it off hides.
    pub about: &'static str,
}

const fn info(feature: Feature, name: &'static str, about: &'static str) -> FeatureInfo {
    FeatureInfo { feature, name, about }
}

/// Every feature that can be turned off, in groups, in the order Settings lists them.
pub const GROUPS: &[(&str, &[FeatureInfo])] = &[
    (
        "Tools",
        &[
            info(Feature::TrackSelect, "Track Select", "Picks a clip and everything after it."),
            info(Feature::CutTool, "Cut", "The razor: click a clip to split it."),
            info(Feature::RippleTool, "Ripple", "Trims a clip and moves the clips after it."),
            info(Feature::RollTool, "Roll", "Moves the cut between two touching clips."),
            info(Feature::RateStretchTool, "Rate Stretch", "Changes a clip's speed by dragging its end."),
            info(Feature::SlipTool, "Slip", "Changes which part of the footage a clip plays."),
            info(Feature::SlideTool, "Slide", "Moves a clip between its neighbors."),
            info(Feature::ZoomTool, "Zoom", "Click the viewer to zoom in or out."),
            info(Feature::TextTool, "Text", "Click the viewer to type a text layer."),
            info(Feature::ShapeTool, "Shape", "Drag in the viewer to draw a shape."),
            info(Feature::PenTool, "Pen", "Draw a shape point by point."),
            info(Feature::RotateTool, "Rotate", "Drag in the viewer to turn layers."),
            info(Feature::Eyedropper, "Eyedropper", "Picks a color from the viewer, with swatches."),
            info(Feature::NewLayer, "New Layer", "Solids, adjustment layers and null objects."),
        ],
    ),
    (
        "Panels",
        &[
            info(Feature::Libraries, "Libraries", "Shared colors and text styles."),
            info(Feature::Effects, "Effects", "The Effects panel on the rail, to drag effects onto clips."),
            info(Feature::Elements, "Elements", "Ready-made shapes, arrows, badges and stickers."),
            info(Feature::Templates, "Templates", "Ready-made animated compositions."),
            info(Feature::Mockups, "Mockups", "Phone, tablet and Mac device frames."),
        ],
    ),
    (
        "Editing",
        &[
            info(Feature::Precompose, "Pre-compose", "Groups layers into a composition of their own."),
            info(Feature::PasteAttributes, "Paste Attributes", "Pastes one clip's effects and settings onto others."),
            info(Feature::FreezeFrame, "Freeze Frame", "Holds a clip on one frame."),
            info(Feature::CloseGaps, "Close Gaps", "Pulls clips back to close the empty space on a track."),
            info(Feature::SceneCuts, "Scene Cuts", "Finds the cuts in footage, to split or mark them."),
            info(Feature::Speed, "Speed and Time Remap", "The Speed section in Properties."),
            info(Feature::Stabilize, "Stabilize", "Takes the shake out of footage."),
            info(Feature::TrackMotion, "Track Motion", "Follows a point in footage so other layers can stick to it."),
            info(Feature::Morph, "Morph", "Turns one shape into another."),
            info(Feature::MagicMove, "Magic Move", "Animates between two compositions."),
            info(Feature::MagicResize, "Magic Resize", "Makes copies of a composition in other sizes."),
            info(Feature::Markers, "Markers", "Adding and removing timeline markers."),
            info(Feature::SequenceLayers, "Sequence and Stagger", "Lines layers up one after another."),
            info(Feature::GraphEditor, "Graph Editor", "Speed curves for keyframes, under the timeline."),
            info(Feature::Guides, "Guides", "Safe areas and grids over the viewer."),
            info(Feature::RenderAndPlay, "Render & Play", "Renders the work area before playing it."),
        ],
    ),
    (
        "Output",
        &[
            info(Feature::Export, "Export", "Exporting movies, from the rail, the File menu and clips."),
            info(Feature::ExportQueue, "Export Queue", "Lining up exports to run one after another."),
            info(Feature::CaptionFiles, "Caption Files", "Saving captions as SRT or WebVTT."),
            info(Feature::Package, "Package", "Collecting a project and its media into one zip."),
        ],
    ),
    (
        AI_GROUP,
        &[
            info(Feature::Captions, "Captions", "Generate Captions, and the Captions panel on the rail."),
            info(Feature::RemoveBackground, "Object Matte", "Painting an object out of footage and images, like Roto Brush."),
            info(Feature::AutoReframe, "Auto Reframe", "New shapes of a clip that follow its subject."),
            info(Feature::EnhanceSpeech, "Enhance Speech", "Cleaning up voice recordings."),
        ],
    ),
];

/// Starting points for the welcome setup's "What will you make?": each turns on every tool and
/// feature except its own short list. The AI features are chosen separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Everything,
    VideoEditing,
    MotionDesign,
    Basics,
}

impl Focus {
    pub const ALL: [Focus; 4] = [Self::Everything, Self::VideoEditing, Self::MotionDesign, Self::Basics];

    pub fn name(self) -> &'static str {
        match self {
            Self::Everything => "Everything",
            Self::VideoEditing => "Video editing",
            Self::MotionDesign => "Motion design",
            Self::Basics => "Just the basics",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Everything => "Every tool and feature Studio has.",
            Self::VideoEditing => "Cutting footage together: trims, speed, stabilizing and scene cuts.",
            Self::MotionDesign => "Animating shapes and text: the pen, morphs, the graph editor.",
            Self::Basics => "Cut, add text and effects, export. Fewer buttons to learn.",
        }
    }

    /// What this starting point turns off.
    pub fn off(self) -> &'static [Feature] {
        use Feature::*;
        match self {
            Self::Everything => &[],
            Self::VideoEditing => &[PenTool, RotateTool, Morph, MagicMove, Mockups, GraphEditor, SequenceLayers],
            Self::MotionDesign => &[TrackSelect, RippleTool, RollTool, RateStretchTool, SlipTool, SlideTool, SceneCuts, CloseGaps, FreezeFrame, Stabilize],
            Self::Basics => &[
                TrackSelect, RippleTool, RollTool, RateStretchTool, SlipTool, SlideTool, ZoomTool, PenTool, RotateTool, Eyedropper, NewLayer, Libraries, Mockups, Precompose, PasteAttributes, SceneCuts,
                Stabilize, TrackMotion, Morph, MagicMove, MagicResize, Markers, SequenceLayers, GraphEditor, Guides, RenderAndPlay, ExportQueue, CaptionFiles, Package,
            ],
        }
    }

    /// Turns every non-AI feature on, except this starting point's list.
    pub fn apply(self, settings: &mut AppSettings) {
        for feature in non_ai_features() {
            settings.set_feature(feature, !self.off().contains(&feature));
        }
    }

    /// The starting point the current choices match exactly, if any.
    pub fn matching(settings: &AppSettings) -> Option<Self> {
        Self::ALL.into_iter().find(|focus| non_ai_features().all(|feature| settings.feature_on(feature) != focus.off().contains(&feature)))
    }
}

/// The group that holds the AI features, which have a page of their own in the welcome setup.
pub const AI_GROUP: &str = "AI";

/// Every feature outside the AI group.
pub fn non_ai_features() -> impl Iterator<Item = Feature> {
    GROUPS.iter().filter(|(group, _)| *group != AI_GROUP).flat_map(|(_, features)| features.iter().map(|info| info.feature))
}

impl Feature {
    /// The setting's id in Settings, for its switch.
    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
    }

    /// The feature a tool belongs to, or `None` for the tools that are always on.
    pub fn of_tool(tool: Tool) -> Option<Self> {
        Some(match tool {
            Tool::Selection | Tool::Hand => return None,
            Tool::TrackSelect => Self::TrackSelect,
            Tool::Cut => Self::CutTool,
            Tool::Ripple => Self::RippleTool,
            Tool::Roll => Self::RollTool,
            Tool::RateStretch => Self::RateStretchTool,
            Tool::Slip => Self::SlipTool,
            Tool::Slide => Self::SlideTool,
            Tool::Zoom => Self::ZoomTool,
            Tool::Type => Self::TextTool,
            Tool::Shape => Self::ShapeTool,
            Tool::Pen => Self::PenTool,
            Tool::Rotate => Self::RotateTool,
            Tool::Eyedropper => Self::Eyedropper,
        })
    }

    /// The feature a rail panel belongs to, or `None` for the Media panel, which is always on.
    pub fn of_panel(panel: LeftPanel) -> Option<Self> {
        Some(match panel {
            LeftPanel::Media => return None,
            LeftPanel::Libraries => Self::Libraries,
            LeftPanel::Effects => Self::Effects,
            LeftPanel::Elements => Self::Elements,
            LeftPanel::Templates => Self::Templates,
            LeftPanel::Mockups => Self::Mockups,
            LeftPanel::Captions => Self::Captions,
        })
    }

    /// The feature a keyboard command belongs to (by its id in `keymap::COMMANDS`), if any.
    pub fn of_command(id: &str) -> Option<Self> {
        Some(match id {
            "track-select-tool" => Self::TrackSelect,
            "cut-tool" => Self::CutTool,
            "ripple-tool" => Self::RippleTool,
            "roll-tool" => Self::RollTool,
            "rate-stretch-tool" => Self::RateStretchTool,
            "slip-tool" => Self::SlipTool,
            "slide-tool" => Self::SlideTool,
            "zoom-tool" => Self::ZoomTool,
            "type-tool" => Self::TextTool,
            "shape-tool" => Self::ShapeTool,
            "pen-tool" | "finish-path" => Self::PenTool,
            "rotate-tool" => Self::RotateTool,
            "eyedropper-tool" => Self::Eyedropper,
            "new-solid" | "new-adjustment-layer" | "new-null" => Self::NewLayer,
            "precompose" => Self::Precompose,
            "paste-attributes" => Self::PasteAttributes,
            "close-gaps" => Self::CloseGaps,
            "toggle-marker" => Self::Markers,
            "toggle-guides" => Self::Guides,
            "render-and-play" => Self::RenderAndPlay,
            "export" => Self::Export,
            _ => return None,
        })
    }
}

impl AppSettings {
    /// Whether `feature` is on.
    pub fn feature_on(&self, feature: Feature) -> bool {
        match feature {
            Feature::Captions => self.ai.captions,
            Feature::RemoveBackground => self.ai.remove_background,
            Feature::AutoReframe => self.ai.auto_reframe,
            Feature::EnhanceSpeech => self.ai.enhance_speech,
            // The queue only runs exports, so it goes with them.
            Feature::ExportQueue => self.feature_on(Feature::Export) && !self.features_off.contains(&feature),
            _ => !self.features_off.contains(&feature),
        }
    }

    /// Turns `feature` on or off.
    pub fn set_feature(&mut self, feature: Feature, on: bool) {
        match feature {
            Feature::Captions => self.ai.captions = on,
            Feature::RemoveBackground => self.ai.remove_background = on,
            Feature::AutoReframe => self.ai.auto_reframe = on,
            Feature::EnhanceSpeech => self.ai.enhance_speech = on,
            _ if on => {
                self.features_off.remove(&feature);
            }
            _ => {
                self.features_off.insert(feature);
            }
        }
    }

    /// Whether a keyboard command can be used now (its feature, if it has one, is on).
    pub fn command_on(&self, id: &str) -> bool {
        Feature::of_command(id).is_none_or(|feature| self.feature_on(feature))
    }

    /// Whether a tool can be picked now.
    pub fn tool_on(&self, tool: Tool) -> bool {
        Feature::of_tool(tool).is_none_or(|feature| self.feature_on(feature))
    }

    /// Whether a rail panel is shown now.
    pub fn panel_on(&self, panel: LeftPanel) -> bool {
        Feature::of_panel(panel).is_none_or(|feature| self.feature_on(feature))
    }
}

/// Reads the saved list of features that are off, skipping any name this copy of Studio
/// doesn't know (one from a newer version, say) instead of losing the whole list.
pub fn read_features_off<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeSet<Feature>, D::Error> {
    let names = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(names.into_iter().filter_map(|name| serde_json::from_value(name).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_is_listed_once() {
        let listed: Vec<Feature> = GROUPS.iter().flat_map(|(_, features)| features.iter().map(|info| info.feature)).collect();
        let unique: BTreeSet<Feature> = listed.iter().copied().collect();
        assert_eq!(listed.len(), unique.len(), "no feature is listed twice");
        // Adding a variant without listing it would leave it with no switch.
        assert_eq!(unique.len(), 43);
    }

    #[test]
    fn turning_features_off_and_on_is_remembered_by_name() {
        let mut settings = AppSettings::default();
        assert!(settings.feature_on(Feature::PenTool));
        settings.set_feature(Feature::PenTool, false);
        settings.set_feature(Feature::Captions, false);
        assert!(!settings.feature_on(Feature::PenTool));
        assert!(!settings.command_on("finish-path"), "the pen's other command goes with it");
        assert!(!settings.tool_on(Tool::Pen));
        assert!(settings.tool_on(Tool::Hand), "the Hand tool is always on");
        assert!(!settings.ai.captions, "AI features share their switches with the AI Models page");
        assert!(!settings.panel_on(LeftPanel::Captions));

        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("\"pen-tool\""));
        let read: AppSettings = serde_json::from_str(&text).unwrap();
        assert!(!read.feature_on(Feature::PenTool));

        settings.set_feature(Feature::PenTool, true);
        assert!(settings.feature_on(Feature::PenTool));
    }

    #[test]
    fn turning_export_off_takes_the_queue_with_it() {
        let mut settings = AppSettings::default();
        settings.set_feature(Feature::Export, false);
        assert!(!settings.feature_on(Feature::ExportQueue));
        settings.set_feature(Feature::Export, true);
        assert!(settings.feature_on(Feature::ExportQueue));
    }

    #[test]
    fn starting_points_set_the_non_ai_features_and_are_recognized() {
        let mut settings = AppSettings::default();
        assert_eq!(Focus::matching(&settings), Some(Focus::Everything));
        settings.set_feature(Feature::Captions, false);
        Focus::Basics.apply(&mut settings);
        assert_eq!(Focus::matching(&settings), Some(Focus::Basics));
        assert!(!settings.feature_on(Feature::PenTool) && settings.feature_on(Feature::CutTool) && settings.feature_on(Feature::Export));
        assert!(!settings.feature_on(Feature::Captions), "AI choices are left alone");
        settings.set_feature(Feature::PenTool, true);
        assert_eq!(Focus::matching(&settings), None, "a hand-picked mix matches none");
        Focus::Everything.apply(&mut settings);
        assert!(settings.features_off.is_empty());
    }

    #[test]
    fn an_unknown_name_in_the_saved_list_is_skipped() {
        let read: AppSettings = serde_json::from_str(r#"{"features_off": ["pen-tool", "a-feature-from-the-future"]}"#).unwrap();
        assert!(!read.feature_on(Feature::PenTool));
        assert!(read.feature_on(Feature::CutTool));
    }
}
