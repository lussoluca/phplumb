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

/// What a route attribute gives besides its name.
pub(crate) struct RouteVariant {
    /// The locales of a path written as a map from locale to path.
    pub locales: Vec<String>,
    /// The aliases it gives its route.
    pub aliases: Vec<String>,
    /// Whether an alias is not a string the module reads.
    pub unreadable: bool,
}

/// The locales and aliases of the route attributes in `metadata`, in attribute order
/// like `route_names`, with class constants read as `route_names` reads them.
pub(crate) fn route_variants(
    metadata: &[Metadata],
    index: &Index,
    class: &str,
) -> Vec<RouteVariant> {
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
            let values: Vec<&MetaValue> = match attribute.argument("alias") {
                None => Vec::new(),
                Some(MetaValue::List(entries)) => entries.iter().map(|(_, value)| value).collect(),
                Some(value) => vec![value],
            };
            let mut variant = RouteVariant {
                locales,
                aliases: Vec::new(),
                unreadable: false,
            };
            for value in values {
                match literal(value, index, class) {
                    RouteName::Literal(alias) => variant.aliases.push(alias),
                    _ => variant.unreadable = true,
                }
            }
            variant
        })
        .collect()
}

/// The string `value` stands for: a literal, or a class constant's value read
/// through the index, `self::` being `class`.
fn literal(value: &MetaValue, index: &Index, class: &str) -> RouteName {
    match value {
        MetaValue::String(value) => RouteName::Literal(value.clone()),
        MetaValue::Constant { class: owner, name } => {
            let owner = if owner == "self" { class } else { owner };
            index
                .find_member(owner, MemberKind::Constant, name)
                .first()
                .and_then(|(_, member)| member.value.clone())
                .map_or(RouteName::Unreadable, RouteName::Literal)
        }
        _ => RouteName::Unreadable,
    }
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
                Some(MetaValue::Null) | None => RouteName::Missing,
                Some(name) => literal(name, index, class),
            }
        })
        .collect()
}
