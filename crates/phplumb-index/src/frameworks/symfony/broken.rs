//! Broken Symfony references: route, template and parameter names written in PHP,
//! Twig templates and YAML configuration that are defined nowhere. Any doubt stays
//! silent.

use std::path::Path;

use super::twig::{self, TwigName};
use super::{
    AUTOWIRE_ATTRIBUTE, PARAMETER_METHODS, ROUTE_METHODS, Symfony, TEMPLATE_ATTRIBUTE,
    TEMPLATE_METHODS, TemplateStatus, calls_one_of, config,
};
use crate::diagnostics::{Diagnostic, Severity};
use crate::frameworks::yaml::{self, Yaml};
use crate::frameworks::{FrameworkCall, mentions, mentions_member};
use crate::index::Index;
use crate::infer;
use crate::summary::FileSummary;

/// Classes that leave a file's names of one kind unchecked: the exception a
/// missing route, template or parameter throws, which the file may catch, and the
/// classes a file uses to build its own router or Twig environment, as tests do.
const ROUTE_NOT_FOUND: [&str; 3] = [
    "Symfony\\Component\\Routing\\Exception\\RouteNotFoundException",
    "Symfony\\Component\\Routing\\Generator\\UrlGenerator",
    "Symfony\\Component\\Routing\\RouteCollection",
];
const TEMPLATE_NOT_FOUND: [&str; 4] = [
    "Twig\\Error\\LoaderError",
    "Twig\\Loader\\ArrayLoader",
    "Twig\\Loader\\ChainLoader",
    "Twig\\Loader\\FilesystemLoader",
];
const PARAMETER_NOT_FOUND: [&str; 1] =
    ["Symfony\\Component\\DependencyInjection\\Exception\\ParameterNotFoundException"];

pub(super) fn find(
    symfony: &Symfony,
    index: &Index,
    path: &Path,
    source: &[u8],
    summary: &FileSummary,
) -> Vec<Diagnostic> {
    // Names defined in excluded files are unknown.
    if index.has_exclusions() {
        return Vec::new();
    }
    let routes = !mentions(summary, &ROUTE_NOT_FOUND);
    let templates = !mentions(summary, &TEMPLATE_NOT_FOUND);
    let parameters = !mentions(summary, &PARAMETER_NOT_FOUND);
    let names: Vec<&str> = ROUTE_METHODS
        .iter()
        .chain(&TEMPLATE_METHODS)
        .chain(&PARAMETER_METHODS)
        .flat_map(|(_, methods)| methods.iter().copied())
        .collect();
    let mut found = Vec::new();
    let calls = if mentions_member(summary, &names) {
        infer::calls_named(index, path, summary, source, &names)
    } else {
        Vec::new()
    };
    for call in calls {
        let (Some(Some(name)), Some(Some(range))) = (call.strings.first(), call.ranges.first())
        else {
            continue;
        };
        let framework = FrameworkCall {
            targets: &call.targets,
            strings: &call.strings,
            receiver: Some(&call.receiver),
        };
        let calls = |methods: &[(&str, &[&str])]| calls_one_of(index, &framework, methods);
        let message = if routes && calls(&ROUTE_METHODS) && symfony.route_missing(name) {
            format!("Undefined route '{name}'")
        } else if templates
            && calls(&TEMPLATE_METHODS)
            && symfony.template_status(name) == TemplateStatus::Missing
        {
            format!("Undefined template '{name}'")
        } else if parameters && calls(&PARAMETER_METHODS) && symfony.parameter_missing(index, name)
        {
            format!("Undefined parameter '{name}'")
        } else {
            continue;
        };
        found.push(diagnostic(*range, message));
    }
    let attributes = if mentions(summary, &[AUTOWIRE_ATTRIBUTE, TEMPLATE_ATTRIBUTE]) {
        infer::attribute_strings(summary, source)
    } else {
        Vec::new()
    };
    for (attribute, end) in attributes {
        let first = attribute.key.is_none() && attribute.position == 0;
        let key = attribute.key.as_deref();
        if parameters && attribute.attribute.eq_ignore_ascii_case(AUTOWIRE_ATTRIBUTE) {
            if key == Some("param") && symfony.parameter_missing(index, &attribute.value) {
                found.push(diagnostic(
                    (attribute.start, end),
                    format!("Undefined parameter '{}'", attribute.value),
                ));
            }
            if key == Some("value") || first {
                let raw = String::from_utf8_lossy(&source[attribute.start as usize..end as usize]);
                for (name, start, stop) in config::parameters_in(&raw) {
                    if symfony.parameter_missing(index, name) {
                        found.push(diagnostic(
                            (
                                attribute.start + start as u32,
                                attribute.start + stop as u32,
                            ),
                            format!("Undefined parameter '{name}'"),
                        ));
                    }
                }
            }
        }
        let template = key == Some("template") || first;
        if templates
            && template
            && attribute.attribute.eq_ignore_ascii_case(TEMPLATE_ATTRIBUTE)
            && symfony.template_status(&attribute.value) == TemplateStatus::Missing
        {
            found.push(diagnostic(
                (attribute.start, end),
                format!("Undefined template '{}'", attribute.value),
            ));
        }
    }
    found
}

/// Route and template names a template of the configured directories writes as
/// literals; other Twig files, such as test fixtures, belong to other loaders.
pub(super) fn find_in_twig(
    symfony: &Symfony,
    index: &Index,
    path: &Path,
    source: &[u8],
) -> Vec<Diagnostic> {
    if index.has_exclusions() || !symfony.templates.holds(&index.filter().under_root(path)) {
        return Vec::new();
    }
    /// An element of a template list, with whether its template is missing.
    struct Element {
        name: String,
        range: (u32, u32),
        missing: bool,
    }
    let mut found = Vec::new();
    let mut lists: Vec<(usize, Vec<Element>)> = Vec::new();
    for (kind, name, start, end) in twig::names(source) {
        let missing = |name: &str| symfony.template_status(name) == TemplateStatus::Missing;
        let message = match kind {
            TwigName::Route if symfony.route_missing(&name) => format!("Undefined route '{name}'"),
            TwigName::RoutePattern if symfony.routes.pattern_missing(&twig::pattern(&name)) => {
                format!("Undefined route '{name}'")
            }
            TwigName::Template if missing(&name) => format!("Undefined template '{name}'"),
            TwigName::TemplatePattern
                if symfony.templates.pattern_status(&twig::pattern(&name))
                    == TemplateStatus::Missing =>
            {
                format!("Undefined template '{name}'")
            }
            TwigName::Choice(list) => {
                let element = Element {
                    missing: missing(&name),
                    name,
                    range: (start, end),
                };
                match lists.iter_mut().find(|(open, _)| *open == list) {
                    Some((_, elements)) => elements.push(element),
                    None => lists.push((list, vec![element])),
                }
                continue;
            }
            _ => continue,
        };
        found.push(diagnostic((start, end), message));
    }
    for (_, elements) in lists {
        if elements.iter().all(|element| element.missing) {
            for element in elements {
                let message = format!("Undefined template '{}'", element.name);
                found.push(diagnostic(element.range, message));
            }
        }
    }
    found
}

/// Parameters written as `%name%` in YAML configuration under `config`.
pub(super) fn find_in_yaml(
    symfony: &Symfony,
    index: &Index,
    path: &Path,
    source: &[u8],
) -> Vec<Diagnostic> {
    // Elsewhere, such as in translations, `%name%` is a placeholder.
    let config = symfony.project.dir.join("config");
    if index.has_exclusions() || !index.filter().under_root(path).starts_with(&config) {
        return Vec::new();
    }
    let Ok(text) = std::str::from_utf8(source) else {
        return Vec::new();
    };
    let Some(root) = yaml::parse(text) else {
        return Vec::new();
    };
    let mut scalars = Vec::new();
    collect_scalars(&root, &mut scalars);
    let mut found = Vec::new();
    for (start, end) in scalars {
        let raw = &text[start as usize..end as usize];
        for (name, from, to) in config::parameters_in(raw) {
            if symfony.parameter_missing(index, name) {
                found.push(diagnostic(
                    (start + from as u32, start + to as u32),
                    format!("Undefined parameter '{name}'"),
                ));
            }
        }
    }
    found
}

/// The range of every scalar below `node`, keys included.
fn collect_scalars(node: &Yaml, found: &mut Vec<(u32, u32)>) {
    match node {
        Yaml::Scalar { start, end, .. } => found.push((*start, *end)),
        Yaml::Map(entries) => {
            for (key, value) in entries {
                collect_scalars(key, found);
                collect_scalars(value, found);
            }
        }
        Yaml::Sequence(items) => {
            for item in items {
                collect_scalars(item, found);
            }
        }
        Yaml::Other => {}
    }
}

fn diagnostic((start, end): (u32, u32), message: String) -> Diagnostic {
    Diagnostic {
        start,
        end,
        severity: Severity::Error,
        message,
    }
}
