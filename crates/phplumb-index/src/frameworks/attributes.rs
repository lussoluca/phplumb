//! Route names written in attributes, as both Drupal and Symfony read them.

use crate::index::Index;
use crate::metadata::{MetaValue, Metadata};
use crate::summary::MemberKind;

/// Route attribute classes, whose `name` argument (named or at position 1) names a
/// route.
const ROUTE_ATTRIBUTES: [&str; 2] = [
    "Symfony\\Component\\Routing\\Attribute\\Route",
    "Symfony\\Component\\Routing\\Annotation\\Route",
];

/// The name a route attribute gives.
pub(crate) enum RouteName {
    Literal(String),
    /// A constant or an expression.
    Unreadable,
    Missing,
}

/// The locales of a route attribute whose path is a map from locale to path, and the
/// aliases it gives its route, each list in attribute order like `route_names`.
pub(crate) fn route_variants(metadata: &[Metadata]) -> Vec<(Vec<String>, Vec<String>)> {
    route_attributes(metadata)
        .map(|attribute| {
            let path = attribute.argument("path").or_else(|| {
                attribute
                    .arguments
                    .first()
                    .filter(|(key, _)| key.is_none())
                    .map(|(_, value)| value)
            });
            let locales = match path {
                Some(MetaValue::List(entries)) => {
                    entries.iter().filter_map(|(key, _)| key.clone()).collect()
                }
                _ => Vec::new(),
            };
            let aliases = match attribute.argument("alias") {
                Some(MetaValue::String(alias)) => vec![alias.clone()],
                Some(MetaValue::List(entries)) => entries
                    .iter()
                    .filter_map(|(_, value)| match value {
                        MetaValue::String(alias) => Some(alias.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            (locales, aliases)
        })
        .collect()
}

/// The route attributes in `metadata`.
fn route_attributes(metadata: &[Metadata]) -> impl Iterator<Item = &Metadata> {
    metadata.iter().filter(|m| {
        !m.annotation
            && ROUTE_ATTRIBUTES
                .iter()
                .any(|a| m.name.eq_ignore_ascii_case(a))
    })
}

/// The names of the route attributes in `metadata`: the `name` argument, named or at
/// position 1. A class constant gives its string value, read through the index;
/// `self::` is `class`. A `null` name is missing.
pub(crate) fn route_names(metadata: &[Metadata], index: &Index, class: &str) -> Vec<RouteName> {
    route_attributes(metadata)
        .map(|attribute| {
            let name = attribute.argument("name").or_else(|| {
                attribute
                    .arguments
                    .get(1)
                    .filter(|(key, _)| key.is_none())
                    .map(|(_, value)| value)
            });
            match name {
                Some(MetaValue::String(name)) => RouteName::Literal(name.clone()),
                Some(MetaValue::Constant { class: owner, name }) => {
                    let owner = if owner == "self" { class } else { owner };
                    index
                        .find_member(owner, MemberKind::Constant, name)
                        .first()
                        .and_then(|(_, member)| member.value.clone())
                        .map_or(RouteName::Unreadable, RouteName::Literal)
                }
                Some(MetaValue::Null) | None => RouteName::Missing,
                Some(_) => RouteName::Unreadable,
            }
        })
        .collect()
}
