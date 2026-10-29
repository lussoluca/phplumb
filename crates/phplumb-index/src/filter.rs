//! The rules that decide which files belong to the index.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::composer::VendorFilter;
use crate::discover::DiscoverOptions;
use crate::frameworks::{self, Framework, Project};

/// The extension of PHP source files in every project.
pub const PHP_EXTENSION: &str = "php";

/// Whether `path` is a YAML file, which the engine does not read as PHP: only the
/// frameworks answer definitions in it.
pub fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
}

/// Whether `path` is a Twig template.
pub fn is_twig(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("twig"))
}

/// Whether `path` is a Doctrine XML mapping file.
pub fn is_doctrine_xml(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".orm.xml"))
}

/// Whether `path` is a file the engine does not read as PHP, where only the
/// frameworks answer: YAML, Twig and Doctrine XML mappings.
pub fn is_framework_file(path: &Path) -> bool {
    is_yaml(path) || is_twig(path) || is_doctrine_xml(path)
}

/// Directories never indexed, whatever the exclusions say.
pub(crate) const SKIPPED_DIR_NAMES: &[&str] = &["node_modules"];

/// The rules discovery applies to each file, usable on a single path: PHP extension,
/// no hidden or `node_modules` component, exclude globs, size limit and the Composer
/// vendor filter.
pub struct PathFilter {
    root: PathBuf,
    given_root: PathBuf,
    exclude: GlobSet,
    exclude_patterns: Vec<String>,
    max_file_bytes: u64,
    vendor_filters: HashMap<PathBuf, Option<VendorFilter>>,
    /// Extensions of the files read as PHP.
    extensions: Vec<&'static str>,
    /// The framework project the filter is limited to, if any.
    scope: Option<Project>,
}

impl PathFilter {
    pub fn new(root: &Path, opts: &DiscoverOptions) -> anyhow::Result<PathFilter> {
        Self::for_frameworks(
            root,
            opts,
            &frameworks::detect(root, &opts.frameworks).frameworks(),
        )
    }

    /// The filter for a project whose active frameworks are `active`.
    pub(crate) fn for_frameworks(
        root: &Path,
        opts: &DiscoverOptions,
        active: &[Box<dyn Framework>],
    ) -> anyhow::Result<PathFilter> {
        Ok(PathFilter {
            root: root.canonicalize()?,
            given_root: root.to_path_buf(),
            exclude: build_globset(&opts.exclude)?,
            exclude_patterns: opts.exclude.clone(),
            max_file_bytes: opts.max_file_bytes,
            vendor_filters: HashMap::new(),
            extensions: frameworks::extensions(active),
            scope: None,
        })
    }

    /// The same rules, limited to the files of `project`. A project that covers the
    /// whole root sets no scope, so checks stay as cheap as without one.
    pub(crate) fn scoped(&self, project: &Project) -> PathFilter {
        let whole = project.dir == self.root && project.excluded.is_empty();
        PathFilter {
            root: self.root.clone(),
            given_root: self.given_root.clone(),
            exclude: self.exclude.clone(),
            exclude_patterns: self.exclude_patterns.clone(),
            max_file_bytes: self.max_file_bytes,
            vendor_filters: HashMap::new(),
            extensions: self.extensions.clone(),
            scope: (!whole).then(|| project.clone()),
        }
    }

    /// Where walks start: the project directory, or the root without a project.
    pub(crate) fn start(&self) -> &Path {
        self.scope
            .as_ref()
            .map_or(&self.root, |project| &project.dir)
    }

    /// Whether `path`, under the canonical root, is inside the project, if any.
    pub(crate) fn in_scope(&self, path: &Path) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|project| project.contains(path))
    }

    /// Whether `relative`, a path relative to the root, matches an exclusion.
    pub(crate) fn excludes(&self, relative: &Path) -> bool {
        self.exclude.is_match(relative) || !self.in_scope(&self.root.join(relative))
    }

    /// Whether the user excluded any paths from the index.
    pub fn has_exclusions(&self) -> bool {
        !self.exclude_patterns.is_empty()
    }

    fn has_php_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| self.extensions.contains(&ext))
    }

    /// The canonicalized root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `path` expressed under the canonical root when it was given under the root as
    /// the caller spelled it, for example `/var/...` against `/private/var/...`.
    pub fn under_root(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.given_root) {
            Ok(rest) if self.given_root != self.root => self.root.join(rest),
            _ => path.to_path_buf(),
        }
    }

    /// Whether `path`, an existing file, belongs to the index.
    pub fn accepts(&mut self, path: &Path) -> bool {
        let path = self.under_root(path);
        self.accepts_path(&path)
            && std::fs::metadata(&path)
                .is_ok_and(|meta| meta.is_file() && meta.len() <= self.max_file_bytes)
    }

    /// Whether files inside `dir`, a directory under the canonical root, can belong
    /// to the index: it is inside the root and neither hidden nor `node_modules`.
    pub(crate) fn accepts_dir(&self, dir: &Path) -> bool {
        self.in_scope(dir)
            && dir
                .strip_prefix(&self.root)
                .is_ok_and(|relative| !has_skipped_component(relative))
    }

    /// The path rules alone, without touching the file itself. `path` is under the
    /// canonical root.
    pub(crate) fn accepts_path(&mut self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        if !self.in_scope(path)
            || !self.has_php_extension(path)
            || self.exclude.is_match(relative)
            || has_skipped_component(relative)
        {
            return false;
        }
        let root = &self.root;
        let vendor_filters = &mut self.vendor_filters;
        path.ancestors()
            .skip(1)
            .take_while(|dir| dir.starts_with(root))
            .filter(|dir| dir.file_name().is_some_and(|name| name == "vendor"))
            .all(|vendor| {
                vendor_filters
                    .entry(vendor.to_path_buf())
                    .or_insert_with(|| VendorFilter::load(vendor))
                    .as_ref()
                    .is_none_or(|filter| filter.allows(path))
            })
    }

    pub(crate) fn exclude_patterns(&self) -> &[String] {
        &self.exclude_patterns
    }

    pub(crate) fn max_file_bytes(&self) -> u64 {
        self.max_file_bytes
    }
}

/// Whether a component of `relative` is hidden or a directory never indexed.
pub(crate) fn has_skipped_component(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            name.to_string_lossy().starts_with('.')
                || SKIPPED_DIR_NAMES.iter().any(|skipped| name == *skipped)
        }
        _ => false,
    })
}

/// Invalid patterns are reported on stderr and skipped, so a typo in the settings
/// does not stop indexing.
fn build_globset(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(err) => eprintln!("phplumb: ignoring exclude pattern {pattern:?}: {err}"),
        }
    }
    Ok(builder.build()?)
}
