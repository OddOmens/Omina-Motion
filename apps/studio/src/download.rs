//! Downloading AI models. Each file is fetched with the system's `curl` (on every Mac, and on
//! Windows 10 and most Linux systems) to a temporary name, checked against the SHA-256 it was
//! published with, and only then put in place, so a broken or tampered download is never used.

use std::path::Path;

/// Downloads `url` to `destination` if its SHA-256 is `sha256` (lowercase hex). `what` names
/// the file in error messages ("the captions model").
pub fn fetch_checked(url: &str, destination: &Path, sha256: &str, what: &str) -> Result<(), String> {
    use sha2::Digest as _;
    let partial = destination.with_extension("part");
    let status = std::process::Command::new("curl").args(["-sSfL", "-o"]).arg(&partial).arg(url).status().map_err(|error| format!("could not start the download: {error}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("{what} could not be downloaded. Check the internet connection and try again."));
    }
    let bytes = std::fs::read(&partial).map_err(|error| format!("could not read the download: {error}"))?;
    let digest: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    if digest != sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("the download of {what} was not the expected file, so it was not used."));
    }
    std::fs::rename(&partial, destination).map_err(|error| format!("could not save {what}: {error}"))
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
        let wrong = fetch_checked(&url, &destination, &"0".repeat(64), "the test model");
        assert!(wrong.is_err() && !destination.exists() && !destination.with_extension("part").exists(), "a file that doesn't match is thrown away");
        fetch_checked(&url, &destination, &digest, "the test model").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"weights");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
