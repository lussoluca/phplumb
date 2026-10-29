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

/// Whether a route attribute in `metadata` gives its route aliases.
pub(crate) fn has_route_aliases(metadata: &[Metadata]) -> bool {
    metadata.iter().any(|m| {
        !m.annotation
            && ROUTE_ATTRIBUTES
                .iter()
                .any(|a| m.name.eq_ignore_ascii_case(a))
            && m.argument("alias").is_some()
    })
}

/// The names of the route attributes in `metadata`: the `name` argument, named or at
/// position 1. A class constant gives its string value, read through the index;
/// `self::` is `class`. A `null` name is missing.
pub(crate) fn route_names(metadata: &[Metadata], index: &Index, class: &str) -> Vec<RouteName> {
    metadata
        .iter()
        .filter(|m| {
            !m.annotation
                && ROUTE_ATTRIBUTES
                    .iter()
                    .any(|a| m.name.eq_ignore_ascii_case(a))
        })
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
