//! Small helpers for reading /sys and /proc files.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn read_dir_paths(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

pub(crate) fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
