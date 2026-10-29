//! Symfony routes: the names `#[Route]` attributes give, as Symfony's attribute
//! loader and FrameworkBundle name them, and the routes of YAML route files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::files::ConfigFiles;

use crate::filter::{PathFilter, is_yaml};
use crate::frameworks::Project;
use crate::frameworks::attributes::{RouteName, route_names, route_variants};
use crate::frameworks::yaml::{self, Yaml};
use crate::index::{Index, Location};
use crate::infer::php_routes;
use crate::summary::{Declaration, Member, MemberKind, SymbolKind, summarize};

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
    /// A YAML route file of the project the module reads only through this import,
    /// with the prefix the import gives its names.
    File(PathBuf, String),
    /// Attribute routes of project classes in a directory, or of one class, under
    /// the prefix the import gives their names.
    Prefixed(AttributeResource, String),
    /// Routes of one of Symfony's bundles.
    Symfony,
    Unknown,
}

/// The classes an attribute route import reads.
enum AttributeResource {
    Directory(PathBuf),
    Class(String),
}

#[derive(Default)]
pub(crate) struct Routes {
    by_name: HashMap<String, Vec<RouteTarget>>,
    /// Whether a source of route names could not be read, so any name may exist.
    unknown: bool,
    /// Whether Symfony's own routes are imported.
    symfony: bool,
    /// YAML aliases: each alias name and the route it stands for.
    aliases: Vec<(String, String)>,
    /// Each attribute route with the file and class declaring it, for the imports
    /// that prefix them.
    attribute_routes: Vec<(PathBuf, String, String, RouteTarget)>,
    /// The YAML route files read, each with the prefix it was read under, so an
    /// import cycle reads each once.
    read_files: Vec<(PathBuf, String)>,
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
        for file in php_route_files(&config, filter) {
            routes.read_php(index, &file);
        }
        routes
            .read_files
            .extend(files.iter().map(|file| (file.clone(), String::new())));
        for file in &files {
            routes.read_yaml(index, config_files, file, &files, &config, "");
        }
        // An alias leads where its route does; an alias of nothing known is unread.
        for (alias, name) in std::mem::take(&mut routes.aliases) {
            let targets = routes.targets(&name).to_vec();
            if targets.is_empty() {
                routes.unknown = true;
            }
            for target in targets {
                routes.add(alias.clone(), target);
            }
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
            let variants = route_variants(&method.metadata);
            let mut defaults = 0;
            let mut names = Vec::new();
            let mut extra = Vec::new();
            let mut localized = false;
            for (attribute, (locales, aliases)) in attributes.iter().zip(&variants) {
                let name = match attribute {
                    RouteName::Literal(name) => format!("{prefix}{name}"),
                    RouteName::Missing => {
                        defaults += 1;
                        format!("{prefix}{}", default_name(fqn, &method.name, defaults - 1))
                    }
                    RouteName::Unreadable => {
                        self.unknown = true;
                        continue;
                    }
                };
                // A localized route is `name.locale` for each locale; `name` still
                // generates it in the current locale.
                localized |= !locales.is_empty();
                extra.extend(locales.iter().map(|locale| format!("{name}.{locale}")));
                extra.extend(aliases.iter().cloned());
                names.push(name);
            }
            count += attributes.len();
            if method.name.eq_ignore_ascii_case("__invoke") {
                invoke_routes.extend(names.iter().cloned());
            }
            if attributes.len() == 1 && names.len() == 1 && !localized {
                self.add(format!("{fqn}::{}", method.name), target(entry));
            }
            for name in names.into_iter().chain(extra) {
                self.attribute_routes.push((
                    path.to_path_buf(),
                    fqn.clone(),
                    name.clone(),
                    target(entry),
                ));
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
                self.attribute_routes.push((
                    path.to_path_buf(),
                    fqn.clone(),
                    name.clone(),
                    target(invoke),
                ));
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
        config_files: &ConfigFiles,
        file: &Path,
        files: &[PathBuf],
        config: &Path,
        prefix: &str,
    ) {
        let Some(parsed) = config_files.get(file) else {
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
                    Import::File(imported, added) => {
                        let prefix = format!("{prefix}{added}");
                        let key = (imported.clone(), prefix.clone());
                        if !self.read_files.contains(&key) {
                            self.read_files.push(key);
                            self.read_yaml(index, config_files, &imported, files, config, &prefix);
                        }
                    }
                    Import::Prefixed(resource, added) => {
                        self.prefix_attribute_routes(&resource, &format!("{prefix}{added}"))
                    }
                }
                continue;
            }
            if let Some(alias) = value.get("alias").and_then(Yaml::text) {
                self.aliases
                    .push((format!("{prefix}{name}"), alias.to_string()));
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
            let full = format!("{prefix}{name}");
            self.add(
                full.clone(),
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
                self.add(full, target);
            }
        }
    }

    /// The routes and aliases a PHP route file adds through its configurator; an
    /// import or a computed name leaves route names unknown.
    fn read_php(&mut self, index: &Index, file: &Path) {
        let Ok(source) = std::fs::read(file) else {
            self.unknown = true;
            return;
        };
        let found = php_routes(&summarize(&source), &source);
        self.unknown |= found.unknown;
        for (name, start, end, controller) in found.routes {
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
            if let Some(target) = controller.and_then(|c| controller_target(index, &c)) {
                self.add(name, target);
            }
        }
        self.aliases.extend(found.aliases);
    }

    /// Adds `prefix` before the names of the attribute routes `resource` reads.
    fn prefix_attribute_routes(&mut self, resource: &AttributeResource, prefix: &str) {
        let prefixed: Vec<(String, RouteTarget)> = self
            .attribute_routes
            .iter()
            .filter(|(path, class, _, _)| match resource {
                AttributeResource::Directory(dir) => path.starts_with(dir),
                AttributeResource::Class(fqn) => class.eq_ignore_ascii_case(fqn),
            })
            .map(|(_, _, name, target)| (format!("{prefix}{name}"), target.clone()))
            .collect();
        for (name, target) in prefixed {
            self.add(name, target);
        }
    }
}

/// What the import `value` of the route file `file` loads.
fn import(index: &Index, file: &Path, value: &Yaml, files: &[PathBuf], config: &Path) -> Import {
    // A prefix the module cannot read leaves the imported names unknown.
    let prefix = match value.get("name_prefix") {
        None => None,
        Some(prefix) => match prefix.text() {
            Some(prefix) => Some(prefix.to_string()),
            None => return Import::Unknown,
        },
    };
    let resource = value.get("resource");
    let Some(resource) = resource
        .and_then(Yaml::text)
        .or_else(|| resource?.get("path")?.text())
    else {
        return Import::Unknown;
    };
    if resource == "routing.controllers" {
        return prefix.map_or(Import::Read, |_| Import::Unknown);
    }
    if SYMFONY_RESOURCES.iter().any(|r| resource.starts_with(r)) {
        return prefix.map_or(Import::Symfony, |_| Import::Unknown);
    }
    if resource.starts_with('@') || resource.contains(['*', '?', '{', '[']) {
        return Import::Unknown;
    }
    let kind = value.get("type").and_then(Yaml::text);
    // Routes in docblock annotations are not read.
    if kind == Some("annotation") {
        return Import::Unknown;
    }
    let project = config.parent().unwrap_or(config);
    let resolved = normalize(&file.parent().unwrap_or(config).join(resource));
    let in_project =
        resolved.starts_with(project) && !resolved.components().any(|c| c.as_os_str() == "vendor");
    if kind == Some("attribute") {
        // Attribute routes of project classes are read wherever they are.
        if resource.contains('\\') && !resource.contains('/') {
            let class = resource.trim_start_matches('\\');
            let project = index
                .classes_named(class)
                .iter()
                .any(|found| index.is_project_code(found.path));
            return match (project, prefix) {
                (false, _) => Import::Unknown,
                (true, None) => Import::Read,
                (true, Some(prefix)) => {
                    Import::Prefixed(AttributeResource::Class(class.to_string()), prefix)
                }
            };
        }
        return match (in_project, prefix) {
            (false, _) => Import::Unknown,
            (true, None) => Import::Read,
            (true, Some(prefix)) => {
                Import::Prefixed(AttributeResource::Directory(resolved), prefix)
            }
        };
    }
    if kind.is_some_and(|k| k != "yaml" && k != "directory") {
        return Import::Unknown;
    }
    if resolved.is_dir() {
        let read = resolved.starts_with(config.join("routes")) && prefix.is_none();
        return if read { Import::Read } else { Import::Unknown };
    }
    match prefix {
        None if files.contains(&resolved) => Import::Read,
        // A route file read only through this import, or again under a prefix.
        prefix if in_project && is_yaml(&resolved) && resolved.is_file() => {
            Import::File(resolved, prefix.unwrap_or_default())
        }
        _ => Import::Unknown,
    }
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

/// Whether `config` holds route files the module does not read: XML files below
/// `config/routes`.
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
                matches!(kind, Some("xml"))
            }
        })
    }
    below(&config.join("routes"))
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

/// `routes.php` in `config`, then the PHP files directly in `config/routes`, in path
/// order.
fn php_route_files(config: &Path, filter: &PathFilter) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(config.join("routes"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "php"))
        .collect();
    files.sort();
    let main = config.join("routes.php");
    if main.is_file() {
        files.insert(0, main);
    }
    files.retain(|path| filter.in_scope(path));
    files
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
