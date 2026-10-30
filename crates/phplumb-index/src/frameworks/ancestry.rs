//! Whether a class descends from another, through parents and interfaces, and the
//! methods a class has through its parents and traits.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::index::Index;
use crate::summary::{Declaration, Member, MemberKind};

/// Whether `class` is, extends or implements one of `ancestors`.
pub(crate) fn is_a(index: &Index, class: &str, ancestors: &[&str]) -> bool {
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending = vec![class.to_string()];
    while let Some(next) = pending.pop() {
        if ancestors.iter().any(|c| c.eq_ignore_ascii_case(&next)) {
            return true;
        }
        if !seen.insert(next.to_ascii_lowercase()) {
            continue;
        }
        for supertype in index.supertypes(&next) {
            pending.push(supertype.declaration.fqn.clone());
        }
    }
    false
}

/// The methods of `class` as reflection lists them: its own, then those of its
/// traits and parents that it does not redeclare, each with its file and the class
/// `self::` names in it, the declaring class or the class using the trait.
pub(crate) fn methods_of<'a>(
    index: &'a Index,
    path: &Path,
    class: &'a Declaration,
) -> Vec<(PathBuf, String, &'a Member)> {
    let mut found: Vec<(PathBuf, String, &Member)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut pending: Vec<(PathBuf, &Declaration, String)> =
        vec![(path.to_path_buf(), class, class.fqn.clone())];
    let mut visited = 0;
    while let Some((path, declaration, owner)) = pending.pop() {
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
                found.push((path.clone(), owner.clone(), member));
            }
        }
        // Traits last, so they come off the stack before the parent.
        if let Some(parent) = shape
            .parent
            .iter()
            .find_map(|name| index.classes_named(name).into_iter().next())
        {
            let owner = parent.declaration.fqn.clone();
            pending.push((parent.path.to_path_buf(), parent.declaration, owner));
        }
        for name in shape.traits.iter().rev() {
            if let Some(used) = index.classes_named(name).into_iter().next() {
                pending.push((used.path.to_path_buf(), used.declaration, owner.clone()));
            }
        }
    }
    found
}
