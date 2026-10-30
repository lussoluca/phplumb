//! Symfony's YAML configuration: the parameters `config/services*.yaml` and
//! `config/packages/*.yaml` define, and parameter names written as `%name%`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::files::ConfigFiles;
use crate::filter::PathFilter;
use crate::frameworks::Project;
use crate::frameworks::symfony::RouteTarget;
use crate::frameworks::yaml::{self, Yaml};
use crate::index::Location;

/// The configuration files of the project: `config/services.yaml`,
/// `config/services_<env>.yaml`, then `config/packages/*.yaml`, in path order.
pub(crate) fn config_files(project: &Project, filter: &PathFilter) -> Vec<PathBuf> {
    let config = project.dir.join("config");
    let mut files = files_named(&config, |name| {
        name == "services.yaml" || (name.starts_with("services_") && name.ends_with(".yaml"))
    });
    files.extend(files_named(&config.join("packages"), |name| {
        name.ends_with(".yaml")
    }));
    files.retain(|path| filter.in_scope(path));
    files
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

/// A service the configuration files tag: its class, or for `_instanceof` the
/// type whose implementations carry the tag, and the tag's attributes.
pub(crate) struct Tagged {
    pub class: String,
    pub instanceof: bool,
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
                                attributes,
                            });
                        }
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
                        attributes,
                    });
                }
            }
        }
    }
    found
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
        let Some(length) = text[i + 1..].find('%') else {
            break;
        };
        let close = i + 1 + length;
        found.push((&text[i + 1..close], i + 1, close));
        i = close + 1;
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
/// parameters: PHP configuration files, environment directories below
/// `config/packages`, or YAML files that import others, also inside `when@<env>`.
pub(crate) fn has_unread_configuration(
    project: &Project,
    files: &[PathBuf],
    config_files: &ConfigFiles,
) -> bool {
    let config = project.dir.join("config");
    let php = |name: &str| name.ends_with(".php");
    let services_php = !files_named(&config, |name| {
        php(name) && (name == "services.php" || name.starts_with("services_"))
    })
    .is_empty();
    let packages = std::fs::read_dir(config.join("packages"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .any(|path| path.is_dir() || path.file_name().and_then(|n| n.to_str()).is_some_and(php));
    let imports = files.iter().any(|file| {
        config_files.get(file).is_some_and(|parsed| {
            parsed
                .yaml
                .as_ref()
                .is_some_and(|root| top_blocks(root).iter().any(|b| b.get("imports").is_some()))
        })
    });
    services_php || packages || imports
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
        assert_eq!(at("'100%%'", "%%"), None);
        assert_eq!(at("'%env(A)%'", "env"), None);
        assert_eq!(at("'%kernel.secret%'", "kernel"), None);
        assert_eq!(at("'%%a%'", "a%"), None);
    }
}
