//! Writing a File ▸ Package… zip. `studio_core::package` plans where each linked file goes; this
//! copies them into `<name>.zip` under a top folder `<name>/`, beside the project saved as
//! `<name>.studio` with its media paths pointing at the copies, and a `Missing files.txt` note
//! when some linked files weren't found. It runs on a background thread (see
//! `panels::package`), reporting progress and stopping when asked to.

use std::fs::File;
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use studio_core::model::Project;
use studio_core::package::{self, PackagedFile};
use studio_core::project_file::{self, EXTENSION, EditorSettings, EditorView};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Everything a package is made from: a snapshot of the project, taken when Package was chosen.
pub struct PackageRequest {
    /// The package's name (already safe as a file name): the zip, its top folder and the
    /// project file inside are all named after it.
    pub name: String,
    /// Where the zip goes.
    pub target: PathBuf,
    pub project: Project,
    /// The saved project's folder, which any relative media path is read against.
    pub project_folder: Option<PathBuf>,
    /// Where the editor was and how it was set up, saved into the packaged project file.
    pub view: EditorView,
    pub settings: EditorSettings,
}

/// How a package ended, when nothing went wrong.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The zip is at the target, without the linked files listed here, which weren't found.
    Finished { missing: Vec<PathBuf> },
    /// Stopped on request; no zip was left behind.
    Cancelled,
}

/// How much is read from a file between progress reports and checks for cancelling.
const CHUNK_BYTES: usize = 1 << 20;

/// The hidden file a package is written to before it is complete, beside the target. Only a
/// finished package takes the target's name, so a half-written zip is never mistaken for one.
pub fn temporary_path(target: &Path) -> PathBuf {
    let name = target.file_name().map_or_else(|| "Package.zip".into(), |name| name.to_string_lossy().into_owned());
    target.with_file_name(format!(".{name}.partial"))
}

/// How many bytes `request` will copy, for the progress bar. Files are looked at again while
/// copying, so this is only an estimate if they change meanwhile.
pub fn total_bytes(request: &PackageRequest) -> u64 {
    plan(request).total_bytes()
}

/// Writes the package `request` describes. `progress` is told how many bytes have been copied so
/// far; `cancelled` is checked often, and when it says so the package stops. Cancelling or
/// failing removes the partial zip.
pub fn write(request: &PackageRequest, progress: &mut dyn FnMut(u64), cancelled: &dyn Fn() -> bool) -> Result<Outcome, String> {
    let partial = temporary_path(&request.target);
    let written = write_zip(request, &partial, progress, cancelled);
    let finished = match written {
        Ok(Outcome::Finished { missing }) => std::fs::rename(&partial, &request.target)
            .map(|()| Outcome::Finished { missing })
            .map_err(|error| format!("The package couldn't be saved as “{}”: {error}", request.target.display())),
        other => other,
    };
    if !matches!(finished, Ok(Outcome::Finished { .. })) {
        let _ = std::fs::remove_file(&partial);
    }
    finished
}

/// Works out what `request` copies, from the files on disk now.
fn plan(request: &PackageRequest) -> package::PackagePlan {
    package::plan(&request.project, request.project_folder.as_deref(), |path| std::fs::metadata(path).ok().filter(|metadata| metadata.is_file()).map(|metadata| metadata.len()))
}

/// Writes the whole zip to `path`.
fn write_zip(request: &PackageRequest, path: &Path, progress: &mut dyn FnMut(u64), cancelled: &dyn Fn() -> bool) -> Result<Outcome, String> {
    let failed = |error: &dyn std::fmt::Display| format!("The package couldn't be written: {error}");
    let plan = plan(request);
    let file = File::create(path).map_err(|error| format!("The package couldn't be saved in “{}”: {error}", path.parent().unwrap_or(path).display()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let root = &request.name;
    let mut missing = plan.missing.clone();
    let mut copied = Vec::new();
    let mut done = 0;
    for file in &plan.files {
        if cancelled() {
            return Ok(Outcome::Cancelled);
        }
        // A file that went away since the plan was made is missing too.
        let Ok(source) = File::open(&file.source) else {
            missing.push(file.source.clone());
            continue;
        };
        zip.start_file(format!("{root}/{}", file.destination), media_options(file, &source)).map_err(|error| failed(&error))?;
        if !copy(source, &mut zip, &mut done, progress, cancelled).map_err(|error| format!("“{}” couldn't be copied: {error}", file.source.display()))? {
            return Ok(Outcome::Cancelled);
        }
        copied.push(file.clone());
    }

    // The project goes in last, so it only points at copies that were made.
    let packaged = package::packaged_project(&request.project, request.project_folder.as_deref(), &copied);
    let text = project_file::to_text_with(&packaged, request.view, &request.settings)?;
    zip.start_file(format!("{root}/{root}.{EXTENSION}"), text_options()).map_err(|error| failed(&error))?;
    zip.write_all(text.as_bytes()).map_err(|error| failed(&error))?;
    if !missing.is_empty() {
        zip.start_file(format!("{root}/{}", package::MISSING_NOTE), text_options()).map_err(|error| failed(&error))?;
        zip.write_all(package::missing_note(root, &missing).as_bytes()).map_err(|error| failed(&error))?;
    }
    let file = zip.finish().map_err(|error| failed(&error))?.into_inner().map_err(|error| failed(&error.error()))?;
    file.sync_all().map_err(|error| failed(&error))?;
    Ok(Outcome::Finished { missing })
}

/// Copies `source` into the zip entry being written, adding to `done` and reporting it as it
/// goes. Returns false if it stopped because the package was cancelled.
fn copy(mut source: File, zip: &mut ZipWriter<BufWriter<File>>, done: &mut u64, progress: &mut dyn FnMut(u64), cancelled: &dyn Fn() -> bool) -> std::io::Result<bool> {
    let mut buffer = vec![0; CHUNK_BYTES];
    loop {
        if cancelled() {
            return Ok(false);
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            return Ok(true);
        }
        zip.write_all(&buffer[..read])?;
        *done += read as u64;
        progress(*done);
    }
}

/// How a linked file is stored. Video, pictures and sound are already compressed, so they are
/// stored as they are, which is much quicker; LUTs are text, so they are compressed. Files of
/// 4 GB or more need the zip's large-file format.
fn media_options(file: &PackagedFile, source: &File) -> SimpleFileOptions {
    let is_lut = file.destination.starts_with(&format!("{}/", package::LUT_FOLDER));
    let method = if is_lut { CompressionMethod::Deflated } else { CompressionMethod::Stored };
    let modified = source.metadata().and_then(|metadata| metadata.modified()).ok();
    SimpleFileOptions::default()
        .compression_method(method)
        .large_file(file.bytes >= u64::from(u32::MAX))
        .last_modified_time(modified.map_or_else(zip::DateTime::default, zip_time))
}

/// How the project file and the missing-files note are stored: compressed, dated now.
fn text_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(zip_time(SystemTime::now()))
}

/// `time` as a zip timestamp (in UTC; zips store no time zone), so unzipped copies keep their
/// files' dates. Times a zip can't hold (before 1980) fall back to the format's earliest date.
fn zip_time(time: SystemTime) -> zip::DateTime {
    let Ok(since_epoch) = time.duration_since(UNIX_EPOCH) else { return zip::DateTime::default() };
    let seconds = since_epoch.as_secs();
    let (year, month, day) = civil_date(seconds / 86_400);
    let of_day = seconds % 86_400;
    let (hour, minute, second) = ((of_day / 3600) as u8, (of_day % 3600 / 60) as u8, (of_day % 60) as u8);
    u16::try_from(year).ok().and_then(|year| zip::DateTime::from_date_and_time(year, month, day, hour, minute, second).ok()).unwrap_or_default()
}

/// The calendar date `days` after 1 January 1970, as (year, month, day). This is Howard
/// Hinnant's `civil_from_days`, which counts in 400-year cycles of the Gregorian calendar.
fn civil_date(days: u64) -> (u64, u8, u8) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u8;
    let month = (if month_index < 10 { month_index + 3 } else { month_index - 9 }) as u8;
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::sample::{ids, sample_project};
    use studio_core::time::FrameRange;

    /// A fresh, empty folder for one test's files.
    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("studio-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// The sample project with A-cam and the logo linked to small files in `folder`, and
    /// B-roll linked to a file that isn't there.
    fn request(folder: &Path, name: &str) -> PackageRequest {
        let mut project = sample_project();
        std::fs::write(folder.join("A-cam_01.mov"), b"a-cam frames").unwrap();
        std::fs::write(folder.join("logo.png"), b"logo pixels").unwrap();
        let mut link = |id, path: PathBuf| project.items.iter_mut().find(|item| item.id == id).unwrap().path = Some(path);
        link(ids::A_CAM, folder.join("A-cam_01.mov"));
        link(ids::LOGO, folder.join("logo.png"));
        link(ids::B_ROLL, folder.join("gone/B-roll_02.mov"));
        let view = EditorView { active_composition: ids::MAIN_COMP, current_frame: 0, work_area: FrameRange::new(0.0, 24.0) };
        PackageRequest { name: name.into(), target: folder.join(format!("{name}.zip")), project, project_folder: None, view, settings: EditorSettings::default() }
    }

    /// The zip's entries, by name, with their contents.
    fn entries(zip: &Path) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(File::open(zip).unwrap()).unwrap();
        let mut entries: Vec<_> = (0..archive.len())
            .map(|index| {
                let mut entry = archive.by_index(index).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (entry.name().to_owned(), bytes)
            })
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn a_package_holds_the_project_its_media_and_a_note_of_what_is_missing() {
        let folder = scratch("write");
        let request = request(&folder, "Promo");
        let mut reported = 0;
        let outcome = write(&request, &mut |done| reported = done, &|| false).unwrap();
        assert_eq!(outcome, Outcome::Finished { missing: vec![folder.join("gone/B-roll_02.mov")] });
        assert_eq!(reported, total_bytes(&request), "progress reaches the total");
        assert!(!temporary_path(&request.target).exists(), "the partial file became the zip");

        let entries = entries(&request.target);
        let names: Vec<_> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["Promo/Footage/A-cam_01.mov", "Promo/Footage/logo.png", "Promo/Missing files.txt", "Promo/Promo.studio"]);
        assert_eq!(entries[0].1, b"a-cam frames");
        let note = String::from_utf8(entries[2].1.clone()).unwrap();
        assert!(note.contains("B-roll_02.mov"), "{note}");
        let (saved, _) = project_file::from_text(std::str::from_utf8(&entries[3].1).unwrap()).unwrap();
        let path = |id| saved.item(id).unwrap().path.clone().unwrap();
        assert_eq!(path(ids::A_CAM), PathBuf::from("Footage/A-cam_01.mov"));
        assert_eq!(path(ids::B_ROLL), folder.join("gone/B-roll_02.mov"), "the missing file keeps its full path");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn cancelling_leaves_no_zip_behind() {
        let folder = scratch("cancel");
        let request = request(&folder, "Cancelled");
        assert_eq!(write(&request, &mut |_| {}, &|| true).unwrap(), Outcome::Cancelled);
        assert!(!request.target.exists());
        assert!(!temporary_path(&request.target).exists(), "the partial file is removed");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_package_that_cannot_be_written_says_why_and_leaves_nothing() {
        let folder = scratch("fail");
        let mut request = request(&folder, "Nowhere");
        request.target = folder.join("no such folder/Nowhere.zip");
        let error = write(&request, &mut |_| {}, &|| false).unwrap_err();
        assert!(error.contains("couldn't be saved"), "{error}");
        assert!(!request.target.exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn zip_dates_follow_the_calendar() {
        // 29 February 2024, 12:34:56 UTC.
        let time = UNIX_EPOCH + std::time::Duration::from_secs(1_709_210_096);
        let date = zip_time(time);
        assert_eq!((date.year(), date.month(), date.day(), date.hour(), date.minute(), date.second()), (2024, 2, 29, 12, 34, 56));
        assert_eq!(civil_date(0), (1970, 1, 1));
        let early = zip_time(UNIX_EPOCH);
        assert_eq!((early.year(), early.month(), early.day()), (1980, 1, 1), "before 1980, the earliest date a zip holds");
    }
}
