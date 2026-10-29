//! Twig template names: the directories `config/packages/twig.yaml` gives each
//! namespace, in the order TwigBundle adds them to Twig's filesystem loader.

use std::path::{Path, PathBuf};

use super::files::ConfigFiles;
use crate::frameworks::Project;
use crate::frameworks::yaml::Yaml;

/// The placeholder for the project directory in configured paths.
const PROJECT_DIR: &str = "%kernel.project_dir%";

/// TwigBridge's form themes, which FrameworkBundle adds to the main namespace last.
const FORM_THEMES: &str = "vendor/symfony/twig-bridge/Resources/views/Form";

/// Whether a template name resolves to a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateStatus {
    Found(PathBuf),
    /// Every directory of its namespace was read, and none holds it.
    Missing,
    /// Its namespace is not configured, or a directory of it cannot be read.
    Unknown,
}

#[derive(Default)]
pub(crate) struct Templates {
    /// Each directory with its namespace, `None` for the main namespace, in order;
    /// a directory the module cannot read is `None` and ends its namespace.
    paths: Vec<(Option<String>, Option<PathBuf>)>,
    /// Whether Twig configuration the module does not read exists, so any namespace
    /// may have more directories.
    unknown: bool,
    /// Whether an environment adds main-namespace directories or another
    /// `default_path`, which the module leaves out.
    main_unknown: bool,
}

impl Templates {
    /// The `twig.paths` entries of `config/packages/twig.yaml`, then
    /// `default_path` (`templates` by default) when it exists, then TwigBridge's form
    /// themes. A `when@<env>` block
    /// adds only namespaced paths, after the others, since its main-namespace paths
    /// and `default_path` would shadow the other environments' templates.
    pub(crate) fn read(project: &Project, config_files: &ConfigFiles) -> Templates {
        let dir = &project.dir;
        let mut templates = Templates {
            unknown: has_unread_twig_configuration(&dir.join("config/packages"), config_files),
            ..Templates::default()
        };
        let mut default_path = format!("{PROJECT_DIR}/templates");
        for name in ["twig.yaml", "twig.yml"] {
            let Some(parsed) = config_files.get(&dir.join("config/packages").join(name)) else {
                continue;
            };
            let Some(root) = &parsed.yaml else {
                continue;
            };
            if let Some(twig) = root.get("twig") {
                templates.read_paths(dir, twig.get("paths"), true);
                if let Some(path) = twig.get("default_path").and_then(Yaml::text) {
                    default_path = path.to_string();
                }
            }
            if let Yaml::Map(entries) = root {
                let environments = entries
                    .iter()
                    .filter(|(key, _)| key.text().is_some_and(|k| k.starts_with("when@")))
                    .filter_map(|(_, value)| value.get("twig"));
                for twig in environments {
                    templates.read_paths(dir, twig.get("paths"), false);
                    templates.main_unknown |= twig.get("default_path").is_some();
                }
            }
        }
        match resolve(dir, &default_path) {
            Some(path) if path.is_dir() => templates.paths.push((None, Some(path))),
            Some(_) => {}
            None => templates.paths.push((None, None)),
        }
        // Without the installed bridge, the main namespace is not known in full.
        let form_themes = dir.join(FORM_THEMES);
        let form_themes = form_themes.is_dir().then_some(form_themes);
        templates.paths.push((None, form_themes));
        templates
    }

    /// `paths` as a map from directory to namespace (empty or null for the main
    /// one), or as a list of main-namespace directories; `main` says whether
    /// main-namespace entries count.
    fn read_paths(&mut self, dir: &Path, paths: Option<&Yaml>, main: bool) {
        let entries: Vec<(Option<&str>, Option<String>)> = match paths {
            Some(Yaml::Map(entries)) => entries
                .iter()
                .map(|(path, namespace)| {
                    let namespace = namespace
                        .text()
                        .filter(|n| !matches!(n.to_ascii_lowercase().as_str(), "" | "~" | "null"))
                        .map(str::to_string);
                    (path.text(), namespace)
                })
                .collect(),
            Some(Yaml::Sequence(entries)) => {
                entries.iter().map(|path| (path.text(), None)).collect()
            }
            _ => Vec::new(),
        };
        for (path, namespace) in entries {
            if namespace.is_none() && !main {
                self.main_unknown = true;
                continue;
            }
            self.paths
                .push((namespace, path.and_then(|p| resolve(dir, p))));
        }
    }

    /// The first existing file `name` names: `@Ns/rest` in `Ns`'s directories,
    /// anything else in the main namespace's.
    pub(crate) fn file(&self, name: &str) -> Option<PathBuf> {
        match self.status(name) {
            TemplateStatus::Found(path) => Some(path),
            TemplateStatus::Missing | TemplateStatus::Unknown => None,
        }
    }

    /// Whether `name` resolves to a file, as Twig's loader looks for it.
    pub(crate) fn status(&self, name: &str) -> TemplateStatus {
        let name = name.replace('\\', "/");
        let (namespace, rest) = match name.strip_prefix('@') {
            Some(named) => match named.split_once('/') {
                Some((namespace, rest)) => (Some(namespace), rest),
                None => return TemplateStatus::Unknown,
            },
            None => (None, name.as_str()),
        };
        // Twig refuses a name that climbs above its directory.
        let Some(rest) = normalize(rest) else {
            return TemplateStatus::Unknown;
        };
        let mut configured = false;
        for (_, dir) in self
            .paths
            .iter()
            .filter(|(ns, _)| ns.as_deref() == namespace)
        {
            configured = true;
            // Twig would look in an unreadable directory first: its answer is unknown.
            let Some(dir) = dir else {
                return TemplateStatus::Unknown;
            };
            let path = dir.join(&rest);
            if path.is_file() {
                return TemplateStatus::Found(path);
            }
        }
        if configured && !self.unknown && !(namespace.is_none() && self.main_unknown) {
            TemplateStatus::Missing
        } else {
            TemplateStatus::Unknown
        }
    }

    /// Whether `path` is inside one of the configured template directories.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        self.paths
            .iter()
            .filter_map(|(_, dir)| dir.as_deref())
            .any(|dir| path.starts_with(dir))
    }
}

/// Whether `packages`, the `config/packages` directory, holds Twig configuration
/// other than `twig.yaml` and `twig.yml`: `twig.php`, a `twig:` section in another
/// file, or an environment directory with Twig configuration.
fn has_unread_twig_configuration(packages: &Path, config_files: &ConfigFiles) -> bool {
    let Ok(entries) = std::fs::read_dir(packages) else {
        return false;
    };
    entries.flatten().map(|entry| entry.path()).any(|path| {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            return std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("twig."))
                });
        }
        if name == "twig.yaml" || name == "twig.yml" {
            return false;
        }
        if name.ends_with(".php") {
            return name == "twig.php";
        }
        if !(name.ends_with(".yaml") || name.ends_with(".yml")) {
            return false;
        }
        config_files.get(&path).is_some_and(|parsed| {
            parsed.yaml.as_ref().is_some_and(|root| {
                super::config::top_blocks(root)
                    .iter()
                    .any(|block| block.get("twig").is_some())
            })
        })
    })
}

/// `name` with empty and `.` parts dropped and each `..` taking off the part before
/// it, as PHP resolves the path Twig's loader builds; none when a `..` climbs above
/// the start or nothing is left.
fn normalize(name: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A configured path with the project directory filled in; relative paths are
/// relative to the project. A path with another parameter cannot be read.
fn resolve(dir: &Path, path: &str) -> Option<PathBuf> {
    if path.replace(PROJECT_DIR, "").contains('%') {
        return None;
    }
    let path = path.replace(PROJECT_DIR, &dir.to_string_lossy());
    let path = PathBuf::from(path);
    Some(if path.is_absolute() {
        path
    } else {
        dir.join(path)
    })
}
