//! Symfony routes: the names `#[Route]` attributes give, as Symfony's attribute
//! loader and FrameworkBundle name them, and the routes of YAML route files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::files::{ConfigFiles, Parsed};

use crate::filter::{PathFilter, is_yaml};
use crate::frameworks::Project;
use crate::frameworks::attributes::{RouteName, has_route_aliases, route_names};
use crate::frameworks::yaml::{self, Yaml};
use crate::index::{Index, Location};
use crate::summary::{Declaration, Member, MemberKind, SymbolKind};

/// Where a route name leads, with the text expected at the range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    pub location: Location,
    pub text: String,
}

/// Route resources of Symfony's bundles, whose routes all have names starting
/// with `_`.
const SYMFONY_RESOURCES: [&str; 5] = [
    "@FrameworkBundle/",
    "@WebProfilerBundle/",
    "@SecurityBundle/",
    "@TwigBundle/",
    "security.route_loader.logout",
];

/// What a route import loads, as far as the module knows.
enum Import {
    /// Routes the module reads anyway: attribute routes of project code or a route
    /// file it reads.
    Read,
    /// Routes of one of Symfony's bundles.
    Symfony,
    Unknown,
}

#[derive(Default)]
pub(crate) struct Routes {
    by_name: HashMap<String, Vec<RouteTarget>>,
    /// Whether a source of route names could not be read, so any name may exist.
    unknown: bool,
    /// Whether Symfony's own routes are imported.
    symfony: bool,
}

impl Routes {
    /// Every route of the project: attribute routes of project classes, then the
    /// routes of `config/routes.yaml` and the YAML files below `config/routes`.
    pub(crate) fn read(
        index: &Index,
        filter: &PathFilter,
        project: &Project,
        config_files: &ConfigFiles,
    ) -> Routes {
        let mut routes = Routes::default();
        let mut routed: HashMap<String, bool> = HashMap::new();
        for (path, declaration) in index.declarations() {
            if declaration.kind != SymbolKind::Class {
                continue;
            }
            // Path checks cost more than reading attributes, so they come last.
            let routing = has_routes(index, declaration, &mut routed);
            let configures = declares_method(declaration, "configureRoutes");
            if (routing || configures) && filter.in_scope(path) && index.is_project_code(path) {
                // A kernel that configures its routes may load anything.
                routes.unknown |= configures;
                if routing {
                    routes.read_class(index, path, declaration);
                }
            }
        }
        let config = project.dir.join("config");
        let files = route_files(&config, filter);
        routes.unknown |= has_unread_route_files(&config);
        for file in &files {
            routes.read_yaml(index, config_files.get(file), file, &files, &config);
        }
        routes
    }

    pub(crate) fn targets(&self, name: &str) -> &[RouteTarget] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// Whether no route is named `name`, as far as every source of route names was
    /// read: an unread source, Symfony's own `_` routes, and the localized routes
    /// `name.locale` of a route `name` keep it unknown.
    pub(crate) fn missing(&self, name: &str) -> bool {
        if self.unknown || !self.targets(name).is_empty() {
            return false;
        }
        if self.symfony && name.starts_with('_') {
            return false;
        }
        !name
            .rsplit_once('.')
            .is_some_and(|(base, _)| !self.targets(base).is_empty())
    }

    fn add(&mut self, name: String, target: RouteTarget) {
        let targets = self.by_name.entry(name).or_default();
        if !targets.contains(&target) {
            targets.push(target);
        }
    }

    /// The routes of one class, named as `AttributeClassLoader::load` names them:
    /// abstract classes have none, and inherited methods route on the class.
    fn read_class(&mut self, index: &Index, path: &Path, class: &Declaration) {
        let Some(shape) = &class.class else {
            return;
        };
        if shape.is_abstract {
            return;
        }
        let fqn = &class.fqn;
        let class_routes = route_names(&shape.metadata, index, fqn);
        self.unknown |= has_route_aliases(&shape.metadata);
        let prefix = match class_routes.first() {
            Some(RouteName::Literal(prefix)) => prefix.clone(),
            Some(RouteName::Unreadable) => {
                self.unknown = true;
                return;
            }
            Some(RouteName::Missing) | None => String::new(),
        };
        let target = |(path, member): &(PathBuf, &Member)| RouteTarget {
            location: Location {
                path: path.clone(),
                start: member.name_start,
                end: member.name_end,
            },
            text: member.name.clone(),
        };
        let methods = methods_of(index, path, class);
        let mut count = 0;
        let mut invoke_routes: Vec<String> = Vec::new();
        for entry in &methods {
            let method = entry.1;
            let attributes = route_names(&method.metadata, index, fqn);
            self.unknown |= has_route_aliases(&method.metadata);
            let mut defaults = 0;
            let mut names = Vec::new();
            for attribute in &attributes {
                match attribute {
                    RouteName::Literal(name) => names.push(format!("{prefix}{name}")),
                    RouteName::Missing => {
                        names.push(format!(
                            "{prefix}{}",
                            default_name(fqn, &method.name, defaults)
                        ));
                        defaults += 1;
                    }
                    RouteName::Unreadable => self.unknown = true,
                }
            }
            count += attributes.len();
            if method.name.eq_ignore_ascii_case("__invoke") {
                invoke_routes.extend(names.iter().cloned());
            }
            if attributes.len() == 1 && names.len() == 1 {
                self.add(format!("{fqn}::{}", method.name), target(entry));
            }
            for name in names {
                self.add(name, target(entry));
            }
        }
        let invoke = methods
            .iter()
            .find(|(_, m)| m.name.eq_ignore_ascii_case("__invoke"));
        if count == 0
            && let Some(invoke) = invoke
        {
            let mut defaults = 0;
            for attribute in &class_routes {
                count += 1;
                let name = match attribute {
                    RouteName::Literal(name) => name.clone(),
                    RouteName::Missing => {
                        defaults += 1;
                        default_name(fqn, &invoke.1.name, defaults - 1)
                    }
                    RouteName::Unreadable => {
                        self.unknown = true;
                        continue;
                    }
                };
                invoke_routes.push(name.clone());
                self.add(name, target(invoke));
            }
        }
        if count == 1
            && invoke_routes.len() == 1
            && let Some(invoke) = invoke
        {
            self.add(fqn.clone(), target(invoke));
            self.add(format!("{fqn}::__invoke"), target(invoke));
        }
    }

    /// The routes of one YAML route file: top-level keys with a `path`, also inside
    /// `when@<env>` blocks; imports define none, and say whether the routes they
    /// load are known. `files` are the route files read, below `config`.
    fn read_yaml(
        &mut self,
        index: &Index,
        parsed: Option<Arc<Parsed>>,
        file: &Path,
        files: &[PathBuf],
        config: &Path,
    ) {
        let Some(parsed) = parsed else {
            self.unknown = true;
            return;
        };
        let text = parsed.text.as_str();
        let entries = match &parsed.yaml {
            Some(Yaml::Map(entries)) => entries,
            // A file of comments only holds no document.
            None if text.lines().all(|line| {
                let line = line.trim();
                line.is_empty() || line.starts_with('#') || line == "---"
            }) =>
            {
                return;
            }
            _ => {
                self.unknown = true;
                return;
            }
        };
        let mut found: Vec<(&Yaml, &Yaml)> = Vec::new();
        for (key, value) in entries {
            if key.text().is_some_and(|k| k.starts_with("when@")) {
                if let Yaml::Map(inner) = value {
                    found.extend(inner.iter().map(|(k, v)| (k, v)));
                }
            } else {
                found.push((key, value));
            }
        }
        for (key, value) in found {
            let Yaml::Scalar {
                value: name,
                start,
                end,
            } = key
            else {
                continue;
            };
            if value.get("resource").is_some() {
                match import(index, file, value, files, config) {
                    Import::Read => {}
                    Import::Symfony => self.symfony = true,
                    Import::Unknown => self.unknown = true,
                }
                continue;
            }
            if value.get("alias").is_some() {
                self.unknown = true;
                continue;
            }
            if value.get("path").is_none() {
                continue;
            }
            let (start, end) = yaml::value_range(text, name, *start, *end);
            self.add(
                name.clone(),
                RouteTarget {
                    location: Location {
                        path: file.to_path_buf(),
                        start,
                        end,
                    },
                    text: name.clone(),
                },
            );
            let controller = value
                .get("controller")
                .or_else(|| value.get("defaults")?.get("_controller"))
                .and_then(Yaml::text);
            if let Some(target) = controller.and_then(|c| controller_target(index, c)) {
                self.add(name.clone(), target);
            }
        }
    }
}

/// What the import `value` of the route file `file` loads.
fn import(index: &Index, file: &Path, value: &Yaml, files: &[PathBuf], config: &Path) -> Import {
    if value.get("name_prefix").is_some() {
        return Import::Unknown;
    }
    let resource = value.get("resource");
    let Some(resource) = resource
        .and_then(Yaml::text)
        .or_else(|| resource?.get("path")?.text())
    else {
        return Import::Unknown;
    };
    if resource == "routing.controllers" {
        return Import::Read;
    }
    if SYMFONY_RESOURCES.iter().any(|r| resource.starts_with(r)) {
        return Import::Symfony;
    }
    if resource.starts_with('@') || resource.contains(['*', '?', '{', '[']) {
        return Import::Unknown;
    }
    let kind = value.get("type").and_then(Yaml::text);
    // Routes in docblock annotations are not read.
    if kind == Some("annotation") {
        return Import::Unknown;
    }
    if kind == Some("attribute") {
        // Attribute routes of project classes are read wherever they are.
        if resource.contains('\\') && !resource.contains('/') {
            let project = index
                .classes_named(resource.trim_start_matches('\\'))
                .iter()
                .any(|class| index.is_project_code(class.path));
            return if project {
                Import::Read
            } else {
                Import::Unknown
            };
        }
        let resolved = normalize(&file.parent().unwrap_or(config).join(resource));
        let outside = config.parent().is_none_or(|dir| !resolved.starts_with(dir))
            || resolved.components().any(|c| c.as_os_str() == "vendor");
        return if outside {
            Import::Unknown
        } else {
            Import::Read
        };
    }
    if kind.is_some_and(|k| k != "yaml" && k != "directory") {
        return Import::Unknown;
    }
    let resolved = normalize(&file.parent().unwrap_or(config).join(resource));
    let read = files.contains(&resolved)
        || (resolved.is_dir() && resolved.starts_with(config.join("routes")));
    if read { Import::Read } else { Import::Unknown }
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

/// Whether `config` holds route files the module does not read: `routes.php`, or
/// PHP or XML files below `config/routes`.
fn has_unread_route_files(config: &Path) -> bool {
    fn below(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().map(|entry| entry.path()).any(|path| {
            let kind = path.extension().and_then(|e| e.to_str());
            if path.is_dir() {
                below(&path)
            } else {
                matches!(kind, Some("php" | "xml"))
            }
        })
    }
    config.join("routes.php").is_file() || below(&config.join("routes"))
}

/// Whether `class` declares a method named `name`.
fn declares_method(class: &Declaration, name: &str) -> bool {
    class.class.as_ref().is_some_and(|shape| {
        shape
            .members
            .iter()
            .any(|m| m.kind == MemberKind::Method && m.name.eq_ignore_ascii_case(name))
    })
}

/// Whether `class`, one of its traits or one of its parents has a `#[Route]`,
/// memoized by lowercased name.
fn has_routes(index: &Index, class: &Declaration, memo: &mut HashMap<String, bool>) -> bool {
    let key = class.fqn.to_ascii_lowercase();
    if let Some(&known) = memo.get(&key) {
        return known;
    }
    // A cycle in the hierarchy reads as no routes.
    memo.insert(key.clone(), false);
    let Some(shape) = &class.class else {
        return false;
    };
    let own = !route_names(&shape.metadata, index, &class.fqn).is_empty()
        || shape.members.iter().any(|m| {
            m.kind == MemberKind::Method && !route_names(&m.metadata, index, &class.fqn).is_empty()
        });
    let found = own
        || shape.parent.iter().chain(&shape.traits).any(|name| {
            index
                .classes_named(name)
                .first()
                .is_some_and(|ancestor| has_routes(index, ancestor.declaration, memo))
        });
    memo.insert(key, found);
    found
}

/// The methods of `class` as reflection lists them: its own, then those of its
/// traits and parents that it does not redeclare, each with its file.
fn methods_of<'a>(
    index: &'a Index,
    path: &Path,
    class: &'a Declaration,
) -> Vec<(PathBuf, &'a Member)> {
    let mut found: Vec<(PathBuf, &Member)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut pending: Vec<(PathBuf, &Declaration)> = vec![(path.to_path_buf(), class)];
    let mut visited = 0;
    while let Some((path, declaration)) = pending.pop() {
        visited += 1;
        if visited > 16 {
            break;
        }
        let Some(shape) = &declaration.class else {
            continue;
        };
        for member in shape
            .members
            .iter()
            .filter(|m| m.kind == MemberKind::Method)
        {
            let name = member.name.to_ascii_lowercase();
            if !seen.contains(&name) {
                seen.push(name);
                found.push((path.clone(), member));
            }
        }
        // Traits first, so the parent comes off the stack last.
        let ancestors = shape.parent.iter().chain(shape.traits.iter().rev());
        for name in ancestors {
            if let Some(ancestor) = index.classes_named(name).into_iter().next() {
                pending.push((ancestor.path.to_path_buf(), ancestor.declaration));
            }
        }
    }
    found
}

/// The method a route's `controller` names: `Class::method`, or `Class` for an
/// invokable class.
fn controller_target(index: &Index, controller: &str) -> Option<RouteTarget> {
    let controller = controller.trim_start_matches('\\');
    let (class, method) = controller
        .split_once("::")
        .unwrap_or((controller, "__invoke"));
    let (found, member) = index
        .find_member(class, MemberKind::Method, method)
        .into_iter()
        .next()?;
    Some(RouteTarget {
        location: Location {
            path: found.path.to_path_buf(),
            start: member.name_start,
            end: member.name_end,
        },
        text: member.name.clone(),
    })
}

/// `routes.yaml` (or `.yml`) in `config`, then every YAML file below
/// `config/routes`, in path order.
fn route_files(config: &Path, filter: &PathFilter) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ["routes.yaml", "routes.yml"]
        .iter()
        .map(|name| config.join(name))
        .filter(|path| path.is_file())
        .collect();
    let mut below = Vec::new();
    collect_yaml(&config.join("routes"), &mut below);
    below.sort();
    files.extend(below);
    files.retain(|path| filter.in_scope(path));
    files
}

fn collect_yaml(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_yaml(&path, found);
        } else if is_yaml(&path) {
            found.push(path);
        }
    }
}

/// The name Symfony gives the `index`-th unnamed route of `method` (from 0):
/// `AttributeClassLoader::getDefaultRouteName`, then FrameworkBundle's
/// `AttributeRouteControllerLoader` changes.
fn default_name(fqn: &str, method: &str, index: usize) -> String {
    let mut name = format!("{}_{method}", fqn.replace('\\', "_")).to_lowercase();
    if index > 0 {
        name.push_str(&format!("_{index}"));
    }
    let mut name = shorten_bundle_and_controller(&name);
    if method.ends_with("Action") || method.ends_with("_action") {
        name = drop_action(&name);
    }
    name.replace("__", "_")
}

/// `preg_replace('/(bundle|controller)_/', '_', $name)`.
fn shorten_bundle_and_controller(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("bundle_") {
            out.push('_');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("controller_") {
            out.push('_');
            rest = after;
        } else {
            let c = rest.chars().next().expect("not empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// `preg_replace('/action(_\d+)?$/', '\1', $name)`.
fn drop_action(name: &str) -> String {
    let (base, suffix) = match name.rsplit_once('_') {
        Some((base, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (base, &name[base.len()..])
        }
        _ => (name, ""),
    };
    match base.strip_suffix("action") {
        Some(stripped) => format!("{stripped}{suffix}"),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::default_name;

    #[test]
    fn default_names_match_symfony() {
        let fqn = "App\\Controller\\BlogController";
        assert_eq!(default_name(fqn, "listAction", 0), "app_blog_list");
        assert_eq!(default_name(fqn, "listAction", 1), "app_blog_list_1");
        assert_eq!(default_name(fqn, "show", 0), "app_blog_show");
        assert_eq!(
            default_name("Acme\\DemoBundle\\Controller\\Page", "view_action", 2),
            "acme_demo_page_view_2"
        );
    }
}
