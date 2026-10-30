//! Symfony routes: the names `#[Route]` attributes give, as Symfony's attribute
//! loader and FrameworkBundle name them, and the routes of YAML and PHP route files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::bundles;
use super::config::top_blocks;
use super::doctrine::elements;
use super::files::ConfigFiles;

use crate::filter::{PathFilter, is_yaml};
use crate::frameworks::Project;
use crate::frameworks::ancestry::methods_of;
use crate::frameworks::attributes::{RouteName, route_names, route_variants};
use crate::frameworks::yaml::{self, Yaml};
use crate::index::{Index, Location};
use crate::infer::php_routes;
use crate::metadata::Metadata;
use crate::summary::{Declaration, Member, MemberKind, SymbolKind, summarize};

/// Where a route name leads, with the text expected at the range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    pub location: Location,
    pub text: String,
}

/// Route resources of Symfony's bundles, whose routes all have names starting
/// with `_`.
const SYMFONY_RESOURCES: [&str; 4] = [
    "@FrameworkBundle/",
    "@WebProfilerBundle/",
    "@SecurityBundle/",
    "@TwigBundle/",
];

/// The route loader of the security firewalls' logout paths.
const LOGOUT_LOADER: &str = "security.route_loader.logout";

/// What a route import loads, as far as the module knows.
enum Import {
    /// Routes the module reads anyway: attribute routes of project code.
    Read,
    /// YAML, XML and PHP route files, with the prefix the import and the imports
    /// leading to it give their names.
    Files(Vec<PathBuf>, String),
    /// Attribute routes of project classes in a directory, or of one class, under
    /// the prefix the import and the imports leading to it give their names.
    Prefixed(AttributeResource, String),
    /// Routes of one of Symfony's bundles the module cannot read.
    Symfony,
    /// The logout routes of the security firewalls.
    Logout,
    Unknown,
}

/// The classes an attribute route import reads.
enum AttributeResource {
    Directory(PathBuf),
    Class(String),
}

/// What reading route files needs.
struct Sources<'a> {
    index: &'a Index,
    config_files: &'a ConfigFiles,
    /// The project's `config` directory.
    config: &'a Path,
    /// The configuration files of the project.
    configuration: &'a [PathBuf],
    /// Whether configuration the module does not read exists, which may add
    /// security firewalls.
    configuration_unread: bool,
}

#[derive(Default)]
pub(crate) struct Routes {
    by_name: HashMap<String, Vec<RouteTarget>>,
    /// Whether a source of route names could not be read, so any name may exist.
    unknown: bool,
    /// Whether Symfony's own routes are imported.
    symfony: bool,
    /// Aliases of route files: each alias name and the route it stands for.
    aliases: Vec<(String, String)>,
    /// Each attribute route with the file and class declaring it, for the imports
    /// that prefix them.
    attribute_routes: Vec<(PathBuf, String, String, RouteTarget)>,
    /// The route files read, by real path, each with the prefix it was read under,
    /// so a file imported again under the same prefix is read once.
    read_files: Vec<(PathBuf, String)>,
    /// The route files being read, by real path, outermost first.
    reading: Vec<PathBuf>,
}

impl Routes {
    /// Every route of the project: attribute routes of project classes, then the
    /// route files the kernel loads and the files they import. `configuration` are
    /// the configuration files of the project, and `configuration_unread` says
    /// whether others the module does not read exist.
    pub(crate) fn read(
        index: &Index,
        filter: &PathFilter,
        project: &Project,
        config_files: &ConfigFiles,
        configuration: &[PathBuf],
        configuration_unread: bool,
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
        routes.unknown |= has_unread_route_files(&config);
        let sources = Sources {
            index,
            config_files,
            config: &config,
            configuration,
            configuration_unread,
        };
        let files = php_route_files(&config, filter)
            .into_iter()
            .chain(route_files(&config, filter));
        for file in files {
            routes.read_file(&sources, &file, "");
        }
        routes.resolve_aliases();
        routes
    }

    /// Leaves every route name unknown, for a source of routes the module does not
    /// read.
    pub(crate) fn mark_unknown(&mut self) {
        self.unknown = true;
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

    /// Gives each alias the targets of the route it stands for, also through other
    /// aliases; an alias of nothing known leaves names unknown.
    fn resolve_aliases(&mut self) {
        let mut pending = std::mem::take(&mut self.aliases);
        loop {
            let before = pending.len();
            pending.retain(|(alias, name)| {
                let targets = self.targets(name).to_vec();
                for target in &targets {
                    self.add(alias.clone(), target.clone());
                }
                targets.is_empty()
            });
            if pending.len() == before {
                break;
            }
        }
        self.unknown |= !pending.is_empty();
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
        let methods = methods_of(index, path, class);
        let mut count = 0;
        let mut invoke_routes: Vec<String> = Vec::new();
        for (file, owner, method) in &methods {
            let target = method_target(file, method);
            let attributes = Attributes {
                metadata: &method.metadata,
                owner,
                prefix: &prefix,
                method: &method.name,
            };
            let (added, names) = self.add_routes(index, path, fqn, &attributes, &target);
            count += added;
            if method.name.eq_ignore_ascii_case("__invoke") {
                invoke_routes.extend(names.iter().cloned());
            }
            // Symfony aliases `Class::method` to a method's only route.
            if added == 1 && names.len() == 1 {
                self.add(format!("{fqn}::{}", method.name), target);
            }
        }
        let invoke = methods
            .iter()
            .find(|(_, _, m)| m.name.eq_ignore_ascii_case("__invoke"));
        if count == 0
            && let Some((file, _, method)) = invoke
        {
            let attributes = Attributes {
                metadata: &shape.metadata,
                owner: fqn,
                prefix: "",
                method: &method.name,
            };
            let (added, names) =
                self.add_routes(index, path, fqn, &attributes, &method_target(file, method));
            count += added;
            invoke_routes.extend(names);
        }
        if count == 1
            && invoke_routes.len() == 1
            && let Some((file, _, method)) = invoke
        {
            let target = method_target(file, method);
            self.add(fqn.clone(), target.clone());
            self.add(format!("{fqn}::__invoke"), target);
        }
    }

    /// Adds the routes `attributes` give, leading to `target`, for the class `fqn`
    /// declared in `path`; returns how many routes Symfony adds and the name of each
    /// attribute's route.
    fn add_routes(
        &mut self,
        index: &Index,
        path: &Path,
        fqn: &str,
        attributes: &Attributes,
        target: &RouteTarget,
    ) -> (usize, Vec<String>) {
        let Attributes {
            metadata,
            owner,
            prefix,
            method,
        } = *attributes;
        let mut defaults = 0;
        let mut added = 0;
        let mut names = Vec::new();
        let variants = route_variants(metadata, index, owner);
        for (name, variant) in route_names(metadata, index, owner)
            .into_iter()
            .zip(variants)
        {
            added += variant.locales.len().max(1);
            let name = match name {
                RouteName::Literal(name) => format!("{prefix}{name}"),
                RouteName::Missing => {
                    defaults += 1;
                    format!("{prefix}{}", default_name(fqn, method, defaults - 1))
                }
                RouteName::Unreadable => {
                    self.unknown = true;
                    continue;
                }
            };
            self.unknown |= variant.unreadable;
            // A localized route is `name.locale` for each locale; `name` still
            // generates it in the current locale.
            let localized = variant
                .locales
                .iter()
                .map(|locale| format!("{name}.{locale}"));
            let routes: Vec<String> = std::iter::once(name.clone())
                .chain(localized)
                .chain(variant.aliases)
                .collect();
            for route in routes {
                self.attribute_routes.push((
                    path.to_path_buf(),
                    fqn.to_string(),
                    route.clone(),
                    target.clone(),
                ));
                self.add(route, target.clone());
            }
            names.push(name);
        }
        (added, names)
    }

    /// Reads the route file `file` under `prefix`, once for each prefix. A file that
    /// imports itself, directly or not, makes Symfony fail, and leaves names unknown.
    fn read_file(&mut self, sources: &Sources, file: &Path, prefix: &str) {
        let real = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
        if self.reading.contains(&real) {
            self.unknown = true;
            return;
        }
        let key = (real.clone(), prefix.to_string());
        if self.read_files.contains(&key) {
            return;
        }
        self.read_files.push(key);
        self.reading.push(real);
        if is_php(file) {
            self.read_php(sources.index, file, prefix);
        } else if is_xml(file) {
            self.read_xml(sources, file, prefix);
        } else {
            self.read_yaml(sources, file, prefix);
        }
        self.reading.pop();
    }

    /// The routes of one YAML route file, with `prefix` before their names: top-level
    /// keys with a `path`, also inside `when@<env>` blocks, and aliases; imports
    /// define none, and say whether the routes they load are known.
    fn read_yaml(&mut self, sources: &Sources, file: &Path, prefix: &str) {
        let Some(parsed) = sources.config_files.get(file) else {
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
                self.follow(
                    sources,
                    import(sources, file, &ImportSpec::of(value), prefix),
                );
                continue;
            }
            // A name prefix also prefixes the route an alias stands for.
            if let Some(alias) = value.get("alias").and_then(Yaml::text) {
                self.aliases
                    .push((format!("{prefix}{name}"), format!("{prefix}{alias}")));
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
            if let Some(target) = controller.and_then(|c| controller_target(sources.index, c)) {
                self.add(full, target);
            }
        }
    }

    /// Reads what an import loads.
    fn follow(&mut self, sources: &Sources, import: Import) {
        match import {
            Import::Read => {}
            Import::Symfony => self.symfony = true,
            Import::Logout => {
                // Other firewalls may come from configuration not read.
                self.symfony |= sources.configuration_unread;
                self.read_logout(sources);
            }
            Import::Unknown => self.unknown = true,
            Import::Files(files, prefix) => {
                for imported in &files {
                    self.read_file(sources, imported, &prefix);
                }
            }
            Import::Prefixed(resource, prefix) => self.prefix_attribute_routes(&resource, &prefix),
        }
    }

    /// The routes of one XML route file, with `prefix` before their names: each
    /// `<route id>`, also inside `<when>`, with its `controller` attribute or
    /// `_controller` default, aliases, and imports read as in YAML files.
    fn read_xml(&mut self, sources: &Sources, file: &Path, prefix: &str) {
        let Ok(text) = std::fs::read_to_string(file) else {
            self.unknown = true;
            return;
        };
        // The route whose child elements follow.
        let mut route: Option<String> = None;
        for element in elements(&text) {
            match element.name.as_str() {
                "route" => {
                    route = None;
                    let id = element.attributes.iter().find(|(key, ..)| key == "id");
                    let Some((_, id, start, end)) = id else {
                        self.unknown = true;
                        continue;
                    };
                    let full = format!("{prefix}{id}");
                    if let Some(alias) = element.attribute("alias") {
                        self.aliases.push((full, format!("{prefix}{alias}")));
                        continue;
                    }
                    self.add(
                        full.clone(),
                        RouteTarget {
                            location: Location {
                                path: file.to_path_buf(),
                                start: *start,
                                end: *end,
                            },
                            text: id.clone(),
                        },
                    );
                    let controller = element.attribute("controller");
                    if let Some(target) =
                        controller.and_then(|c| controller_target(sources.index, c))
                    {
                        self.add(full.clone(), target);
                    }
                    route = Some(full);
                }
                "default" if element.attribute("key") == Some("_controller") => {
                    let Some(full) = route.clone() else {
                        continue;
                    };
                    let rest = &text[element.end as usize..];
                    let controller = rest[..rest.find('<').unwrap_or(rest.len())].trim();
                    if let Some(target) = controller_target(sources.index, controller) {
                        self.add(full, target);
                    }
                }
                "import" => {
                    route = None;
                    let spec = ImportSpec {
                        resource: element.attribute("resource"),
                        kind: element.attribute("type"),
                        name_prefix: Some(element.attribute("name-prefix")),
                    };
                    self.follow(sources, import(sources, file, &spec, prefix));
                }
                _ => {}
            }
        }
    }

    /// The routes and aliases a PHP route file adds through its configurator, with
    /// `prefix` before their names; an import, a computed name or a file in another
    /// form leaves route names unknown.
    fn read_php(&mut self, index: &Index, file: &Path, prefix: &str) {
        let Ok(source) = std::fs::read(file) else {
            self.unknown = true;
            return;
        };
        let found = php_routes(&summarize(&source), &source);
        self.unknown |= found.unknown;
        for (name, start, end, controller) in found.routes {
            let full = format!("{prefix}{name}");
            self.add(
                full.clone(),
                RouteTarget {
                    location: Location {
                        path: file.to_path_buf(),
                        start,
                        end,
                    },
                    text: name,
                },
            );
            if let Some(target) = controller.and_then(|c| controller_target(index, &c)) {
                self.add(full, target);
            }
        }
        self.aliases.extend(
            found
                .aliases
                .into_iter()
                .map(|(alias, route)| (format!("{prefix}{alias}"), format!("{prefix}{route}"))),
        );
    }

    /// The route `_logout_<firewall>` of each firewall with a `logout` setting in the
    /// configuration files, leading to that setting.
    fn read_logout(&mut self, sources: &Sources) {
        for file in sources.configuration {
            let Some(parsed) = sources.config_files.get(file) else {
                continue;
            };
            let (text, Some(root)) = (parsed.text.as_str(), &parsed.yaml) else {
                continue;
            };
            for block in top_blocks(root) {
                let firewalls = block.get("security").and_then(|s| s.get("firewalls"));
                let Some(Yaml::Map(firewalls)) = firewalls else {
                    continue;
                };
                for (firewall, settings) in firewalls {
                    let (Some(firewall), Yaml::Map(settings)) = (firewall.text(), settings) else {
                        continue;
                    };
                    for (key, value) in settings {
                        let Yaml::Scalar {
                            value: name,
                            start,
                            end,
                        } = key
                        else {
                            continue;
                        };
                        // Symfony adds a route for a logout path, not for a route name.
                        let path = value.get("path").and_then(Yaml::text);
                        let disabled = value.text() == Some("false");
                        if name != "logout" || disabled || path.is_some_and(|p| !p.starts_with('/'))
                        {
                            continue;
                        }
                        let (start, end) = yaml::value_range(text, name, *start, *end);
                        self.add(
                            format!("_logout_{firewall}"),
                            RouteTarget {
                                location: Location {
                                    path: file.clone(),
                                    start,
                                    end,
                                },
                                text: name.clone(),
                            },
                        );
                    }
                }
            }
        }
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

/// The route attributes of a method, or of an invokable class, as `add_routes`
/// reads them.
#[derive(Clone, Copy)]
struct Attributes<'a> {
    metadata: &'a [Metadata],
    /// The class `self::` names in them.
    owner: &'a str,
    /// The class route's name, before each route name.
    prefix: &'a str,
    /// The method the routes lead to, for default names.
    method: &'a str,
}

/// Where a route leads when it runs `method`, declared in `path`.
fn method_target(path: &Path, method: &Member) -> RouteTarget {
    RouteTarget {
        location: Location {
            path: path.to_path_buf(),
            start: method.name_start,
            end: method.name_end,
        },
        text: method.name.clone(),
    }
}

/// What a route import names: its resource, its `type`, and its name prefix,
/// `Some(None)` when there is none and `None` when the module cannot read it.
struct ImportSpec<'a> {
    resource: Option<&'a str>,
    kind: Option<&'a str>,
    name_prefix: Option<Option<&'a str>>,
}

impl<'a> ImportSpec<'a> {
    /// The import a YAML route file's entry `value` makes.
    fn of(value: &'a Yaml) -> ImportSpec<'a> {
        let resource = value.get("resource");
        ImportSpec {
            resource: resource
                .and_then(Yaml::text)
                .or_else(|| resource?.get("path")?.text()),
            kind: value.get("type").and_then(Yaml::text),
            name_prefix: match value.get("name_prefix") {
                None => Some(None),
                Some(prefix) => prefix.text().map(Some),
            },
        }
    }
}

/// What the import `spec` of the route file `file`, read under the prefix `outer`,
/// loads.
fn import(sources: &Sources, file: &Path, spec: &ImportSpec, outer: &str) -> Import {
    // A prefix the module cannot read leaves the imported names unknown.
    let prefix = match spec.name_prefix {
        Some(None) => outer.to_string(),
        Some(Some(prefix)) => format!("{outer}{prefix}"),
        None => return Import::Unknown,
    };
    let Some(resource) = spec.resource else {
        return Import::Unknown;
    };
    if resource == "routing.controllers" {
        return if prefix.is_empty() {
            Import::Read
        } else {
            Import::Unknown
        };
    }
    if resource == LOGOUT_LOADER {
        return if prefix.is_empty() {
            Import::Logout
        } else {
            Import::Unknown
        };
    }
    let kind = spec.kind;
    if let Some(bundled) = resource.strip_prefix('@') {
        // A bundle's routes the module cannot read: unknown, or Symfony's `_` names.
        let symfony = SYMFONY_RESOURCES.iter().any(|r| resource.starts_with(r));
        let unprefixed = prefix.is_empty();
        let unread = || {
            if symfony && unprefixed {
                Import::Symfony
            } else {
                Import::Unknown
            }
        };
        if resource.contains(['*', '?', '{', '['])
            || kind.is_some_and(|k| !matches!(k, "yaml" | "xml" | "php" | "directory"))
        {
            return unread();
        }
        let Some((bundle, path)) = bundled.split_once('/') else {
            return unread();
        };
        let Some(dir) = bundles::bundle_dir(sources.index, sources.config, bundle) else {
            return unread();
        };
        return resource_files(&normalize(&dir.join(path)))
            .map_or_else(unread, |files| Import::Files(files, prefix));
    }
    if resource.contains(['*', '?', '{', '[']) {
        return Import::Unknown;
    }
    // Routes in docblock annotations are not read.
    if kind == Some("annotation") {
        return Import::Unknown;
    }
    let config = sources.config;
    let project = config.parent().unwrap_or(config);
    let resolved = normalize(&file.parent().unwrap_or(config).join(resource));
    let in_project =
        resolved.starts_with(project) && !resolved.components().any(|c| c.as_os_str() == "vendor");
    if kind == Some("attribute") {
        // Attribute routes of project classes are read wherever they are.
        let attributes = if resource.contains('\\') && !resource.contains('/') {
            let class = resource.trim_start_matches('\\');
            let project = sources
                .index
                .classes_named(class)
                .iter()
                .any(|found| sources.index.is_project_code(found.path));
            if !project {
                return Import::Unknown;
            }
            AttributeResource::Class(class.to_string())
        } else if in_project {
            AttributeResource::Directory(resolved)
        } else {
            return Import::Unknown;
        };
        return if prefix.is_empty() {
            Import::Read
        } else {
            Import::Prefixed(attributes, prefix)
        };
    }
    if !in_project || kind.is_some_and(|k| !matches!(k, "yaml" | "xml" | "php" | "directory")) {
        return Import::Unknown;
    }
    resource_files(&resolved).map_or(Import::Unknown, |files| Import::Files(files, prefix))
}

/// The route files a resource resolved to `path` loads: the file, or those below a
/// directory; none when one is a file the module does not read.
fn resource_files(path: &Path) -> Option<Vec<PathBuf>> {
    if path.is_dir() {
        return route_files_below(path);
    }
    (path.is_file() && is_route_file(path)).then(|| vec![path.to_path_buf()])
}

/// The route files below `dir`, as Symfony's directory loader imports them, in path
/// order and skipping hidden entries; `None` when one is a file the module does not
/// read.
fn route_files_below(dir: &Path) -> Option<Vec<PathBuf>> {
    fn walk(dir: &Path, seen: &mut Vec<PathBuf>, found: &mut Vec<PathBuf>) -> bool {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if seen.contains(&real) {
            return true;
        }
        seen.push(real);
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
            })
            .collect();
        paths.sort();
        paths.iter().all(|path| {
            if path.is_dir() {
                walk(path, seen, found)
            } else if is_route_file(path) {
                found.push(path.clone());
                true
            } else {
                false
            }
        })
    }
    let mut found = Vec::new();
    walk(dir, &mut Vec::new(), &mut found).then_some(found)
}

/// Whether `path` is a route file the module reads: YAML, XML or PHP.
fn is_route_file(path: &Path) -> bool {
    is_yaml(path) || is_xml(path) || is_php(path)
}

/// Whether `path` is an XML file.
fn is_xml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
}

/// Whether `path` is a PHP file.
pub(crate) fn is_php(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("php"))
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

/// `routes.php` in `config` when there is no `routes.yaml`, then the PHP files the
/// kernel imports from `config/routes`.
fn php_route_files(config: &Path, filter: &PathFilter) -> Vec<PathBuf> {
    let main = config.join("routes.php");
    let main = (main.is_file() && !config.join("routes.yaml").is_file()).then_some(main);
    kernel_route_files(config, filter, main, "php")
}

/// `routes.yaml` in `config`, then the YAML files the kernel imports from
/// `config/routes`.
fn route_files(config: &Path, filter: &PathFilter) -> Vec<PathBuf> {
    let main = config.join("routes.yaml");
    kernel_route_files(config, filter, main.is_file().then_some(main), "yaml")
}

/// `main`, then the files with `extension` directly in `config/routes` and in its
/// environment directories, as `MicroKernelTrait` imports them, each once (by real
/// path), in path order.
fn kernel_route_files(
    config: &Path,
    filter: &PathFilter,
    main: Option<PathBuf>,
    extension: &str,
) -> Vec<PathBuf> {
    let routes = config.join("routes");
    let files_in = |dir: &Path| -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
            })
            .collect();
        found.sort();
        found
    };
    let mut environments: Vec<PathBuf> = std::fs::read_dir(&routes)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    environments.sort();
    let mut files: Vec<PathBuf> = main.into_iter().collect();
    files.extend(files_in(&routes));
    for environment in &environments {
        files.extend(files_in(environment));
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    files.retain(|path| {
        let real = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.contains(&real) {
            return false;
        }
        seen.push(real);
        filter.in_scope(path)
    });
    files
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
