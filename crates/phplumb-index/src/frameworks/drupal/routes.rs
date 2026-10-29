//! Drupal routes: every route of every `*.routing.yml` on disk outside `vendor` and
//! test modules.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::services::{Tests, changed_drupal_files, in_tests, project_drupal_files};
use crate::filter::PathFilter;
use crate::frameworks::ancestry::is_a;
use crate::frameworks::attributes::{RouteName, route_names};
use crate::frameworks::yaml::{self, Yaml};
use crate::frameworks::{CallTarget, FrameworkCall, under_any};
use crate::index::{Index, Location};
use crate::summary::MemberKind;

const ROUTING: &str = ".routing.yml";

/// Methods taking a route name, with the class declaring them and the position of
/// the name.
const ROUTE_CALLS: [(&str, &str, usize); 7] = [
    ("Drupal\\Core\\Url", "fromRoute", 0),
    ("Drupal\\Core\\Link", "createFromRoute", 1),
    ("Drupal\\Core\\Form\\FormStateInterface", "setRedirect", 0),
    ("Drupal\\Core\\Controller\\ControllerBase", "redirect", 0),
    ("Drupal\\Core\\Form\\FormBase", "redirect", 0),
    (
        "Drupal\\Core\\Routing\\UrlGeneratorInterface",
        "generateFromRoute",
        0,
    ),
    (
        "Drupal\\Core\\Access\\AccessManagerInterface",
        "checkNamedRoute",
        0,
    ),
];

/// One definition of a route: where its key is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteDefinition {
    pub name: String,
    pub path: PathBuf,
    /// The route key in a routing file, or the method or class a route attribute
    /// is on.
    pub start: u32,
    pub end: u32,
    /// The text at the range.
    pub text: String,
}

#[derive(Default)]
pub(crate) struct Routes {
    by_file: HashMap<PathBuf, Vec<RouteDefinition>>,
    /// The routes each file's aliases point to, which exist wherever they are built.
    alias_targets: HashMap<PathBuf, Vec<String>>,
    /// Routes outside test modules, which navigation uses.
    by_name: HashMap<String, Vec<RouteDefinition>>,
    /// Every route name defined, test modules included.
    defined: HashSet<String>,
    /// Routes declared by attributes on classes and methods.
    attributes: Vec<RouteDefinition>,
    /// Some route attribute names a route with something else than a literal string.
    uncertain: bool,
    root: PathBuf,
}

impl Routes {
    pub(crate) fn read(index: &Index, filter: &PathFilter) -> Routes {
        let mut routes = Routes {
            root: filter.root().to_path_buf(),
            ..Routes::default()
        };
        for path in project_drupal_files(index, filter, ROUTING, Tests::Include) {
            routes.insert(path);
        }
        routes.reindex();
        routes
    }

    /// Re-reads changed routing files and directories; drops removed ones.
    pub(crate) fn update(
        &mut self,
        index: &Index,
        filter: &PathFilter,
        changed: &[PathBuf],
        removed: &[PathBuf],
    ) {
        let gone: HashSet<PathBuf> = removed.iter().chain(changed).cloned().collect();
        self.by_file.retain(|file, _| !under_any(file, &gone));
        self.alias_targets.retain(|file, _| !under_any(file, &gone));
        for file in changed_drupal_files(index, filter, changed, ROUTING, Tests::Include) {
            self.insert(file);
        }
        self.reindex();
    }

    fn insert(&mut self, path: PathBuf) {
        let (routes, aliases) = parse_file(&path);
        self.by_file.insert(path.clone(), routes);
        self.alias_targets.insert(path, aliases);
    }

    /// Reads the routes that attributes on the index's classes and methods declare,
    /// as Drupal's attribute route discovery does: the class attribute's name prefixes
    /// the names of its methods' routes, and names a route itself only on an
    /// invokable class whose methods declare none. A method with one route is also
    /// `Class::method`, and an invokable class with one route is also `Class` and
    /// `Class::__invoke`. A name that is not a literal string makes route names
    /// uncertain.
    pub(crate) fn read_attributes(&mut self, index: &Index, filter: &PathFilter) {
        let mut found = Vec::new();
        let mut uncertain = false;
        for (path, declaration) in index.declarations() {
            if !filter.in_scope(path) {
                continue;
            }
            let Some(shape) = &declaration.class else {
                continue;
            };
            let class_names = route_names(&shape.metadata, index, &declaration.fqn);
            let prefix = match class_names.first() {
                Some(RouteName::Unreadable) => {
                    uncertain = true;
                    continue;
                }
                Some(RouteName::Literal(prefix)) => prefix.as_str(),
                Some(RouteName::Missing) | None => "",
            };
            let fqn = &declaration.fqn;
            let short = fqn.rsplit('\\').next().unwrap_or(fqn);
            let mut routes: Vec<RouteDefinition> = Vec::new();
            let mut aliases: Vec<RouteDefinition> = Vec::new();
            let mut on_invoke = false;
            let methods = shape
                .members
                .iter()
                .filter(|m| m.kind == MemberKind::Method);
            for member in methods.clone() {
                let names = route_names(&member.metadata, index, &declaration.fqn);
                let before = routes.len();
                for name in &names {
                    match name {
                        RouteName::Literal(name) => routes.push(RouteDefinition {
                            name: format!("{prefix}{name}"),
                            path: path.to_path_buf(),
                            start: member.name_start,
                            end: member.name_end,
                            text: member.name.clone(),
                        }),
                        RouteName::Unreadable => uncertain = true,
                        // Drupal rejects a method route without a name.
                        RouteName::Missing => {}
                    }
                }
                on_invoke |= !names.is_empty() && member.name.eq_ignore_ascii_case("__invoke");
                if names.len() == 1 && routes.len() == before + 1 {
                    let mut alias = routes[before].clone();
                    alias.name = format!("{fqn}::{}", member.name);
                    aliases.push(alias);
                }
            }
            let invoke = methods
                .clone()
                .find(|m| m.name.eq_ignore_ascii_case("__invoke"));
            if routes.is_empty()
                && let Some(invoke) = invoke
            {
                for name in &class_names {
                    match name {
                        RouteName::Literal(name) => routes.push(RouteDefinition {
                            name: name.clone(),
                            path: path.to_path_buf(),
                            start: declaration.name_start,
                            end: declaration.name_end,
                            text: short.to_string(),
                        }),
                        RouteName::Unreadable => uncertain = true,
                        RouteName::Missing => {}
                    }
                }
                on_invoke |= !class_names.is_empty();
                if on_invoke && routes.len() == 1 {
                    let mut alias = routes[0].clone();
                    alias.name = format!("{fqn}::{}", invoke.name);
                    aliases.push(alias);
                }
            }
            if on_invoke && routes.len() == 1 {
                let mut alias = routes[0].clone();
                alias.name = fqn.clone();
                aliases.push(alias);
            }
            found.extend(routes);
            found.extend(aliases);
        }
        self.attributes = found;
        self.uncertain = uncertain;
        self.reindex();
    }

    fn reindex(&mut self) {
        self.defined = self
            .by_file
            .values()
            .flatten()
            .chain(&self.attributes)
            .map(|r| r.name.clone())
            .chain(self.alias_targets.values().flatten().cloned())
            .collect();
        let mut by_name: HashMap<String, Vec<RouteDefinition>> = HashMap::new();
        for definition in self
            .by_file
            .values()
            .flatten()
            .chain(&self.attributes)
            .filter(|definition| !in_tests(&self.root, &definition.path))
        {
            by_name
                .entry(definition.name.clone())
                .or_default()
                .push(definition.clone());
        }
        for definitions in by_name.values_mut() {
            definitions.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
        }
        self.by_name = by_name;
    }

    /// Whether some routing file or route attribute defines `name`, test modules
    /// included; always, when a route attribute's name cannot be read.
    pub(crate) fn defines(&self, name: &str) -> bool {
        self.uncertain || self.defined.contains(name)
    }

    pub(crate) fn get(&self, name: &str) -> &[RouteDefinition] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }

    /// The route keys a route name at position `argument` of `call` names, each with
    /// the route name as the expected text.
    pub(crate) fn call_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
        value: &str,
    ) -> Vec<(Location, String)> {
        if !takes_route(index, call.targets, argument) {
            return Vec::new();
        }
        self.targets(value)
    }

    /// Every definition of the route `name`, with the name as the expected text.
    pub(crate) fn targets(&self, name: &str) -> Vec<(Location, String)> {
        self.get(name)
            .iter()
            .map(|route| {
                let location = Location {
                    path: route.path.clone(),
                    start: route.start,
                    end: route.end,
                };
                (location, route.text.clone())
            })
            .collect()
    }
}

/// Whether position `argument` of a call to `targets` takes a route name.
pub(crate) fn takes_route(index: &Index, targets: &[CallTarget], argument: usize) -> bool {
    targets.iter().any(|target| {
        let CallTarget::Method { class, name } = target else {
            return false;
        };
        ROUTE_CALLS.iter().any(|(owner, method, position)| {
            *position == argument
                && name.eq_ignore_ascii_case(method)
                && is_a(index, class, &[owner])
        })
    })
}

/// The names of the methods taking a route name.
pub(crate) fn route_methods() -> Vec<&'static str> {
    ROUTE_CALLS.iter().map(|(_, method, _)| *method).collect()
}

/// The routes of one file, and the routes its aliases point to; nothing when it does
/// not parse.
fn parse_file(path: &Path) -> (Vec<RouteDefinition>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), Vec::new());
    };
    let Some(Yaml::Map(entries)) = yaml::parse(&text) else {
        return (Vec::new(), Vec::new());
    };
    let aliases = entries
        .iter()
        .filter_map(|(_, value)| value.get("alias")?.text().map(str::to_string))
        .collect();
    let routes = entries
        .iter()
        .filter_map(|(key, _)| {
            let Yaml::Scalar { value, start, end } = key else {
                return None;
            };
            if value == "route_callbacks" {
                return None;
            }
            let (start, end) = yaml::value_range(&text, value, *start, *end);
            Some(RouteDefinition {
                name: value.clone(),
                path: path.to_path_buf(),
                start,
                end,
                text: value.clone(),
            })
        })
        .collect();
    (routes, aliases)
}
