//! The part of the workspace a framework module reads and answers for: the
//! directory of its project's `composer.lock`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::filter::SKIPPED_DIR_NAMES;

/// How deep below the root detection looks for `composer.lock`: a project whose
/// Drupal site sits in `src/drupal` keeps its lock at depth 3.
const LOCK_DEPTH: usize = 4;

/// A framework module's project: a directory, without the projects of other active
/// modules inside it, with the directories symlinks inside it lead to elsewhere
/// under the root, as a Composer path repository links a module. Paths are
/// canonical.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub dir: PathBuf,
    pub excluded: Vec<PathBuf>,
    pub linked: Vec<PathBuf>,
}

impl Project {
    /// Whether `path` is inside the project, comparing whole path components. A
    /// linked directory wins over a wider excluded one that holds it.
    pub fn contains(&self, path: &Path) -> bool {
        let depth = |dirs: &[PathBuf]| {
            dirs.iter()
                .filter(|dir| path.starts_with(dir))
                .map(|dir| dir.components().count())
                .max()
        };
        let linked = depth(&self.linked);
        if !path.starts_with(&self.dir) && linked.is_none() {
            return false;
        }
        match (linked, depth(&self.excluded)) {
            (_, None) => true,
            (Some(linked), Some(excluded)) => linked > excluded,
            (None, Some(_)) => false,
        }
    }
}

/// A `composer.lock` found below the root, with the packages it lists.
pub(crate) struct Lock {
    pub dir: PathBuf,
    packages: HashSet<String>,
}

impl Lock {
    /// Whether the lock lists `package`.
    pub(crate) fn requires(&self, package: &str) -> bool {
        self.packages.contains(package)
    }
}

/// The `composer.lock` files below `root`, outside hidden directories, `vendor` and
/// `node_modules`, nearest first and in path order at the same depth, each read
/// once. A lock that cannot be read lists no package.
pub(crate) fn locks(root: &Path) -> Vec<Lock> {
    let mut found: Vec<(usize, PathBuf)> = WalkBuilder::new(root)
        .standard_filters(false)
        .hidden(true)
        .max_depth(Some(LOCK_DEPTH))
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !(is_dir
                && (entry.file_name() == "vendor"
                    || SKIPPED_DIR_NAMES
                        .iter()
                        .any(|name| entry.file_name() == *name)))
        })
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name() == "composer.lock" && entry.file_type().is_some_and(|t| t.is_file())
        })
        .map(|entry| (entry.depth(), entry.into_path()))
        .collect();
    found.sort();
    found
        .into_iter()
        .filter_map(|(_, lock)| {
            Some(Lock {
                packages: packages(&lock),
                dir: lock.parent()?.to_path_buf(),
            })
        })
        .collect()
}

/// The names of the packages the `composer.lock` at `path` lists.
fn packages(path: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read(path) else {
        return HashSet::new();
    };
    let Ok(lock) = serde_json::from_slice::<serde_json::Value>(&text) else {
        return HashSet::new();
    };
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|section| lock[section].as_array())
        .flatten()
        .filter_map(|package| package["name"].as_str().map(str::to_string))
        .collect()
}
