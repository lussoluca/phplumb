//! Broken Drupal references: service ids, route names and plugin ids written in PHP
//! that are defined nowhere on disk. Any doubt stays silent.

use std::path::Path;

use super::routes::{route_methods, takes_route};
use super::services::in_tests;
use super::{Drupal, plugins};
use crate::diagnostics::{Diagnostic, Severity};
use crate::frameworks::ancestry::is_a;
use crate::frameworks::{CallTarget, FoundCall, FrameworkCall, mentions, mentions_member};
use crate::index::Index;
use crate::infer;
use crate::summary::FileSummary;

/// Containers whose `get(id)` takes a service id. A plain PSR container may be any
/// other container.
const CONTAINERS: [&str; 2] = [
    "Symfony\\Component\\DependencyInjection\\ContainerInterface",
    "Drupal\\Component\\DependencyInjection\\ContainerInterface",
];

/// Routes Drupal defines without a routing file entry.
const SPECIAL_ROUTES: [&str; 5] = ["<front>", "<none>", "<nolink>", "<current>", "<button>"];

/// Prefixes of routes that entity route providers and views build at run time.
const GENERATED_ROUTES: [&str; 2] = ["entity.", "view."];

/// Exceptions a missing service, route or plugin throws: a file that mentions one
/// may catch it, so its lookups of that kind tolerate missing ids. Render element
/// types and display options throw nothing and stay checked.
const SERVICE_NOT_FOUND: [&str; 2] = [
    "Symfony\\Component\\DependencyInjection\\Exception\\ServiceNotFoundException",
    "Psr\\Container\\NotFoundExceptionInterface",
];
const ROUTE_NOT_FOUND: [&str; 1] =
    ["Symfony\\Component\\Routing\\Exception\\RouteNotFoundException"];
const PLUGIN_NOT_FOUND: [&str; 2] = [
    "Drupal\\Component\\Plugin\\Exception\\PluginNotFoundException",
    "Drupal\\Component\\Plugin\\Exception\\PluginException",
];

pub(super) fn find(
    drupal: &Drupal,
    index: &Index,
    path: &Path,
    source: &[u8],
    summary: &FileSummary,
) -> Vec<Diagnostic> {
    // Ids defined in excluded files are unknown.
    if index.has_exclusions() {
        return Vec::new();
    }
    // Test code mocks containers, routes and plugin managers; running the tests
    // catches its mistakes.
    if in_tests(index.filter().root(), &index.filter().under_root(path)) {
        return Vec::new();
    }
    let catches = |exceptions: &[&str]| mentions(summary, exceptions);
    let (services_caught, routes_caught, plugins_caught) = (
        catches(&SERVICE_NOT_FOUND),
        catches(&ROUTE_NOT_FOUND),
        catches(&PLUGIN_NOT_FOUND),
    );
    let mut names = vec![
        "service",
        "get",
        "has",
        "hasService",
        "hasDefinition",
        "createInstance",
        "getDefinition",
        "create",
        "addConstraint",
    ];
    names.extend(route_methods());
    let calls = if mentions_member(summary, &names) {
        infer::calls_named(index, path, summary, source, &names)
    } else {
        Vec::new()
    };
    let guarded = |method: &str, id: &str| {
        calls.iter().any(|call| {
            call.strings.first().cloned().flatten().as_deref() == Some(id)
                && call.targets.iter().any(|t| matches!(t, CallTarget::Method { name, .. } if name.eq_ignore_ascii_case(method)))
        })
    };
    let mut found = Vec::new();
    for call in &calls {
        let framework = FrameworkCall {
            targets: &call.targets,
            strings: &call.strings,
            receiver: Some(&call.receiver),
        };
        if let Some((id, range)) = service_id(index, call)
            && !services_caught
            && !soft(call)
            && !guarded("has", id)
            && !guarded("hasService", id)
            && !drupal.defines_service(index, id)
        {
            found.push(diagnostic(
                range,
                Severity::Error,
                format!("Undefined service '{id}'"),
            ));
        }
        for (position, name) in call.strings.iter().enumerate() {
            let (Some(name), Some(Some(range))) = (name, call.ranges.get(position)) else {
                continue;
            };
            if takes_route(index, &call.targets, position)
                && !routes_caught
                && !SPECIAL_ROUTES.contains(&name.as_str())
                && !GENERATED_ROUTES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                && !drupal.defines_route(name)
            {
                found.push(diagnostic(
                    *range,
                    Severity::Error,
                    format!("Undefined route '{name}'"),
                ));
            }
            if let Some(manager) = drupal.plugins.call_manager(index, &framework, position)
                && !plugins_caught
                && !soft(call)
                && let Some(message) =
                    undefined_plugin(drupal, &manager, name, call_method(call), &guarded)
            {
                found.push(diagnostic(*range, Severity::Error, message));
            }
        }
    }
    for keyed in infer::keyed_strings(index, path, summary, source) {
        let call = keyed.keyed.call.as_ref().map(|argument| {
            (
                FrameworkCall {
                    targets: &argument.targets,
                    strings: &argument.strings,
                    receiver: Some(&argument.receiver),
                },
                argument.position,
            )
        });
        let Some(manager) = plugins::keyed_manager(
            index,
            &keyed.keyed.key,
            call.as_ref().map(|(call, position)| (call, *position)),
        ) else {
            continue;
        };
        if let Some(message) = undefined_plugin(drupal, manager, &keyed.keyed.value, None, &guarded)
        {
            found.push(diagnostic(
                (keyed.start, keyed.end),
                Severity::Warning,
                message,
            ));
        }
    }
    found
}

/// The service id a call reads and its range: `\Drupal::service(id)` or a Drupal
/// container's `get(id)`, with a literal id.
fn service_id<'a>(index: &Index, call: &'a FoundCall) -> Option<(&'a str, (u32, u32))> {
    let id = call.strings.first()?.as_deref()?;
    let range = (*call.ranges.first()?)?;
    let reads = call.targets.iter().any(|target| match target {
        CallTarget::Method { class, name } => {
            (class.eq_ignore_ascii_case("Drupal") && name.eq_ignore_ascii_case("service"))
                || (name.eq_ignore_ascii_case("get") && is_a(index, class, &CONTAINERS))
        }
        CallTarget::Function(_) => false,
    });
    reads.then_some((id, range))
}

/// The message for a plugin id `manager` does not define, unless the file checks it
/// with `hasDefinition()`, the manager may find it its own way through `method` (the
/// method called, if any), or, for a render element, another element manager
/// defines it.
fn undefined_plugin(
    drupal: &Drupal,
    manager: &str,
    id: &str,
    method: Option<&str>,
    guarded: &dyn Fn(&str, &str) -> bool,
) -> Option<String> {
    let custom_lookup = drupal
        .plugin_types()
        .iter()
        .any(|t| t.manager.eq_ignore_ascii_case(manager) && t.finds_its_own_way(method));
    if guarded("hasDefinition", id) || custom_lookup || drupal.defines_plugin(manager, id) {
        return None;
    }
    // Other element managers, such as webform's, define `#type` values too.
    let other_element = manager.eq_ignore_ascii_case(plugins::ELEMENTS)
        && drupal.plugin_types().iter().any(|t| {
            t.manager.ends_with("ElementManager")
                && !t.manager.eq_ignore_ascii_case(plugins::ELEMENTS)
                && drupal.defines_plugin(&t.manager, id)
        });
    if other_element {
        return None;
    }
    let short = manager.rsplit('\\').next().unwrap_or(manager);
    Some(format!("Undefined plugin '{id}' of {short}"))
}

/// Whether a call looks an id up softly: a container's `get()` or a plugin manager's
/// `getDefinition()` with a second argument, which may ask for no exception.
fn soft(call: &FoundCall) -> bool {
    call.shapes.len() > 1
        && call_method(call).is_some_and(|m| {
            m.eq_ignore_ascii_case("get") || m.eq_ignore_ascii_case("getDefinition")
        })
}

/// The method name a call resolves to.
fn call_method(call: &FoundCall) -> Option<&str> {
    call.targets.iter().find_map(|target| match target {
        CallTarget::Method { name, .. } => Some(name.as_str()),
        CallTarget::Function(_) => None,
    })
}

fn diagnostic((start, end): (u32, u32), severity: Severity, message: String) -> Diagnostic {
    Diagnostic {
        start,
        end,
        severity,
        message,
    }
}
