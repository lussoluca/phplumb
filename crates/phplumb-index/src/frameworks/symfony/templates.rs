//! Twig template names: the directories `config/packages/twig.yaml` and the
//! bundles give each namespace, in the order TwigBundle adds them to Twig's
//! filesystem loader.

use std::path::{Path, PathBuf};

use super::bundles;
use super::files::ConfigFiles;
use crate::frameworks::Project;
use crate::frameworks::yaml::Yaml;
use crate::index::Index;

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
    /// The positions in `paths` of the bundles' entries, whose templates are not
    /// the project's.
    bundled: Vec<usize>,
}

impl Templates {
    /// The `twig.paths` entries of `config/packages/twig.yaml`, then the bundles'
    /// namespaces, then `default_path` (`templates` by default) when it exists, then
    /// TwigBridge's form themes. A `when@<env>` block or an environment directory
    /// adds only namespaced paths, after the configured ones, since its
    /// main-namespace paths and `default_path` would shadow the other environments'
    /// templates.
    pub(crate) fn read(index: &Index, project: &Project, config_files: &ConfigFiles) -> Templates {
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
                    templates.read_environment(dir, twig);
                }
            }
        }
        // Environment directories, loaded after `config/packages`.
        for file in environment_twig_files(&dir.join("config/packages")) {
            let Some(parsed) = config_files.get(&file) else {
                continue;
            };
            let Some(root) = &parsed.yaml else {
                continue;
            };
            for block in super::config::top_blocks(root) {
                if let Some(twig) = block.get("twig") {
                    templates.read_environment(dir, twig);
                }
            }
        }
        templates.read_bundles(index, dir, &default_path);
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

    /// The namespace of each bundle, as TwigBundle adds them: `Name` for `NameBundle`,
    /// with the project's overrides in `<default_path>/bundles/NameBundle` first,
    /// then the bundle's `Resources/views` or else its `templates`; and `!Name` with
    /// the bundle's own directory alone. A namespace whose directories the module
    /// cannot tell is unknown.
    fn read_bundles(&mut self, index: &Index, dir: &Path, default_path: &str) {
        let overrides = resolve(dir, default_path).filter(|_| !self.main_unknown);
        for (name, bundle) in bundles::bundles(index, &dir.join("config")) {
            let namespace = name.strip_suffix("Bundle").unwrap_or(&name).to_string();
            let (Some(overrides), Some(bundle)) = (&overrides, bundle) else {
                self.paths.push((Some(namespace.clone()), None));
                self.paths.push((Some(format!("!{namespace}")), None));
                continue;
            };
            let own = overrides.join("bundles").join(&name);
            if own.exists() {
                self.bundled.push(self.paths.len());
                self.paths.push((Some(namespace.clone()), Some(own)));
            }
            let views = [bundle.join("Resources/views"), bundle.join("templates")]
                .into_iter()
                .find(|views| views.exists());
            if let Some(views) = views {
                self.bundled
                    .extend([self.paths.len(), self.paths.len() + 1]);
                self.paths
                    .push((Some(namespace.clone()), Some(views.clone())));
                self.paths
                    .push((Some(format!("!{namespace}")), Some(views)));
            }
        }
    }

    /// The Twig configuration of one environment: its namespaced paths, while its
    /// main-namespace paths and `default_path` leave main-namespace names unknown.
    fn read_environment(&mut self, dir: &Path, twig: &Yaml) {
        self.read_paths(dir, twig.get("paths"), false);
        self.main_unknown |= twig.get("default_path").is_some();
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
            // The operating system resolves `.` and `..`, as for Twig's `is_file()`.
            let path = dir.join(&rest);
            if path.is_file() {
                let relative = rest.split('/').any(|part| part == "." || part == "..");
                return TemplateStatus::Found(match relative {
                    true => path.canonicalize().unwrap_or(path),
                    false => path,
                });
            }
        }
        if configured && !self.unknown && !(namespace.is_none() && self.main_unknown) {
            TemplateStatus::Missing
        } else {
            TemplateStatus::Unknown
        }
    }

    /// Whether `path` is inside one of the project's template directories: the
    /// configured ones, `default_path` and the form themes, not those of bundles.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        self.paths
            .iter()
            .enumerate()
            .filter(|(position, _)| !self.bundled.contains(position))
            .filter_map(|(_, (_, dir))| dir.as_deref())
            .any(|dir| path.starts_with(dir))
    }
}

/// The `twig.yaml` and `twig.yml` files of the environment directories of
/// `packages`, in path order.
fn environment_twig_files(packages: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(packages)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .flat_map(|dir| [dir.join("twig.yaml"), dir.join("twig.yml")])
        .filter(|file| file.is_file())
        .collect();
    files.sort();
    files
}

/// Whether `packages`, the `config/packages` directory or one of its environment
/// directories, holds Twig configuration other than `twig.yaml` and `twig.yml`:
/// `twig.php`, or a `twig:` section in another file.
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
            return has_unread_twig_configuration(&path, config_files);
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

/// `name` with runs of `/` collapsed, as Twig's loader normalizes it; none when a
/// `..` climbs above the start, which `FilesystemLoader::validateName` refuses.
fn normalize(name: &str) -> Option<String> {
    let mut level = 0;
    let mut parts: Vec<&str> = Vec::new();
    for part in name.split('/').filter(|part| !part.is_empty()) {
        match part {
            ".." => level -= 1,
            "." => {}
            _ => level += 1,
        }
        if level < 0 {
            return None;
        }
        parts.push(part);
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
