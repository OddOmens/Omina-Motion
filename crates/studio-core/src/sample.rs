//! A placeholder project used until projects can be opened from disk.
//! It covers each layer kind so every UI state can be seen during development.

use crate::animation::{Animated, Keyframe, Vec2};
use crate::effects;
use crate::model::{
    BlendMode, Composition, Effect, Id, LabelColor, Layer, LayerContent, LayerSwitches, Project,
    ProjectItem, ProjectItemKind, Rgba, TextStyle, TrackMatte, Transform,
};
use crate::time::FrameRate;

/// Fixed ids for sample objects, so tests can refer to them.
pub mod ids {
    use crate::model::Id;
    pub const FOOTAGE_FOLDER: Id = Id(1);
    pub const SOLIDS_FOLDER: Id = Id(2);
    pub const MAIN_COMP: Id = Id(3);
    pub const A_CAM: Id = Id(4);
    pub const B_ROLL: Id = Id(5);
    pub const LOGO: Id = Id(6);
    pub const MUSIC: Id = Id(7);
    pub const GRAY_SOLID: Id = Id(8);
}

const FPS_24: FrameRate = FrameRate::new(24, 1);
const COMP_SIZE: Vec2 = Vec2::new(1920.0, 1080.0);

fn keys<T: crate::animation::Lerp>(frames: &[(i64, T)]) -> Animated<T> {
    Animated::keyframed(frames.iter().map(|&(frame, value)| Keyframe::linear(frame, value)).collect())
}

/// A layer centered in the composition with default settings; callers adjust what differs.
fn layer(id: u64, name: &str, label: LabelColor, content: LayerContent, size: Vec2, frames: (i64, i64)) -> Layer {
    let has_picture = content.has_picture();
    Layer {
        id: Id(id),
        name: name.into(),
        label,
        content,
        size,
        in_frame: frames.0,
        out_frame: frames.1,
        start_frame: frames.0,
        track: 0,
        blend_mode: BlendMode::Normal,
        track_matte: TrackMatte::None,
        parent: None,
        switches: LayerSwitches::default(),
        transform: has_picture.then(|| Transform::centered(size, COMP_SIZE * 0.5)),
        effects: Vec::new(),
        has_audio: false,
    }
}

/// A catalog effect with some parameters changed or animated.
fn effect(id: u64, name: &str, overrides: Vec<(usize, Animated<f32>)>) -> Effect {
    let mut effect = effects::instantiate(effects::find(name).expect("sample effects exist in the catalog"), Id(id));
    for (parameter, value) in overrides {
        effect.properties[parameter].value = value;
    }
    effect
}

fn item(id: Id, name: &str, kind: ProjectItemKind, label: LabelColor, parent: Option<Id>) -> ProjectItem {
    ProjectItem {
        id,
        name: name.into(),
        kind,
        label,
        parent,
        size: None,
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: None,
    }
}

pub fn sample_project() -> Project {
    let gray = Rgba::hex(0x3a3a3a);

    let mut title = layer(
        100,
        "Title",
        LabelColor::Red,
        LayerContent::Text { text: "STUDIO".into(), style: TextStyle::default() },
        TextStyle::default().estimate_size("STUDIO"),
        (24, 168),
    );
    if let Some(transform) = title.transform.as_mut() {
        transform.position = keys(&[(24, Vec2::new(960.0, 620.0)), (48, Vec2::new(960.0, 540.0))]);
        transform.opacity = keys(&[(24, 0.0), (36, 100.0), (156, 100.0), (168, 0.0)]);
    }

    let mut grade = layer(101, "Grade", LabelColor::Lavender, LayerContent::Adjustment, COMP_SIZE, (0, 240));
    grade.switches.adjustment = true;
    grade.effects.push(effect(200, "Color Balance", vec![]));

    let mut logo = layer(
        102,
        "logo.png",
        LabelColor::Peach,
        LayerContent::Image { item: ids::LOGO },
        Vec2::new(1200.0, 400.0),
        (48, 240),
    );
    if let Some(transform) = logo.transform.as_mut() {
        transform.position = Animated::constant(Vec2::new(960.0, 300.0));
        transform.scale = keys(&[(48, Vec2::new(40.0, 40.0)), (72, Vec2::new(50.0, 50.0))]);
    }
    logo.effects.push(effect(201, "Glow", vec![(1, Animated::constant(25.0)), (2, keys(&[(60, 1.0), (120, 2.0)]))]));

    let mut b_roll =
        layer(103, "B-roll_02.mov", LabelColor::Aqua, LayerContent::Footage { item: ids::B_ROLL }, COMP_SIZE, (120, 240));
    b_roll.has_audio = true;
    b_roll.switches.audio = true;

    let mut a_cam = layer(
        104,
        "A-cam_01.mov",
        LabelColor::Aqua,
        LayerContent::Footage { item: ids::A_CAM },
        Vec2::new(3840.0, 2160.0),
        (0, 132),
    );
    a_cam.has_audio = true;
    a_cam.switches.audio = true;
    if let Some(transform) = a_cam.transform.as_mut() {
        transform.scale = Animated::constant(Vec2::new(50.0, 50.0));
    }
    a_cam.effects.push(effect(202, "Gaussian Blur", vec![(0, keys(&[(108, 0.0), (132, 40.0)]))]));

    let background =
        layer(105, "Dark Gray Solid 1", LabelColor::Red, LayerContent::Solid { color: gray }, COMP_SIZE, (0, 240));

    let composition = Composition {
        id: ids::MAIN_COMP,
        name: "Main Comp".into(),
        width: 1920,
        height: 1080,
        frame_rate: FPS_24,
        duration_frames: 240,
        background: Rgba::hex(0x000000),
        layers: vec![title, grade, logo, b_roll, a_cam, background],
    };

    let footage = Some(ids::FOOTAGE_FOLDER);
    let items = vec![
        item(ids::FOOTAGE_FOLDER, "Footage", ProjectItemKind::Folder, LabelColor::None, None),
        item(ids::SOLIDS_FOLDER, "Solids", ProjectItemKind::Folder, LabelColor::None, None),
        ProjectItem {
            size: Some(COMP_SIZE),
            duration_seconds: Some(10.0),
            frame_rate: Some(FPS_24),
            ..item(ids::MAIN_COMP, "Main Comp", ProjectItemKind::Composition, LabelColor::None, None)
        },
        ProjectItem {
            size: Some(Vec2::new(3840.0, 2160.0)),
            duration_seconds: Some(5.5),
            frame_rate: Some(FrameRate::new(24000, 1001)),
            has_audio: true,
            ..item(ids::A_CAM, "A-cam_01.mov", ProjectItemKind::Footage, LabelColor::Aqua, footage)
        },
        ProjectItem {
            size: Some(COMP_SIZE),
            duration_seconds: Some(12.0),
            frame_rate: Some(FPS_24),
            has_audio: true,
            ..item(ids::B_ROLL, "B-roll_02.mov", ProjectItemKind::Footage, LabelColor::Aqua, footage)
        },
        ProjectItem {
            size: Some(Vec2::new(1200.0, 400.0)),
            ..item(ids::LOGO, "logo.png", ProjectItemKind::Image, LabelColor::Peach, footage)
        },
        ProjectItem {
            duration_seconds: Some(151.0),
            has_audio: true,
            ..item(ids::MUSIC, "score.wav", ProjectItemKind::Audio, LabelColor::Green, footage)
        },
        ProjectItem {
            size: Some(COMP_SIZE),
            color: Some(gray),
            ..item(ids::GRAY_SOLID, "Dark Gray Solid 1", ProjectItemKind::Solid, LabelColor::Red, Some(ids::SOLIDS_FOLDER))
        },
    ];

    let mut composition = composition;
    crate::tracks::assign_tracks_by_stack(&mut composition);
    Project::new(items, vec![composition])
}
