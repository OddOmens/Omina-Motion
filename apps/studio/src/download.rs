//! Downloading AI models. Each file is fetched with the system's `curl` (on every Mac, and on
//! Windows 10 and most Linux systems) to a temporary name, checked against the SHA-256 it was
//! published with, and only then put in place, so a broken or tampered download is never used.
//! A download that stalls gives up, and one that is cancelled stops at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The only protocol downloads (and any redirects they follow) may use. Tests fetch local files.
#[cfg(not(test))]
const PROTOCOLS: &str = "=https";
#[cfg(test)]
const PROTOCOLS: &str = "=https,file";

/// How often a running download checks whether it was cancelled.
const CANCEL_CHECK: Duration = Duration::from_millis(100);

/// Where one download is written until it checks out. Each download has its own, so two of the
/// same file at once (Settings and a job, say) never write into each other.
fn partial_path(destination: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = destination.file_name().map_or_else(|| "download".into(), |name| name.to_string_lossy().into_owned());
    destination.with_file_name(format!(".{name}.{}-{}.part", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// Fetches a small text file (at most `max_bytes`) from `url`, such as the update feed. Gives
/// up after 15 seconds, so a slow network never leaves a check hanging.
pub fn fetch_text(url: &str, max_bytes: u64) -> Result<String, String> {
    let output = std::process::Command::new("curl")
        .args(["-q", "-sSfL", "--proto", PROTOCOLS, "--proto-redir", PROTOCOLS, "--connect-timeout", "10", "--max-time", "15", "--max-filesize"])
        .arg(max_bytes.to_string())
        .arg(url)
        .output()
        .map_err(|error| format!("could not start the request: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().trim_start_matches("curl: ").to_owned());
    }
    String::from_utf8(output.stdout).map_err(|_| "the reply was not text".to_owned())
}

/// Downloads `url` to `destination` if its SHA-256 is `sha256` (lowercase hex). `what` names
/// the file in error messages ("the captions model"). Stops, removing what it fetched, when
/// `cancelled` says so.
pub fn fetch_checked(url: &str, destination: &Path, sha256: &str, what: &str, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    use sha2::Digest as _;
    let partial = partial_path(destination);
    let discard = |message: String| {
        let _ = std::fs::remove_file(&partial);
        Err(message)
    };
    // `-q` ignores the user's ~/.curlrc; a connection that can't be made in 20 s, or that moves
    // less than 1 KB a second for a minute, gives up rather than hanging the job.
    let mut curl = std::process::Command::new("curl")
        .args(["-q", "-sSfL", "--proto", PROTOCOLS, "--proto-redir", PROTOCOLS, "--connect-timeout", "20", "--speed-limit", "1024", "--speed-time", "60", "-o"])
        .arg(&partial)
        .arg(url)
        .spawn()
        .map_err(|error| format!("could not start the download: {error}"))?;
    let status = loop {
        if cancelled() {
            let _ = curl.kill();
            let _ = curl.wait();
            return discard("cancelled".into());
        }
        match curl.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(CANCEL_CHECK),
            Err(error) => return discard(format!("the download failed: {error}")),
        }
    };
    if !status.success() {
        return discard(format!("{what} could not be downloaded. Check the internet connection and try again."));
    }
    let bytes = match std::fs::read(&partial) {
        Ok(bytes) => bytes,
        Err(error) => return discard(format!("could not read the download: {error}")),
    };
    let digest: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    if digest != sha256 {
        return discard(format!("the download of {what} was not the expected file, so it was not used."));
    }
    std::fs::rename(&partial, destination).or_else(|error| discard(format!("could not save {what}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_published_file_is_kept() {
        let folder = std::env::temp_dir().join(format!("studio-download-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join("model.bin");
        std::fs::write(&source, b"weights").unwrap();
        let url = format!("file://{}", source.display());
        let digest: String = { use sha2::Digest as _; sha2::Sha256::digest(b"weights").iter().map(|byte| format!("{byte:02x}")).collect() };
        let destination = folder.join("kept.bin");
        let leftovers = || std::fs::read_dir(&folder).unwrap().filter(|entry| entry.as_ref().unwrap().file_name().to_string_lossy().ends_with(".part")).count();
        let wrong = fetch_checked(&url, &destination, &"0".repeat(64), "the test model", &|| false);
        assert!(wrong.is_err() && !destination.exists() && leftovers() == 0, "a file that doesn't match is thrown away");
        assert_eq!(fetch_checked(&url, &destination, &digest, "the test model", &|| true).unwrap_err(), "cancelled");
        assert!(!destination.exists() && leftovers() == 0, "a cancelled download leaves nothing");
        fetch_checked(&url, &destination, &digest, "the test model", &|| false).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"weights");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
