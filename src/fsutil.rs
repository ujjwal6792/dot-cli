//! Atomic file replacement: readers never observe a half-written file.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

/// Write `data` to a sibling temporary file, flush it, then rename over `path`.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = parent.join(format!(".{name}.tmp.{}", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data.as_ref())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_content_and_leaves_no_temporary_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/f.toml");
        write_atomic(&p, "one").unwrap();
        write_atomic(&p, "two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["f.toml"]);
    }

    #[test]
    fn a_failed_write_keeps_the_old_content() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        write_atomic(&p, "old").unwrap();
        // The destination is a directory, so the final rename fails.
        let dir = d.path().join("dir");
        fs::create_dir(&dir).unwrap();
        assert!(write_atomic(&dir, "x").is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "old");
        assert!(
            fs::read_dir(d.path())
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().contains(".tmp."))
        );
    }
}
