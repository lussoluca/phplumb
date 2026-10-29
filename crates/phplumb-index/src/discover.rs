//! Finds the PHP files of a workspace.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use ignore::{WalkBuilder, WalkState};

use crate::filter::{PathFilter, SKIPPED_DIR_NAMES, has_skipped_component, is_framework_file};
use crate::frameworks::FrameworkSettings;

pub use crate::filter::PHP_EXTENSION;

pub struct DiscoverOptions {
    /// Globs relative to the root, such as `vendor/rector/rector/**`.
    pub exclude: Vec<String>,
    /// Larger files are skipped: they are almost always generated data.
    pub max_file_bytes: u64,
    /// Which framework modules run.
    pub frameworks: FrameworkSettings,
    /// The directory of the phpstorm stubs that declare PHP's built-in symbols;
    /// none leaves built-in names unresolved.
    pub stubs: Option<PathBuf>,
}

impl Default for DiscoverOptions {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            max_file_bytes: 4 * 1024 * 1024,
            frameworks: FrameworkSettings::default(),
            stubs: crate::stubs::default_dir(),
        }
    }
}

/// Returns the PHP files under `root` as absolute paths under the canonicalized root,
/// sorted, each file once.
///
/// Ignore files such as `.gitignore` are not honored, because projects ignore
/// `vendor` in git while the code there is still what their own code calls. Hidden
/// directories and `node_modules` are skipped. Inside a Composer `vendor` directory only
/// the autoload paths of installed packages are kept. Symlinks are followed; a file
/// reachable through several paths is reported once, under its canonical path when
/// that path is inside the root.
pub fn discover(root: &Path, opts: &DiscoverOptions) -> anyhow::Result<Vec<PathBuf>> {
    let mut filter = PathFilter::new(root, opts)?;
    Ok(discover_with(&mut filter).php)
}

pub(crate) fn discover_with(filter: &mut PathFilter) -> Walked {
    let root = filter.root().to_path_buf();
    walk(filter, &root)
}

/// What a walk below a directory finds, each list sorted.
pub(crate) struct Walked {
    /// The PHP files of the index.
    pub php: Vec<PathBuf>,
    /// The YAML, Twig and Doctrine XML files only the frameworks read, outside
    /// `vendor`.
    pub framework: Vec<PathBuf>,
}

/// A file or a symlinked directory a walk thread met.
enum Seen {
    LinkedDir(PathBuf),
    Php { path: PathBuf, link: bool, len: u64 },
    Framework { path: PathBuf, link: bool },
}

/// Walks `start`, a directory under the filter's root, on every core. Files keep
/// the path they were reached through unless that path goes through a symlink:
/// those are canonicalized, and each file is listed once under the path PHP
/// discovery chooses for it.
pub(crate) fn walk(filter: &mut PathFilter, start: &Path) -> Walked {
    let root = filter.root().to_path_buf();
    let seen: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
    let shared: &PathFilter = filter;
    WalkBuilder::new(start)
        .hidden(true)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .follow_links(true)
        .filter_entry(|entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !(is_dir
                && SKIPPED_DIR_NAMES
                    .iter()
                    .any(|name| entry.file_name() == *name))
        })
        .build_parallel()
        .run(|| {
            let mut batch = Batch {
                found: Vec::new(),
                into: &seen,
            };
            Box::new(move |entry| {
                // Unreadable entries and symlink loops are skipped.
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                let Some(kind) = entry.file_type() else {
                    return WalkState::Continue;
                };
                let link = entry.path_is_symlink();
                if kind.is_dir() {
                    if link {
                        batch.found.push(Seen::LinkedDir(entry.into_path()));
                    }
                } else if kind.is_file() {
                    let path = entry.path();
                    if shared.has_php_extension(path) {
                        if let Ok(metadata) = entry.metadata() {
                            batch.found.push(Seen::Php {
                                len: metadata.len(),
                                path: entry.into_path(),
                                link,
                            });
                        }
                    } else if is_framework_file(path) {
                        batch.found.push(Seen::Framework {
                            path: entry.into_path(),
                            link,
                        });
                    }
                }
                WalkState::Continue
            })
        });

    let seen = seen.into_inner().unwrap_or_else(PoisonError::into_inner);
    let links: HashSet<&Path> = seen
        .iter()
        .filter_map(|seen| match seen {
            Seen::LinkedDir(dir) => Some(dir.as_path()),
            _ => None,
        })
        .collect();
    let start_linked = start
        .canonicalize()
        .map_or(true, |canonical| canonical != start);
    let chosen = |path: &Path, link: bool| -> Option<(PathBuf, PathBuf)> {
        let through_link = start_linked
            || link
            || (!links.is_empty() && path.ancestors().skip(1).any(|a| links.contains(a)));
        if through_link {
            chosen_path(&root, path)
        } else {
            Some((path.to_path_buf(), path.to_path_buf()))
        }
    };
    let mut php: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut framework: HashMap<PathBuf, PathBuf> = HashMap::new();
    for item in &seen {
        let (found, path, link) = match item {
            Seen::LinkedDir(_) => continue,
            Seen::Php { path, link, len } => {
                if *len > filter.max_file_bytes() || !filter.accepts_path(path) {
                    continue;
                }
                (&mut php, path, *link)
            }
            Seen::Framework { path, link } => {
                if !is_recorded_framework_file(filter, path) {
                    continue;
                }
                (&mut framework, path, *link)
            }
        };
        let Some((canonical, chosen)) = chosen(path, link) else {
            continue;
        };
        found
            .entry(canonical)
            .and_modify(|current| {
                if chosen < *current {
                    *current = chosen.clone();
                }
            })
            .or_insert(chosen);
    }
    let sorted = |found: HashMap<PathBuf, PathBuf>| {
        let mut files: Vec<PathBuf> = found.into_values().collect();
        files.sort();
        files
    };
    Walked {
        php: sorted(php),
        framework: sorted(framework),
    }
}

/// A walk thread's findings, added to the shared list when the thread ends.
struct Batch<'a> {
    found: Vec<Seen>,
    into: &'a Mutex<Vec<Seen>>,
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        self.into
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .append(&mut self.found);
    }
}

/// Whether `path`, under the canonical root, is a YAML, Twig or Doctrine XML file
/// the index records for the frameworks: outside `vendor`, hidden directories,
/// `node_modules` and excluded paths.
pub(crate) fn is_recorded_framework_file(filter: &PathFilter, path: &Path) -> bool {
    is_framework_file(path)
        && path.strip_prefix(filter.root()).is_ok_and(|relative| {
            !has_skipped_component(relative)
                && !relative.components().any(|c| c.as_os_str() == "vendor")
                && !filter.excludes(relative)
        })
}

/// The canonical path of `path` and the path the index uses for it: the canonical
/// path when it is inside the root, otherwise the path as reached.
pub(crate) fn chosen_path(root: &Path, path: &Path) -> Option<(PathBuf, PathBuf)> {
    let canonical = path.canonicalize().ok()?;
    let chosen = if canonical.starts_with(root) {
        canonical.clone()
    } else {
        path.to_path_buf()
    };
    Some((canonical, chosen))
}
