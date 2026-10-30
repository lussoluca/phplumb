//! Symfony's YAML configuration: the files the kernel loads and those they import,
//! the parameters, services, aliases and tags they define, and parameter names
//! written as `%name%`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::files::ConfigFiles;
use crate::filter::PathFilter;
use crate::frameworks::Project;
use crate::frameworks::symfony::RouteTarget;
use crate::frameworks::yaml::{self, Yaml};
use crate::index::Location;

/// The configuration files the kernel loads: `config/services.{yaml,yml}`,
/// `config/services_<env>.{yaml,yml}`, `config/packages/*.{yaml,yml}` and
/// `config/packages/<env>/*.{yaml,yml}`, in path order, then the YAML files their
/// `imports:` name, each once; and whether an import names something the module
/// does not read: a glob, a directory, another format, or a missing file whose
/// errors are not ignored.
pub(crate) fn config_files(
    project: &Project,
    filter: &PathFilter,
    config_files: &ConfigFiles,
) -> (Vec<PathBuf>, bool) {
    let config = project.dir.join("config");
    let yaml = |name: &str| name.ends_with(".yaml") || name.ends_with(".yml");
    let mut files = files_named(&config, |name| {
        let stem = name.trim_end_matches(".yaml").trim_end_matches(".yml");
        yaml(name) && (stem == "services" || stem.starts_with("services_"))
    });
    let packages = config.join("packages");
    files.extend(files_named(&packages, yaml));
    let mut environments: Vec<PathBuf> = std::fs::read_dir(&packages)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    environments.sort();
    for environment in environments {
        files.extend(files_named(&environment, yaml));
    }
    files.retain(|path| filter.in_scope(path));
    let mut unread = false;
    let mut seen: Vec<PathBuf> = files
        .iter()
        .map(|file| file.canonicalize().unwrap_or_else(|_| file.clone()))
        .collect();
    let mut next = 0;
    while next < files.len() {
        let file = files[next].clone();
        next += 1;
        for import in imports_of(&file, config_files) {
            match import {
                Import::File(imported) => {
                    let real = imported.canonicalize().unwrap_or_else(|_| imported.clone());
                    if !seen.contains(&real) {
                        seen.push(real);
                        files.push(imported);
                    }
                }
                Import::Ignored => {}
                Import::Unread => unread = true,
            }
        }
    }
    (files, unread)
}

/// What an entry of `imports:` loads.
enum Import {
    File(PathBuf),
    /// A missing file whose errors are ignored.
    Ignored,
    Unread,
}

/// The imports of the configuration file `file`, also inside `when@<env>` blocks.
fn imports_of(file: &Path, config_files: &ConfigFiles) -> Vec<Import> {
    let Some(parsed) = config_files.get(file) else {
        return Vec::new();
    };
    let Some(root) = &parsed.yaml else {
        return Vec::new();
    };
    top_blocks(root)
        .into_iter()
        .filter_map(|block| match block.get("imports") {
            Some(Yaml::Sequence(entries)) => Some(entries),
            _ => None,
        })
        .flatten()
        .map(|entry| {
            let resource = entry.text().or_else(|| entry.get("resource")?.text());
            let Some(resource) = resource else {
                return Import::Unread;
            };
            let ignored = entry
                .get("ignore_errors")
                .and_then(Yaml::text)
                .is_some_and(|value| matches!(value, "true" | "not_found"));
            match imported_file(file, resource) {
                Some(path) if path.is_file() => {
                    let name = path.to_string_lossy();
                    if name.ends_with(".yaml") || name.ends_with(".yml") {
                        Import::File(path)
                    } else {
                        Import::Unread
                    }
                }
                Some(path) if !path.exists() && ignored => Import::Ignored,
                _ => Import::Unread,
            }
        })
        .collect()
}

/// The file the import `resource` of `file` names, relative to its directory;
/// none for a glob or a bundle path.
pub(crate) fn imported_file(file: &Path, resource: &str) -> Option<PathBuf> {
    if resource.starts_with('@') || resource.contains(['*', '?', '{', '[']) {
        return None;
    }
    Some(normalize(&file.parent()?.join(resource)))
}

/// The files of `dir` whose name `keep` accepts, in path order.
fn files_named(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(&keep))
        .collect();
    files.sort();
    files
}

/// The parameters the configuration files define under `parameters:`, also
/// inside `when@<env>` blocks, each at its key; `env(NAME)` keys are environment
/// defaults, not parameters.
pub(crate) fn read_parameters(
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> HashMap<String, Vec<RouteTarget>> {
    let mut found: HashMap<String, Vec<RouteTarget>> = HashMap::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let (text, Some(root)) = (parsed.text.as_str(), &parsed.yaml) else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(Yaml::Map(entries)) = block.get("parameters") else {
                continue;
            };
            for (key, _) in entries {
                let Yaml::Scalar { value, start, end } = key else {
                    continue;
                };
                if value.starts_with("env(") {
                    continue;
                }
                let (start, end) = yaml::value_range(text, value, *start, *end);
                found.entry(value.clone()).or_default().push(RouteTarget {
                    location: Location {
                        path: file.clone(),
                        start,
                        end,
                    },
                    text: value.clone(),
                });
            }
        }
    }
    found
}

/// The services the configuration files define under `services:`, also inside
/// `when@<env>` blocks, each at its key; `_defaults`, `_instanceof` and prototypes
/// (keys with a `resource`) are no services.
pub(crate) fn read_services(
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> HashMap<String, Vec<RouteTarget>> {
    let mut found: HashMap<String, Vec<RouteTarget>> = HashMap::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let (text, Some(root)) = (parsed.text.as_str(), &parsed.yaml) else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(Yaml::Map(entries)) = block.get("services") else {
                continue;
            };
            for (key, value) in entries {
                let Yaml::Scalar {
                    value: id,
                    start,
                    end,
                } = key
                else {
                    continue;
                };
                if id == "_defaults" || id == "_instanceof" || value.get("resource").is_some() {
                    continue;
                }
                let (start, end) = yaml::value_range(text, id, *start, *end);
                found.entry(id.clone()).or_default().push(RouteTarget {
                    location: Location {
                        path: file.clone(),
                        start,
                        end,
                    },
                    text: id.clone(),
                });
            }
        }
    }
    found
}

/// The bus Symfony adds when the configuration defines none.
const DEFAULT_BUS: &str = "messenger.bus.default";

/// The message buses the configuration files define under `framework.messenger`
/// and the default one: `default_bus`, or the only bus. Without buses, the one
/// Symfony adds; none when several buses have no default, or when a `when@<env>`
/// block changes them.
pub(crate) fn buses(
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> Option<(Vec<String>, String)> {
    let mut names: Vec<String> = Vec::new();
    let mut default: Option<String> = None;
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let Some(root) = &parsed.yaml else {
            continue;
        };
        // A file of an environment directory changes the buses of that environment
        // only, as a `when@<env>` block does.
        let environment = file
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name == "packages");
        for (position, block) in top_blocks(root).into_iter().enumerate() {
            let Some(messenger) = block.get("framework").and_then(|f| f.get("messenger")) else {
                continue;
            };
            let changes_buses =
                messenger.get("buses").is_some() || messenger.get("default_bus").is_some();
            if (environment || position > 0) && changes_buses {
                return None;
            }
            if let Some(Yaml::Map(buses)) = messenger.get("buses") {
                for name in buses.iter().filter_map(|(key, _)| key.text()) {
                    if !names.iter().any(|known| known == name) {
                        names.push(name.to_string());
                    }
                }
            }
            if let Some(bus) = messenger.get("default_bus").and_then(Yaml::text) {
                default = Some(bus.to_string());
            }
        }
    }
    if names.is_empty() {
        return Some((vec![DEFAULT_BUS.to_string()], DEFAULT_BUS.to_string()));
    }
    let default = default.or_else(|| (names.len() == 1).then(|| names[0].clone()))?;
    Some((names, default))
}

/// The route resource Symfony loads unless `framework.router.resource` names another.
const DEFAULT_ROUTER_RESOURCE: &str = "kernel::loadRoutes";

/// Whether the configuration files give `framework.router.resource` another
/// resource than the kernel's routes, also inside a `when@<env>` block.
pub(crate) fn router_resource_changed(files: &[PathBuf], config_files: &ConfigFiles) -> bool {
    files.iter().any(|file| {
        let Some(parsed) = config_files.get(file) else {
            return false;
        };
        let Some(root) = &parsed.yaml else {
            return false;
        };
        top_blocks(root).into_iter().any(|block| {
            block
                .get("framework")
                .and_then(|framework| framework.get("router"))
                .and_then(|router| router.get("resource"))
                .is_some_and(|resource| resource.text() != Some(DEFAULT_ROUTER_RESOURCE))
        })
    })
}

/// A service the configuration files tag: its class, or for `_instanceof` the
/// type whose implementations carry the tag, and the tag's attributes.
pub(crate) struct Tagged {
    pub class: String,
    pub instanceof: bool,
    /// For a prototype, the directory its `resource` loads, `class` being its
    /// namespace.
    pub resource: Option<PathBuf>,
    pub attributes: Vec<(String, String)>,
}

/// The services the configuration files tag `tag`, also inside `when@<env>`
/// blocks and through `_instanceof`: by name in the list of tags, as
/// `{ name: tag, ... }`, or as `tag: { ... }`. A service's class is its `class`, or
/// its id when written like a class.
pub(crate) fn tagged_services(
    files: &[PathBuf],
    config_files: &ConfigFiles,
    tag: &str,
) -> Vec<Tagged> {
    let mut found = Vec::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let Some(root) = &parsed.yaml else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(Yaml::Map(entries)) = block.get("services") else {
                continue;
            };
            for (key, value) in entries {
                let Some(id) = key.text() else {
                    continue;
                };
                if id == "_instanceof" {
                    let Yaml::Map(types) = value else {
                        continue;
                    };
                    for (class, service) in types {
                        let Some(class) = class.text() else {
                            continue;
                        };
                        for attributes in tags_of(service, tag) {
                            found.push(Tagged {
                                class: class.trim_start_matches('\\').to_string(),
                                instanceof: true,
                                resource: None,
                                attributes,
                            });
                        }
                    }
                    continue;
                }
                if let Some(resource) = value.get("resource").and_then(Yaml::text) {
                    // A prototype: the classes of a namespace loaded from a directory.
                    let namespace = value.get("namespace").and_then(Yaml::text).unwrap_or(id);
                    let dir = file
                        .parent()
                        .map(|parent| normalize(&parent.join(resource)));
                    for attributes in tags_of(value, tag) {
                        found.push(Tagged {
                            class: namespace.trim_start_matches('\\').to_string(),
                            instanceof: false,
                            resource: dir.clone(),
                            attributes,
                        });
                    }
                    continue;
                }
                let class = match value.get("class").and_then(Yaml::text) {
                    Some(class) => class,
                    None if looks_like_class(id) => id,
                    None => continue,
                };
                for attributes in tags_of(value, tag) {
                    found.push(Tagged {
                        class: class.trim_start_matches('\\').to_string(),
                        instanceof: false,
                        resource: None,
                        attributes,
                    });
                }
            }
        }
    }
    found
}

/// `path` with `.` and `..` components resolved without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The argument names the configuration binds or passes by name, `$name` keys of
/// `bind` and `arguments` maps, and the classes of services given `arguments` or
/// `bind`, so an argument of theirs may be wired otherwise than by name.
pub(crate) fn bindings(
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> (Vec<String>, Vec<String>) {
    let mut names: Vec<String> = Vec::new();
    let mut classes: Vec<String> = Vec::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let Some(root) = &parsed.yaml else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(Yaml::Map(entries)) = block.get("services") else {
                continue;
            };
            let mut services: Vec<(&str, &Yaml)> = Vec::new();
            for (key, value) in entries {
                let Some(id) = key.text() else {
                    continue;
                };
                if id == "_instanceof" {
                    if let Yaml::Map(types) = value {
                        services.extend(types.iter().filter_map(|(k, v)| Some((k.text()?, v))));
                    }
                } else {
                    services.push((id, value));
                }
            }
            for (id, service) in services {
                let mut wired = false;
                for key in ["bind", "arguments"] {
                    let Some(map) = service.get(key) else {
                        continue;
                    };
                    wired = true;
                    if let Yaml::Map(entries) = map {
                        for name in entries.iter().filter_map(|(k, _)| k.text()) {
                            names.push(name.to_string());
                        }
                    }
                }
                if wired {
                    let class = service.get("class").and_then(Yaml::text).unwrap_or(id);
                    classes.push(class.trim_start_matches('\\').to_string());
                }
            }
        }
    }
    (names, classes)
}

/// The attributes of each `tag` in the `tags` of `service`.
fn tags_of(service: &Yaml, tag: &str) -> Vec<Vec<(String, String)>> {
    let Some(Yaml::Sequence(tags)) = service.get("tags") else {
        return Vec::new();
    };
    let scalars = |entries: &[(Yaml, Yaml)]| -> Vec<(String, String)> {
        entries
            .iter()
            .filter_map(|(key, value)| Some((key.text()?.to_string(), value.text()?.to_string())))
            .filter(|(key, _)| key != "name")
            .collect()
    };
    let mut found = Vec::new();
    for entry in tags {
        match entry {
            Yaml::Scalar { value, .. } if value == tag => found.push(Vec::new()),
            Yaml::Map(entries) if entry.get("name").and_then(Yaml::text) == Some(tag) => {
                found.push(scalars(entries));
            }
            Yaml::Map(entries) if entries.len() == 1 && entries[0].0.text() == Some(tag) => {
                found.push(match &entries[0].1 {
                    Yaml::Map(attributes) => scalars(attributes),
                    _ => Vec::new(),
                });
            }
            _ => {}
        }
    }
    found
}

/// The service id a reference `@id` names, past the `?`, `!` and `>` markers;
/// none for an expression (`@=`) or an escaped `@` (`@@`).
pub(crate) fn service_reference(value: &str) -> Option<&str> {
    let reference = value.strip_prefix('@')?;
    if reference.starts_with('=') || reference.starts_with('@') {
        return None;
    }
    Some(reference.trim_start_matches(['?', '!', '>']))
}

/// The aliases the configuration files define under `services:`, also inside
/// `when@<env>` blocks: `id: '@other'` and `id: { alias: other }`, each id with the
/// services it stands for in one environment or another.
pub(crate) fn read_aliases(
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> HashMap<String, Vec<String>> {
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let Some(root) = &parsed.yaml else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(Yaml::Map(entries)) = block.get("services") else {
                continue;
            };
            for (key, value) in entries {
                let Some(id) = key.text() else {
                    continue;
                };
                let target = match value {
                    Yaml::Scalar { value, .. } => service_reference(value),
                    _ => value.get("alias").and_then(Yaml::text),
                };
                if let Some(target) = target {
                    let targets = found.entry(id.to_string()).or_default();
                    if !targets.iter().any(|known| known == target) {
                        targets.push(target.to_string());
                    }
                }
            }
        }
    }
    found
}

/// Whether `text` is written like a fully qualified class name.
pub(crate) fn looks_like_class(text: &str) -> bool {
    let name = text.trim_start_matches('\\');
    name.contains('\\')
        && name
            .split('\\')
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// The document itself, then each `when@<env>` block in it.
pub(crate) fn top_blocks(root: &Yaml) -> Vec<&Yaml> {
    let mut blocks = vec![root];
    if let Yaml::Map(entries) = root {
        blocks.extend(
            entries
                .iter()
                .filter(|(key, _)| key.text().is_some_and(|k| k.starts_with("when@")))
                .map(|(_, value)| value),
        );
    }
    blocks
}

/// Each `%name%` of `text` with the range of its name, `%%` escapes skipped.
fn placeholders(text: &str) -> Vec<(&str, usize, usize)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'%') {
            i += 2;
            continue;
        }
        // A name runs to the next `%`, without spaces; a `%` starting none, as in
        // `50% of`, is left alone, as Symfony's `%%|%([^%\s]+)%` does.
        let close = bytes[i + 1..]
            .iter()
            .position(|&b| b == b'%' || b.is_ascii_whitespace())
            .map(|length| i + 1 + length);
        match close {
            Some(close) if close > i + 1 && bytes[close] == b'%' => {
                found.push((&text[i + 1..close], i + 1, close));
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    found
}

/// Whether a placeholder name is a parameter the configuration may define: not
/// `%env(...)%` nor one of the kernel's own `kernel.*` parameters.
fn readable(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(char::is_whitespace)
        && !name.starts_with("env(")
        && !name.starts_with("kernel.")
}

/// The parameter name of the `%name%` that holds `offset` in `text`; none for the
/// `%%` escape, `%env(...)%` and the kernel's own `kernel.*` parameters.
pub(crate) fn parameter_at(text: &str, offset: usize) -> Option<String> {
    let (name, ..) = placeholders(text)
        .into_iter()
        .find(|(_, start, end)| *start <= offset + 1 && offset <= *end)?;
    readable(name).then(|| name.to_string())
}

/// The parameter names written as `%name%` in `text`, each with its range, apart
/// from `%env(...)%` and `kernel.*`.
pub(crate) fn parameters_in(text: &str) -> Vec<(&str, usize, usize)> {
    placeholders(text)
        .into_iter()
        .filter(|(name, ..)| readable(name))
        .collect()
}

/// Whether the project has configuration the module does not read, which may define
/// parameters: PHP or XML configuration files in `config`, `config/packages` and its
/// environment directories.
pub(crate) fn has_unread_configuration(project: &Project) -> bool {
    let config = project.dir.join("config");
    let php = |name: &str| name.ends_with(".php");
    let services_php = !files_named(&config, |name| {
        php(name) && (name == "services.php" || name.starts_with("services_"))
    })
    .is_empty();
    // PHP and XML configuration, in `config/packages` and its environment directories.
    let unread =
        |dir: &Path| !files_named(dir, |name| php(name) || name.ends_with(".xml")).is_empty();
    let packages = config.join("packages");
    let environments = std::fs::read_dir(&packages)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .any(|path| path.is_dir() && unread(&path));
    services_php || unread(&packages) || environments
}

#[cfg(test)]
mod tests {
    use super::parameter_at;

    fn at(text: &str, needle: &str) -> Option<String> {
        parameter_at(text, text.find(needle).unwrap() + 1)
    }

    #[test]
    fn reads_parameter_names() {
        assert_eq!(at("'%a.b%'", "a.b"), Some("a.b".into()));
        assert_eq!(at("x/%a%/y", "a%"), Some("a".into()));
        assert_eq!(at("x/%a%/y", "/y"), None);
        assert_eq!(at("50% of %app.name%", "app.name"), Some("app.name".into()));
        assert_eq!(at("100%", "%"), None);
        assert_eq!(at("%", "%"), None);
        assert_eq!(at("%%a%%", "a"), None);
        assert_eq!(at("'100%%'", "%%"), None);
        assert_eq!(at("'%env(A)%'", "env"), None);
        assert_eq!(at("'%kernel.secret%'", "kernel"), None);
        assert_eq!(at("'%%a%'", "a%"), None);
    }
}
