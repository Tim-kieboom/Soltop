//! Small helpers for reading /sys and /proc files.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Full paths of everything in `dir`; empty when the directory is missing or unreadable.
pub(crate) fn read_dir_paths(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// Contents of a text file with surrounding whitespace removed (sysfs values end in a newline),
/// or `None` when the file cannot be read.
pub(crate) fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
    };

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A fresh, empty directory under the system temp dir, unique per call so tests can run in
    /// parallel. Remove it with `fs::remove_dir_all` when the test is done.
    pub(crate) fn temp_dir(label: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hardware_scanner_{label}_{}_{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::{test_util::temp_dir, *};

    #[test]
    fn read_trimmed_strips_the_trailing_newline() {
        let dir = temp_dir("trim");
        fs::write(dir.join("value"), "  42\n").unwrap();

        assert_eq!(read_trimmed(&dir.join("value")), Some("42".to_string()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_trimmed_of_a_missing_file_is_none() {
        let dir = temp_dir("missing");

        assert_eq!(read_trimmed(&dir.join("nope")), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_dir_paths_lists_every_entry_with_its_full_path() {
        let dir = temp_dir("list");
        fs::write(dir.join("a"), "").unwrap();
        fs::create_dir(dir.join("b")).unwrap();

        let mut paths = read_dir_paths(&dir);
        paths.sort();
        assert_eq!(paths, vec![dir.join("a"), dir.join("b")]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn read_dir_paths_of_a_missing_dir_is_empty() {
        let dir = temp_dir("gone");
        fs::remove_dir_all(&dir).unwrap();

        assert!(read_dir_paths(&dir).is_empty());
    }
}
