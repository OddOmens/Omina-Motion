//! The user's own LUTs. Importing a `.cube` file checks that it reads as a lookup table, then
//! copies it into the `LUTs` folder in Studio's settings folder, so it stays available (in the
//! Effects panel and the Color panel) in every project even if the original is moved.

use std::path::{Path, PathBuf};

/// Where imported LUTs are kept.
pub fn folder() -> PathBuf {
    crate::app_settings::folder().join("LUTs")
}

/// The imported LUTs, by name.
pub fn list() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder()) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| is_cube(path)).collect();
    found.sort_by_key(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()));
    found
}

/// Whether `path` names a `.cube` file (by its extension).
pub fn is_cube(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("cube"))
}

/// Copies the LUT at `source` into the library and returns where it now is. A LUT already in
/// the library is returned as it is; one whose name is taken by a different file gets a number
/// added ("Film 2.cube"). Files that are not readable LUTs are refused, saying why.
pub fn import(source: &Path) -> Result<PathBuf, String> {
    let name = source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    if !is_cube(source) {
        return Err(format!("\"{name}\" is not a .cube LUT."));
    }
    studio_render::read_cube(source).map_err(|problem| format!("\"{name}\" can't be used as a LUT: {problem}."))?;
    let folder = folder();
    if source.parent() == Some(folder.as_path()) {
        return Ok(source.to_path_buf());
    }
    std::fs::create_dir_all(&folder).map_err(|error| format!("Could not make the LUTs folder: {error}"))?;
    let bytes = std::fs::read(source).map_err(|error| format!("Could not read \"{name}\": {error}"))?;
    let stem = source.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "LUT".into());
    for number in 1.. {
        let candidate = folder.join(if number == 1 { format!("{stem}.cube") } else { format!("{stem} {number}.cube") });
        match std::fs::read(&candidate) {
            // The same table imported again: use the copy already there.
            Ok(existing) if existing == bytes => return Ok(candidate),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&candidate, &bytes).map_err(|error| format!("Could not copy \"{name}\" into the LUTs folder: {error}"))?;
                return Ok(candidate);
            }
        }
    }
    unreachable!("some numbered name is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_cube(folder: &Path, name: &str, lut: &studio_core::grade::Lut) -> PathBuf {
        std::fs::create_dir_all(folder).unwrap();
        let path = folder.join(name);
        std::fs::write(&path, lut.to_cube()).unwrap();
        path
    }

    #[test]
    fn luts_are_checked_copied_in_and_listed() {
        let _ = std::fs::remove_dir_all(folder());
        let outside = std::env::temp_dir().join(format!("studio-lut-import-{}", std::process::id()));
        let identity = studio_core::grade::Lut::from_fn("Identity", 5, |c| c);
        let swap = studio_core::grade::Lut::from_fn("Swap", 5, |[r, g, b]| [b, g, r]);

        let film = write_cube(&outside, "Film.cube", &identity);
        let copied = import(&film).unwrap();
        assert_eq!(copied, folder().join("Film.cube"));
        assert_eq!(import(&film).unwrap(), copied, "the same file again is not copied twice");
        // A different table with the same name gets a number.
        let other = write_cube(&outside.join("other"), "Film.cube", &swap);
        assert_eq!(import(&other).unwrap(), folder().join("Film 2.cube"));
        assert_eq!(list(), vec![folder().join("Film 2.cube"), folder().join("Film.cube")]);

        std::fs::write(outside.join("Broken.cube"), "LUT_3D_SIZE 4\n0 0 0\n").unwrap();
        assert!(import(&outside.join("Broken.cube")).unwrap_err().contains("can't be used"));
        assert!(import(&outside.join("photo.png")).unwrap_err().contains("not a .cube"));
        let _ = std::fs::remove_dir_all(outside);
        let _ = std::fs::remove_dir_all(folder());
    }
}
