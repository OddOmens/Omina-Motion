//! Compositions sent from Canvas (its File ▸ Bring to Motion). Canvas writes each one into a
//! folder of its own inside "From Canvas" in Studio's settings folder: a Lottie animation with
//! its pictures beside it, and finally an empty `ready` file. Studio checks for ready folders
//! while it runs (and when it starts) and imports their animation like any Lottie file, so the
//! composition opens here with one layer per Canvas layer.
//!
//! Taking the `ready` file away marks a folder as brought in. The folder itself stays, since
//! the imported layers use the pictures in it.

use std::path::{Path, PathBuf};

/// Where Canvas puts compositions for Studio.
pub fn folder() -> PathBuf {
    crate::app_settings::folder().join("From Canvas")
}

/// The animation file in a package folder: its one `.json` file.
fn animation_in(package: &Path) -> Option<PathBuf> {
    std::fs::read_dir(package).ok()?.filter_map(Result::ok).map(|entry| entry.path()).find(|path| path.extension().is_some_and(|extension| extension == "json"))
}

/// The animations of the packages in `inbox` that are ready and not yet brought in, oldest
/// first. Each one's `ready` file is taken away, so it is handed out only once.
pub fn take_ready(inbox: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(inbox) else { return Vec::new() };
    let mut ready: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|package| {
            let marker = package.join("ready");
            let made = std::fs::metadata(&marker).and_then(|metadata| metadata.modified()).ok()?;
            Some((made, package))
        })
        .collect();
    ready.sort();
    ready
        .into_iter()
        .filter_map(|(_, package)| {
            let animation = animation_in(&package)?;
            // Only hand it out if this is the one taking it.
            std::fs::remove_file(package.join("ready")).ok()?;
            Some(animation)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_packages_are_handed_out_once_oldest_first() {
        let inbox = std::env::temp_dir().join(format!("studio-canvas-inbox-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&inbox);
        let package = |name: &str, ready: bool| {
            let folder = inbox.join(name);
            std::fs::create_dir_all(folder.join("images")).unwrap();
            std::fs::write(folder.join(format!("{name}.json")), "{}").unwrap();
            if ready {
                std::fs::write(folder.join("ready"), "").unwrap();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        package("First", true);
        package("Still writing", false);
        package("Second", true);
        let names: Vec<String> = take_ready(&inbox).iter().map(|path| path.file_stem().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["First", "Second"]);
        assert!(take_ready(&inbox).is_empty(), "each comes in once");
        assert!(inbox.join("First/First.json").exists(), "the pictures' folder stays");
        let _ = std::fs::remove_dir_all(&inbox);
    }

    /// A package as Canvas writes it (see Canvas's `motion.rs`) reads as a composition of
    /// pictures, placed and faded as they were in Canvas, the pictures found beside it.
    #[test]
    fn a_canvas_package_reads_as_a_composition_of_placed_pictures() {
        let text = r#"{"v":"5.7.4","nm":"Poster","fr":30.0,"ip":0,"op":150.0,"w":200,"h":100,"ddd":0,
            "assets":[{"id":"image_0","w":200,"h":100,"u":"images/","p":"layer-0.png","e":0,"nm":"Background"},
                      {"id":"image_1","w":20,"h":10,"u":"images/","p":"layer-1.png","e":0,"nm":"Patch"}],
            "layers":[
              {"ddd":0,"ind":2,"ty":2,"nm":"Patch","refId":"image_1","sr":1,"ks":{"o":{"a":0,"k":50.0},"r":{"a":0,"k":0},"p":{"a":0,"k":[30,40,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},"ao":0,"ip":0,"op":150.0,"st":0,"bm":0,"hd":false},
              {"ddd":0,"ind":1,"ty":2,"nm":"Background","refId":"image_0","sr":1,"ks":{"o":{"a":0,"k":100.0},"r":{"a":0,"k":0},"p":{"a":0,"k":[0,0,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},"ao":0,"ip":0,"op":150.0,"st":0,"bm":0,"hd":true}]}"#;
        let file = Path::new("/inbox/Poster 1/Poster.json");
        let scene = studio_core::import::lottie::read(text, file, Path::new("/unused")).unwrap();
        let comp = &scene.comps[0];
        assert_eq!((comp.width, comp.height, comp.fps, comp.seconds), (200, 100, 30.0, 5.0));
        let names: Vec<&str> = comp.layers.iter().map(|layer| layer.name.as_str()).collect();
        assert_eq!(names, ["Patch", "Background"]);
        let patch = &comp.layers[0];
        assert_eq!(patch.transform.position.as_ref().map(|track| (track.value.x, track.value.y)), Some((30.0, 40.0)));
        assert_eq!(patch.transform.opacity.as_ref().map(|track| track.value), Some(50.0));
        assert!(!comp.layers[1].visible, "a hidden Canvas layer comes in hidden");
        let paths: Vec<PathBuf> = scene.sources.iter().filter_map(|source| match &source.kind {
            studio_core::import::SourceKind::File { path, .. } => Some(path.clone()),
            _ => None,
        }).collect();
        assert!(paths.contains(&PathBuf::from("/inbox/Poster 1/images/layer-1.png")));
    }
}
