//! Hover text: signatures and docblocks of definitions, inferred types of variables.

use std::path::Path;

use crate::index::{Index, Location};
use crate::infer;
use crate::phpdoc::{TypeExpr, parse_docblock};
use crate::stubs::summarize_stub;
use crate::summary::{FileSummary, Member, MemberKind, summarize};

/// Markdown for the word at `start..end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub start: u32,
    pub end: u32,
    pub markdown: String,
}

pub(crate) fn hover(index: &mut Index, path: &Path, source: &[u8], offset: u32) -> Option<Hover> {
    let (start, end) = word_at(source, offset)?;
    let summary = summarize(source);
    // The `$name` of `Class::$name` is a property, not a variable.
    let on_static_member = summary
        .static_refs
        .iter()
        .any(|r| r.start <= offset && offset <= r.end);
    if !on_static_member
        && let Some((name, ty)) = infer::variable_at(index, path, &summary, source, offset)
    {
        return (ty != TypeExpr::Other).then(|| Hover {
            start,
            end,
            markdown: code(&format!("{} ${name}", display(&ty))),
        });
    }
    let targets = index.targets(path, source, offset);
    let blocks: Vec<String> = targets
        .iter()
        .filter_map(|target| describe(index, target))
        .collect();
    (!blocks.is_empty()).then(|| Hover {
        start,
        end,
        markdown: blocks.join("\n\n---\n\n"),
    })
}

/// The signature and docblock summary of the declaration at `target`, read from its
/// file, since the index keeps neither. A stub file is read as the index read it.
fn describe(index: &Index, target: &Location) -> Option<String> {
    let text = std::fs::read(&target.path).ok()?;
    let summary = if index.is_stub(&target.path) {
        summarize_stub(&text)
    } else {
        summarize(&text)
    };
    for declaration in &summary.declarations {
        if declaration.name_start == target.start {
            return Some(block(
                &declaration.signature,
                declaration.doc.as_deref(),
                &summary,
                target.start,
            ));
        }
        let Some(shape) = &declaration.class else {
            continue;
        };
        if let Some(member) = shape.members.iter().find(|m| m.name_start == target.start) {
            return Some(block(
                &qualify(member, &declaration.fqn),
                member.doc.as_deref(),
                &summary,
                target.start,
            ));
        }
    }
    None
}

/// A member signature with its declaring class before the name:
/// `App\Foo::load(int $id): ?static`, `protected ?Bar App\Foo::$bar`.
fn qualify(member: &Member, class: &str) -> String {
    let signature = &member.signature;
    match member.kind {
        MemberKind::Method => match signature.find("function ") {
            Some(i) => format!("{class}::{}", &signature[i + "function ".len()..]),
            None => signature.clone(),
        },
        MemberKind::Property => signature.replacen(
            &format!("${}", member.name),
            &format!("{class}::${}", member.name),
            1,
        ),
        MemberKind::Constant | MemberKind::EnumCase => match signature.rfind(&member.name) {
            Some(i) => format!("{}{class}::{}", &signature[..i], &signature[i..]),
            None => signature.clone(),
        },
    }
}

fn block(signature: &str, doc: Option<&str>, summary: &FileSummary, at: u32) -> String {
    let mut markdown = code(signature);
    if let Some(doc) = doc {
        let text = parse_docblock(doc, &summary.file_names.at(at)).summary;
        if !text.is_empty() {
            markdown.push_str("\n\n");
            markdown.push_str(&text);
        }
    }
    markdown
}

fn code(text: &str) -> String {
    format!("```php\n{text}\n```")
}

fn display(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Named(name) | TypeExpr::Tagged { class: name, .. } => format!("\\{name}"),
        TypeExpr::SelfType => "self".to_string(),
        TypeExpr::StaticType => "static".to_string(),
        TypeExpr::ParentType => "parent".to_string(),
        TypeExpr::ArrayOf(element) => format!("{}[]", display(element)),
        TypeExpr::Generic { class, args } => format!(
            "\\{class}<{}>",
            args.iter().map(display).collect::<Vec<_>>().join(", ")
        ),
        TypeExpr::ClassString(class) => format!("class-string<{}>", display(class)),
        TypeExpr::Scalar(name) => name.clone(),
        TypeExpr::Union(members) => members.iter().map(display).collect::<Vec<_>>().join("|"),
        TypeExpr::Other => "mixed".to_string(),
    }
}

/// The identifier, variable or qualified name around `offset`.
pub(crate) fn word_at(source: &[u8], offset: u32) -> Option<(u32, u32)> {
    let is_word =
        |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'\\' || b >= 0x80;
    let offset = offset as usize;
    let at = if source.get(offset).copied().is_some_and(is_word) {
        offset
    } else if offset > 0 && source.get(offset - 1).copied().is_some_and(is_word) {
        offset - 1
    } else {
        return None;
    };
    let start = source[..at]
        .iter()
        .rposition(|&b| !is_word(b))
        .map_or(0, |i| i + 1);
    let end = source[at..]
        .iter()
        .position(|&b| !is_word(b))
        .map_or(source.len(), |i| at + i);
    Some((start as u32, end as u32))
}
