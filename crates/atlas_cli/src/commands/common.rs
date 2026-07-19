#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use crate::error::CliError;

/// Bounds and default applied to every `--limit` flag before hitting the API.
pub(crate) const LIMIT_MIN: u32 = 1;
pub(crate) const LIMIT_MAX: u32 = 200;
pub(crate) const LIMIT_DEFAULT: u32 = 20;

/// Reads a file for attachment upload and derives the filename from the path.
pub(crate) fn read_upload_file(path: &std::path::Path) -> Result<(String, Vec<u8>), CliError> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_owned();
    let data = std::fs::read(path)?;
    Ok((filename, data))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_upload_file_returns_io_error_for_nonexistent_path() {
        let err = read_upload_file(std::path::Path::new(
            "/nonexistent/definitely/missing/file.txt",
        ))
        .unwrap_err();
        assert!(
            matches!(err, CliError::Io(_)),
            "missing file must yield CliError::Io"
        );
    }

    #[test]
    fn read_upload_file_reads_content_and_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upload.bin");
        std::fs::write(&path, b"payload").unwrap();

        let (filename, data) = read_upload_file(&path).unwrap();

        assert_eq!(filename, "upload.bin");
        assert_eq!(data, b"payload");
    }
}
