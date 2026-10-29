//! Drupal extensions: every module, profile and theme with an `*.info.yml` file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yaml_rust2::YamlLoader;

use super::hooks::ExtensionKind;
use super::services::{Tests, project_drupal_files};
use crate::filter::PathFilter;
use crate::index::Index;

pub(crate) const INFO: &str = ".info.yml";

/// Every module, profile and theme on disk, outside test modules.
#[derive(Default)]
pub(crate) struct Extensions {
    /// Extension directory to its name and kind.
    pub(crate) by_dir: HashMap<PathBuf, (String, ExtensionKind)>,
    pub(crate) kinds: HashMap<String, ExtensionKind>,
    /// Module and profile names, then theme names, longest first.
    pub(crate) modules: Vec<String>,
    pub(crate) themes: Vec<String>,
}

impl Extensions {
    pub(crate) fn read(index: &Index, filter: &PathFilter) -> Extensions {
        let mut extensions = Extensions::default();
        for path in project_drupal_files(index, filter, INFO, Tests::Skip) {
            let Some(kind) = info_kind(&path) else {
                continue;
            };
            let (Some(dir), Some(name)) = (
                path.parent(),
                path.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(INFO)),
            ) else {
                continue;
            };
            extensions
                .by_dir
                .insert(dir.to_path_buf(), (name.to_string(), kind));
            extensions.kinds.insert(name.to_string(), kind);
        }
        for (name, kind) in &extensions.kinds {
            if kind.is_theme() {
                extensions.themes.push(name.clone());
            } else {
                extensions.modules.push(name.clone());
            }
        }
        for names in [&mut extensions.modules, &mut extensions.themes] {
            names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        }
        extensions
    }

    /// The extension a file belongs to: the one of its nearest ancestor directory
    /// holding an `.info.yml`, with that directory.
    pub(crate) fn owner<'a>(&'a self, path: &Path) -> Option<(&'a Path, &'a str, ExtensionKind)> {
        path.ancestors().skip(1).find_map(|dir| {
            self.by_dir
                .get_key_value(dir)
                .map(|(dir, (name, kind))| (dir.as_path(), name.as_str(), *kind))
        })
    }
}

/// The `type` of an `.info.yml` file, when it declares a module, profile or theme.
fn info_kind(path: &Path) -> Option<ExtensionKind> {
    let text = std::fs::read_to_string(path).ok()?;
    let docs = YamlLoader::load_from_str(&text).ok()?;
    match docs.first()?["type"].as_str()? {
        "module" => Some(ExtensionKind::Module),
        "profile" => Some(ExtensionKind::Profile),
        "theme" => Some(ExtensionKind::Theme),
        _ => None,
    }
}
