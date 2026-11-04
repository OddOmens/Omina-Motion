//! Embedded assets. Only the Lucide icons the editor uses are bundled, which keeps
//! the binary small; anything else falls back to the component library's defaults.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

// Every icon referenced as `IconName::...` in this crate must be listed here,
// or it renders blank.
icon_assets!(EditorIcons, [ALargeSmall, AlignCenterHorizontal, Aperture, AlignCenterVertical, AlignEndHorizontal, AlignEndVertical, AlignHorizontalDistributeCenter, AlignHorizontalDistributeEnd, AlignHorizontalDistributeStart, AlignStartHorizontal, AlignStartVertical, AlignVerticalDistributeCenter, AlignVerticalDistributeEnd, AlignVerticalDistributeStart, ArrowLeftRight, ArrowRight, ArrowRightToLine, ArrowUpDown, AudioLines, Blend, Brush, Captions, ChartColumn, Check, ChevronDown, ChevronLeft, ChevronRight, ChevronsRight, Circle, CircleDashed, CircleDot, Clapperboard, Columns2, Contrast, Crop, Crosshair, Diamond, Download, Eraser, Eye, EyeOff, FileInput, FileVideoCamera, Film, Flag, Folder, FolderOpen, FolderPlus, Gauge, Ghost, Grid3x3, Hand, Hexagon, Hourglass, Image, Info, Layers, LayoutTemplate, Library, Link, Lock, LockOpen, Magnet, Menu, Mountain, MountainSnow, MousePointer2, Move, MoveHorizontal, Music, Palette, Pause, PenLine, PenTool, Pencil, Pin, Pipette, Play, Rows3, Plus, RectangleHorizontal, Repeat, RotateCcw, RotateCw, Scan, Scissors, Settings, Shapes, SkipBack, SkipForward, Slash, Sparkles, Spline, Square, SquareDashed, SquareFunction, SquarePlay, Stamp, Star, StepBack, Sticker, StepForward, Sun, TabletSmartphone, Tag, TextAlignCenter, TextAlignEnd, TextAlignStart, Timer, Trash, Triangle, Type, Unlink, VenetianMask, Volume2, VolumeX, WandSparkles, Wind, X, ZoomIn, ZoomOut]);

/// The editor's icons first, then the component library's own (chevrons, checkmarks, carets).
pub struct StudioAssets;

impl AssetSource for StudioAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match EditorIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = EditorIcons.list(path)?;
        paths.extend(Assets.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Collects every `IconName::Name` written in the crate's source files.
    fn icons_used_in(dir: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("source directory is readable") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                icons_used_in(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("assets.rs") {
                let source = std::fs::read_to_string(&path).expect("source file is readable");
                for (index, _) in source.match_indices("IconName::") {
                    let name: String = source[index + "IconName::".len()..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    found.insert(name);
                }
            }
        }
    }

    #[test]
    fn every_icon_used_is_bundled() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut used = BTreeSet::new();
        icons_used_in(&src, &mut used);
        let assets = std::fs::read_to_string(src.join("assets.rs")).expect("assets.rs is readable");
        let list_start = assets.find("icon_assets!(EditorIcons, [").expect("icon list") + "icon_assets!(EditorIcons, [".len();
        let list_end = list_start + assets[list_start..].find(']').expect("end of icon list");
        let bundled: BTreeSet<String> = assets[list_start..list_end].split(',').map(|name| name.trim().to_owned()).collect();
        let missing: Vec<&String> = used.difference(&bundled).collect();
        assert!(missing.is_empty(), "icons used but not bundled (they would render blank): {missing:?}");
    }
}
