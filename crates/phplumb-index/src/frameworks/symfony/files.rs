//! The configuration files the Symfony module reads, each read and parsed once
//! until it changes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::filter::is_yaml;
use crate::frameworks::under_any;
use crate::frameworks::yaml::{self, Yaml};

/// A file's text and, for a YAML file, its parsed document.
pub(crate) struct Parsed {
    pub text: String,
    pub yaml: Option<Yaml>,
}

/// The YAML and XML files read so far, by canonical path, so an edit reported at a
/// symlink's target reaches the file read through the link.
#[derive(Default)]
pub(crate) struct ConfigFiles {
    read: Mutex<HashMap<PathBuf, Option<Arc<Parsed>>>>,
}

impl ConfigFiles {
    /// The file at `path`, read on first use; none when it cannot be read.
    pub(crate) fn get(&self, path: &Path) -> Option<Arc<Parsed>> {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut read = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        read.entry(key)
            .or_insert_with(|| {
                let text = std::fs::read_to_string(path).ok()?;
                let yaml = if is_yaml(path) {
                    yaml::parse(&text)
                } else {
                    None
                };
                Some(Arc::new(Parsed { text, yaml }))
            })
            .clone()
    }

    /// Forgets the files at or below each of `paths`, so they are read again.
    pub(crate) fn forget(&mut self, paths: &HashSet<PathBuf>) {
        self.read
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|file, _| !under_any(file, paths));
    }
}
