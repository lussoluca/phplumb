//! Drupal services: every `*.services.yml` and site `services.yml` on disk outside
//! `vendor` and test modules, and the classes a service id can give.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::discover::chosen_path;
use crate::filter::{PathFilter, SKIPPED_DIR_NAMES, has_skipped_component};
use crate::frameworks::under_any;
use crate::frameworks::yaml::{self, Yaml};
use crate::index::Index;

/// One definition of a service id: where its key is and what it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceDefinition {
    pub path: PathBuf,
    /// The id key in the YAML file.
    pub start: u32,
    pub end: u32,
    pub class: Option<String>,
    pub alias: Option<String>,
    pub parent: Option<String>,
    pub decorates: Option<String>,
}

#[derive(Default)]
pub(crate) struct Services {
    by_file: HashMap<PathBuf, Vec<(String, ServiceDefinition)>>,
    /// Definitions outside test modules, which navigation uses.
    by_id: HashMap<String, Vec<ServiceDefinition>>,
    /// Every id defined, test modules included.
    defined: HashSet<String>,
    root: PathBuf,
    /// Decorated id to the ids decorating it.
    decorators: HashMap<String, Vec<String>>,
}

impl Services {
    /// Every services file below the filter's root.
    pub(crate) fn read(index: &Index, filter: &PathFilter) -> Services {
        let mut services = Services {
            root: filter.root().to_path_buf(),
            ..Services::default()
        };
        for path in drupal_files(index, filter, filter.start(), SERVICES, Tests::Include) {
            services.by_file.insert(path.clone(), parse_file(&path));
        }
        services.reindex();
        services
    }

    /// Re-reads changed services files and directories; drops removed ones.
    pub(crate) fn update(
        &mut self,
        index: &Index,
        filter: &PathFilter,
        changed: &[PathBuf],
        removed: &[PathBuf],
    ) {
        let gone: HashSet<PathBuf> = removed.iter().chain(changed).cloned().collect();
        let before = self.by_file.len();
        self.by_file.retain(|file, _| !under_any(file, &gone));
        let mut touched = self.by_file.len() != before;
        for file in changed_drupal_files(index, filter, changed, SERVICES, Tests::Include) {
            self.by_file.insert(file.clone(), parse_file(&file));
            touched = true;
        }
        if touched {
            self.reindex();
        }
    }

    /// Whether some services file defines `id`, test modules included.
    pub(crate) fn defines(&self, id: &str) -> bool {
        self.defined.contains(id)
    }

    pub(crate) fn definitions(&self, id: &str) -> &[ServiceDefinition] {
        self.by_id.get(id).map(Vec::as_slice).unwrap_or_default()
    }

    /// The classes `id` can give, sorted: see `Drupal::service_classes`.
    pub(crate) fn classes(&self, index: &Index, id: &str) -> Vec<String> {
        let mut found = BTreeSet::new();
        self.collect(index, id, &mut HashSet::new(), &mut found);
        found.into_iter().collect()
    }

    fn collect(
        &self,
        index: &Index,
        id: &str,
        seen: &mut HashSet<String>,
        found: &mut BTreeSet<String>,
    ) {
        if !seen.insert(id.to_string()) {
            return;
        }
        for definition in self.definitions(id) {
            match &definition.alias {
                Some(alias) => self.collect(index, alias, seen, found),
                None => found.extend(self.own_class(index, id, definition, &mut HashSet::new())),
            }
        }
        for decorator in self.decorators.get(id).into_iter().flatten() {
            self.collect(index, decorator, seen, found);
        }
    }

    /// The class a definition declares, takes from its parent, or names as its id.
    fn own_class(
        &self,
        index: &Index,
        id: &str,
        definition: &ServiceDefinition,
        seen: &mut HashSet<String>,
    ) -> Option<String> {
        if let Some(class) = &definition.class {
            return Some(class.clone());
        }
        if let Some(parent) = &definition.parent {
            if !seen.insert(parent.clone()) {
                return None;
            }
            return self
                .definitions(parent)
                .iter()
                .find_map(|p| self.own_class(index, parent, p, seen));
        }
        index.has_class(id).then(|| id.to_string())
    }

    fn reindex(&mut self) {
        self.by_id.clear();
        self.decorators.clear();
        self.defined = self
            .by_file
            .values()
            .flatten()
            .map(|(id, _)| id.clone())
            .collect();
        let mut files: Vec<&PathBuf> = self
            .by_file
            .keys()
            .filter(|file| !in_tests(&self.root, file))
            .collect();
        files.sort();
        for file in files {
            for (id, definition) in &self.by_file[file] {
                if let Some(decorated) = &definition.decorates {
                    self.decorators
                        .entry(decorated.clone())
                        .or_default()
                        .push(id.clone());
                }
                self.by_id
                    .entry(id.clone())
                    .or_default()
                    .push(definition.clone());
            }
        }
    }
}

const SERVICES: &str = ".services.yml";

/// A site's services file, such as `sites/default/services.yml`.
const SITE_SERVICES: &str = "services.yml";

/// Whether the Drupal registries read files of test modules.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tests {
    Skip,
    Include,
}

/// Whether `path`, below the root, is in a test module: a `tests` component.
pub(super) fn in_tests(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|relative| relative.components().any(|c| c.as_os_str() == "tests"))
}

/// Whether `path` is a file named `*{suffix}` (or a site `services.yml`, for services
/// files) that the Drupal registries read:
/// outside `vendor`, hidden or skipped directories, and test modules unless `tests`
/// includes them.
pub(super) fn is_drupal_file(filter: &PathFilter, path: &Path, suffix: &str, tests: Tests) -> bool {
    let name_matches = has_suffix(path, suffix);
    let Ok(relative) = path.strip_prefix(filter.root()) else {
        return false;
    };
    name_matches
        && !filter.excludes(relative)
        && !has_skipped_component(relative)
        && !relative.components().any(|c| {
            let name = c.as_os_str();
            name == "vendor"
                || (tests == Tests::Skip && name == "tests")
                || SKIPPED_DIR_NAMES.iter().any(|s| name == *s)
        })
}

/// The files named `*{suffix}` that the Drupal registries read below `start`, a
/// directory under the filter's root, from the files the index records, each under
/// the path PHP discovery chose for it.
pub(super) fn drupal_files(
    index: &Index,
    filter: &PathFilter,
    start: &Path,
    suffix: &str,
    tests: Tests,
) -> Vec<PathBuf> {
    index
        .framework_files_under(start)
        .filter(|path| is_drupal_file(filter, path, suffix, tests))
        .map(Path::to_path_buf)
        .collect()
}

/// The files named `*{suffix}` the registries read at or below the changed paths,
/// under the paths the index chose for them.
pub(super) fn changed_drupal_files(
    index: &Index,
    filter: &PathFilter,
    changed: &[PathBuf],
    suffix: &str,
    tests: Tests,
) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = changed
        .iter()
        .filter(|path| path.is_dir() || has_suffix(path, suffix))
        .filter_map(|path| chosen_path(filter.root(), path).map(|(_, chosen)| chosen))
        .flat_map(|chosen| drupal_files(index, filter, &chosen, suffix, tests))
        .collect();
    files.sort();
    files.dedup();
    files
}

fn has_suffix(path: &Path, suffix: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(suffix) || (suffix == SERVICES && n == SITE_SERVICES))
}

/// The service definitions of one file, with their ids; nothing when it does not
/// parse.
fn parse_file(path: &Path) -> Vec<(String, ServiceDefinition)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Some(top) = yaml::parse(&text) else {
        return Vec::new();
    };
    let Some(Yaml::Map(services)) = top.get("services") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (key, value) in services {
        let Yaml::Scalar {
            value: id,
            start,
            end,
        } = key
        else {
            continue;
        };
        if id.starts_with('_') {
            continue;
        }
        let (start, end) = yaml::value_range(&text, id, *start, *end);
        let mut definition = ServiceDefinition {
            path: path.to_path_buf(),
            start,
            end,
            class: None,
            alias: None,
            parent: None,
            decorates: None,
        };
        match value {
            Yaml::Scalar { value, .. } => {
                definition.alias = value.strip_prefix('@').map(str::to_string);
            }
            Yaml::Map(entries) => {
                for (key, value) in entries {
                    let Some(text) = value.text() else {
                        continue;
                    };
                    let text = text.trim_start_matches('\\').to_string();
                    match key.text() {
                        Some("class") => definition.class = Some(text),
                        Some("alias") => definition.alias = Some(text),
                        Some("parent") => definition.parent = Some(text),
                        Some("decorates") => definition.decorates = Some(text),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        found.push((id.clone(), definition));
    }
    found
}
