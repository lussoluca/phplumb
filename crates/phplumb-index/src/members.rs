//! Member lookup through the class hierarchy.

use std::collections::{HashSet, VecDeque};

use crate::index::{ClassRef, Index};
use crate::summary::{Member, MemberKind};

/// Every declaration of `name` of `kind` visible on `class`, one per declaration of
/// `class` in the index, deduplicated. The lookup order per declaration: its own
/// members, its traits (recursively), its parent chain (each with its traits), then
/// all interfaces reached on the way (recursively). `Constant` also finds enum cases.
pub(crate) fn find_member<'a>(
    index: &'a Index,
    class: &str,
    kind: MemberKind,
    name: &str,
) -> Vec<(ClassRef<'a>, &'a Member)> {
    let mut found: Vec<(ClassRef<'a>, &'a Member)> = Vec::new();
    for root in index.classes_named(class) {
        if let Some(hit) = search(index, root, kind, name) {
            let duplicate = found
                .iter()
                .any(|(c, m)| c.path == hit.0.path && m.name_start == hit.1.name_start);
            if !duplicate {
                found.push(hit);
            }
        }
    }
    found
}

/// The nearest declaration of member `name` above `class` whose type is known:
/// along the parent chain first, then through the interfaces. Members documented
/// with `{@inheritdoc}` and no type of their own take this one's type.
pub(crate) fn typed_ancestor<'a>(
    index: &'a Index,
    class: &str,
    kind: MemberKind,
    name: &str,
) -> Option<(ClassRef<'a>, &'a Member)> {
    let mut visited: HashSet<String> = HashSet::new();
    typed_above(index, class, kind, name, &mut visited)
}

fn typed_above<'a>(
    index: &'a Index,
    class: &str,
    kind: MemberKind,
    name: &str,
    visited: &mut HashSet<String>,
) -> Option<(ClassRef<'a>, &'a Member)> {
    if !visited.insert(class.to_ascii_lowercase()) {
        return None;
    }
    let declaration = index.classes_named(class).into_iter().next()?;
    let shape = declaration.declaration.class.as_ref()?;
    for ancestor in shape.parent.iter().chain(shape.interfaces.iter()) {
        for (found, member) in find_member(index, ancestor, kind, name) {
            // A private member is not inherited, so its type says nothing here.
            if member.is_private {
                continue;
            }
            // A type that names no class, such as a callable, says nothing here.
            if member
                .ty
                .as_ref()
                .is_some_and(|ty| !crate::infer::names_no_class(ty))
            {
                return Some((found, member));
            }
            if let Some(hit) = typed_above(index, &found.declaration.fqn, kind, name, visited) {
                return Some(hit);
            }
        }
    }
    None
}

/// The declaration of member `name` that `class` itself uses, following the
/// lookup order from that declaration rather than from its name.
pub(crate) fn member_from<'a>(
    index: &'a Index,
    class: ClassRef<'a>,
    kind: MemberKind,
    name: &str,
) -> Option<(ClassRef<'a>, &'a Member)> {
    search(index, class, kind, name)
}

fn search<'a>(
    index: &'a Index,
    root: ClassRef<'a>,
    kind: MemberKind,
    name: &str,
) -> Option<(ClassRef<'a>, &'a Member)> {
    // Declarations, not names, are visited: a name may be declared more than once
    // (a stub and the real class), and each declaration has its own members.
    let mut visited: HashSet<usize> = HashSet::new();
    let mut interfaces: VecDeque<String> = VecDeque::new();

    let mut chain: VecDeque<ClassRef<'a>> = VecDeque::from([root]);
    while let Some(class) = chain.pop_front() {
        if !visited.insert(identity(&class)) {
            continue;
        }
        if let Some(member) = own(&class, kind, name) {
            return Some((class, member));
        }
        if let Some(hit) = in_traits(index, &class, kind, name, &mut visited) {
            return Some(hit);
        }
        let Some(shape) = class.declaration.class.as_ref() else {
            continue;
        };
        interfaces.extend(shape.interfaces.iter().cloned());
        if let Some(parent) = shape.parent.as_deref() {
            chain.extend(index.classes_named(parent));
        }
    }

    while let Some(interface) = interfaces.pop_front() {
        for declaration in index.classes_named(&interface) {
            if !visited.insert(identity(&declaration)) {
                continue;
            }
            if let Some(member) = own(&declaration, kind, name) {
                return Some((declaration, member));
            }
            if let Some(shape) = &declaration.declaration.class {
                interfaces.extend(shape.interfaces.iter().cloned());
            }
        }
    }
    None
}

fn in_traits<'a>(
    index: &'a Index,
    class: &ClassRef<'a>,
    kind: MemberKind,
    name: &str,
    visited: &mut HashSet<usize>,
) -> Option<(ClassRef<'a>, &'a Member)> {
    let shape = class.declaration.class.as_ref()?;
    for name_of_trait in &shape.traits {
        for declaration in index.classes_named(name_of_trait) {
            if !visited.insert(identity(&declaration)) {
                continue;
            }
            if let Some(member) = own(&declaration, kind, name) {
                return Some((declaration, member));
            }
            if let Some(hit) = in_traits(index, &declaration, kind, name, visited) {
                return Some(hit);
            }
        }
    }
    None
}

/// One declaration's identity within the index.
fn identity(class: &ClassRef<'_>) -> usize {
    std::ptr::from_ref(class.declaration) as usize
}

/// A member declared directly on `class`.
fn own<'a>(class: &ClassRef<'a>, kind: MemberKind, name: &str) -> Option<&'a Member> {
    class
        .declaration
        .class
        .as_ref()?
        .members
        .iter()
        .find(|member| matches(member, kind, name))
}

/// Whether `member` is the `kind` member `name`. Methods match case-insensitively;
/// properties, constants and enum cases match exactly.
pub(crate) fn matches(member: &Member, kind: MemberKind, name: &str) -> bool {
    match (kind, member.kind) {
        (MemberKind::Method, MemberKind::Method) => member.name.eq_ignore_ascii_case(name),
        (MemberKind::Property, MemberKind::Property) => member.name == name,
        (MemberKind::Constant, MemberKind::Constant | MemberKind::EnumCase)
        | (MemberKind::EnumCase, MemberKind::EnumCase) => member.name == name,
        _ => false,
    }
}
