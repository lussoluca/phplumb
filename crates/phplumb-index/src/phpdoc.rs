//! PHPDoc tags and type expressions, reduced to what navigation needs.

use serde::{Deserialize, Serialize};

/// A type as navigation needs it. Names are fully qualified, without a leading
/// backslash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeExpr {
    Named(String),
    /// `self`
    SelfType,
    /// `static` and `$this`
    StaticType,
    /// `parent`
    ParentType,
    /// `X[]`, `array<X>`, `array<K, X>`, `iterable<X>`, `list<X>`
    ArrayOf(Box<TypeExpr>),
    /// A class written with arguments, such as `Collection<int, Item>`: the
    /// arguments in order, `Other` for scalars and `null`.
    Generic {
        class: String,
        args: Vec<TypeExpr>,
    },
    /// `array{key: A, B}`: each entry's key, the position for unkeyed ones, and type.
    Shape(Vec<(String, TypeExpr)>),
    /// `callable(A, B): R` or `Closure(A, B): R`: the parameter types.
    Callable(Vec<TypeExpr>),
    /// A scalar or pseudo-type written as a generic argument, such as the `int` of
    /// `Collection<int, Item>`, kept for display; read like `Other` elsewhere.
    Scalar(String),
    /// `class-string<T>`: the name of a class, not an object of it.
    ClassString(Box<TypeExpr>),
    /// Unions and intersections: navigation treats both as "any of these".
    Union(Vec<TypeExpr>),
    /// A class carrying a framework tag, such as a storage that knows its entity
    /// type. Everything that reads classes from a type treats it as its class.
    Tagged {
        class: String,
        tag: String,
    },
    /// Scalars, `mixed`, `null`, shapes, callables and anything unknown.
    Other,
}

/// Resolves a class name as written in a file (not fully qualified) to its fully
/// qualified form.
pub trait NameContext {
    fn resolve_class(&self, name: &str) -> String;
    /// Whether the first segment of the class name `name` is imported.
    fn is_imported(&self, _name: &str) -> bool {
        false
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocBlock {
    /// Text before the first tag, lines joined with spaces.
    pub summary: String,
    /// `@var Type [$name]`, names without `$`.
    pub var: Vec<(Option<String>, TypeExpr)>,
    /// `@param Type $name`, names without `$`.
    pub params: Vec<(String, TypeExpr)>,
    /// `@return Type`
    pub returns: Option<TypeExpr>,
    /// `@template` names, resolved like class names so that they equal the
    /// `Named` types the class's members carry.
    pub templates: Vec<String>,
    /// The generic types of `@extends`, `@implements` and `@use`, as class and
    /// arguments.
    pub generic_supertypes: Vec<(String, Vec<TypeExpr>)>,
}

const NON_CLASS: &[&str] = &[
    "int",
    "integer",
    "string",
    "bool",
    "boolean",
    "float",
    "double",
    "array",
    "mixed",
    "void",
    "false",
    "true",
    "object",
    "callable",
    "iterable",
    "never",
    "resource",
    "scalar",
    "numeric",
    "list",
    "non-empty-array",
    "non-empty-list",
    "non-empty-string",
    "positive-int",
    "negative-int",
    "class-string",
    "array-key",
    "key-of",
    "value-of",
    "closed-resource",
    "literal-string",
    "callable-string",
    "numeric-string",
    "lowercase-string",
    "noreturn",
];

const ARRAY_LIKE: &[&str] = &[
    "array",
    "iterable",
    "list",
    "non-empty-array",
    "non-empty-list",
];

pub fn parse_type(text: &str, names: &dyn NameContext) -> TypeExpr {
    let mut parser = TypeParser {
        text: text.trim().as_bytes(),
        pos: 0,
        names,
        found: Vec::new(),
        generic_depth: 0,
    };
    finish(parser.union())
}

/// The classes a type names, with their byte ranges in `text`: the names
/// `parse_type` resolves to `TypeExpr::Named`.
fn type_class_names(text: &str, names: &dyn NameContext) -> Vec<(usize, usize, String)> {
    let lead = text.len() - text.trim_start().len();
    let mut parser = TypeParser {
        text: text.trim().as_bytes(),
        pos: 0,
        names,
        found: Vec::new(),
        generic_depth: 0,
    };
    parser.union();
    parser
        .found
        .into_iter()
        .map(|(start, end, fqn)| (lead + start, lead + end, fqn))
        .collect()
}

/// Parsed form before `null` is folded away.
enum Parsed {
    Type(TypeExpr),
    Null,
}

fn finish(parsed: Parsed) -> TypeExpr {
    match parsed {
        Parsed::Type(ty) => ty,
        Parsed::Null => TypeExpr::Other,
    }
}

struct TypeParser<'a> {
    text: &'a [u8],
    pos: usize,
    names: &'a dyn NameContext,
    /// The byte range and name of each class the type names.
    found: Vec<(usize, usize, String)>,
    /// How many generic argument lists the parser is inside.
    generic_depth: usize,
}

impl TypeParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_spaces();
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn union(&mut self) -> Parsed {
        let mut parts = vec![self.intersection()];
        while self.eat(b'|') {
            parts.push(self.intersection());
        }
        merge(parts)
    }

    fn intersection(&mut self) -> Parsed {
        let mut parts = vec![self.postfix()];
        while self.peek_intersection() {
            self.pos += 1;
            parts.push(self.postfix());
        }
        merge(parts)
    }

    /// `&` followed by a type, not `&$name` or `&...` in a parameter tag.
    fn peek_intersection(&mut self) -> bool {
        self.skip_spaces();
        self.peek() == Some(b'&') && !matches!(self.text.get(self.pos + 1), Some(b'$') | Some(b'.'))
    }

    fn postfix(&mut self) -> Parsed {
        let mut parsed = self.primary();
        loop {
            self.skip_spaces();
            if self.text[self.pos..].starts_with(b"[]") {
                self.pos += 2;
                parsed = match parsed {
                    Parsed::Type(ty) => Parsed::Type(array_of(ty)),
                    Parsed::Null => Parsed::Type(TypeExpr::Other),
                };
            } else {
                return parsed;
            }
        }
    }

    fn primary(&mut self) -> Parsed {
        self.skip_spaces();
        match self.peek() {
            Some(b'?') => {
                self.pos += 1;
                self.postfix()
            }
            Some(b'(') => {
                self.pos += 1;
                let inner = self.union();
                // `(Subject is [not] Type ? A : B)` is either branch.
                if self.keyword("is") {
                    self.keyword("not");
                    self.union();
                    self.eat(b'?');
                    let yes = self.union();
                    self.eat(b':');
                    let no = self.union();
                    self.eat(b')');
                    return merge(vec![yes, no]);
                }
                self.eat(b')');
                inner
            }
            Some(b'$') => {
                let word = self.word();
                Parsed::Type(if word == "$this" {
                    TypeExpr::StaticType
                } else {
                    TypeExpr::Other
                })
            }
            Some(b) if b == b'\\' || b == b'_' || b.is_ascii_alphabetic() => self.named(),
            Some(b'"') | Some(b'\'') => {
                self.skip_quoted();
                Parsed::Type(TypeExpr::Other)
            }
            Some(_) => {
                self.word();
                Parsed::Type(TypeExpr::Other)
            }
            None => Parsed::Type(TypeExpr::Other),
        }
    }

    fn named(&mut self) -> Parsed {
        let start = self.pos;
        let word = self.word();
        // `Foo::BAR` and `Foo::BAR_*` are constant values, not objects of `Foo`.
        if self.text[self.pos..].starts_with(b"::") {
            let end = self.pos;
            let lower = word.to_ascii_lowercase();
            if !matches!(lower.as_str(), "self" | "static" | "parent") {
                let fqn = match word.strip_prefix('\\') {
                    Some(qualified) => qualified.to_string(),
                    None => self.names.resolve_class(&word),
                };
                self.found.push((start, end, fqn));
            }
            self.pos += 2;
            while self
                .peek()
                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'*')
            {
                self.pos += 1;
            }
            return Parsed::Type(TypeExpr::Other);
        }
        let end = self.pos;
        let recorded = self.found.len();
        self.skip_spaces();
        let args = match self.peek() {
            Some(b'<') => {
                self.pos += 1;
                Some(self.generic_args())
            }
            Some(b'{') => {
                if ARRAY_LIKE.contains(&word.to_ascii_lowercase().as_str()) {
                    self.pos += 1;
                    return Parsed::Type(TypeExpr::Shape(self.shape_entries()));
                }
                self.skip_balanced(b'{', b'}');
                return Parsed::Type(TypeExpr::Other);
            }
            Some(b'(') => {
                let lower = word.trim_start_matches('\\').to_ascii_lowercase();
                let params = if lower == "callable" || lower == "closure" {
                    self.pos += 1;
                    Some(self.callable_params())
                } else {
                    self.skip_balanced(b'(', b')');
                    None
                };
                if self.eat(b':') {
                    self.postfix();
                }
                return Parsed::Type(params.map_or(TypeExpr::Other, TypeExpr::Callable));
            }
            _ => None,
        };

        let lower = word.to_ascii_lowercase();
        // `int<0, max>` bounds are numbers, `min` and `max`, never classes.
        if lower == "int" || lower == "integer" {
            self.found.truncate(recorded);
        }
        if ARRAY_LIKE.contains(&lower.as_str()) {
            return Parsed::Type(match args.and_then(|mut args| args.pop()) {
                Some(Parsed::Type(element)) => array_of(element),
                _ => TypeExpr::Other,
            });
        }
        Parsed::Type(match lower.as_str() {
            "null" => return Parsed::Null,
            "self" => TypeExpr::SelfType,
            "static" => TypeExpr::StaticType,
            "parent" => TypeExpr::ParentType,
            "class-string" => match args.map(|mut args| (args.len(), args.pop())) {
                Some((1, Some(Parsed::Type(class)))) => TypeExpr::ClassString(Box::new(class)),
                _ => TypeExpr::Other,
            },
            // A generic argument keeps its scalar as written, for display.
            _ if NON_CLASS.contains(&lower.as_str()) && self.generic_depth > 0 => {
                TypeExpr::Scalar(word.clone())
            }
            _ if NON_CLASS.contains(&lower.as_str()) => TypeExpr::Other,
            _ => {
                let fqn = match word.strip_prefix('\\') {
                    Some(qualified) => qualified.to_string(),
                    None => self.names.resolve_class(&word),
                };
                self.found.push((start, end, fqn.clone()));
                match args {
                    Some(args) => TypeExpr::Generic {
                        class: fqn,
                        args: args.into_iter().map(finish).collect(),
                    },
                    None => TypeExpr::Named(fqn),
                }
            }
        })
    }

    /// The parameter types of `callable(A, B $name, ...)`, after its `(`, up to and
    /// including its `)`.
    fn callable_params(&mut self) -> Vec<TypeExpr> {
        let mut params = Vec::new();
        loop {
            self.skip_spaces();
            if self.peek().is_none() || self.eat(b')') {
                return params;
            }
            params.push(finish(self.union()));
            // A name, a variadic or by-reference marker, or a default marker.
            self.skip_spaces();
            while matches!(self.peek(), Some(b'$' | b'.' | b'&' | b'=')) {
                if self.peek() == Some(b'$') {
                    self.word();
                } else {
                    self.pos += 1;
                }
                self.skip_spaces();
            }
            if !self.eat(b',') {
                self.eat(b')');
                return params;
            }
        }
    }

    /// The entries of `array{key: A, B, ...}` after its `{`, up to and including its
    /// `}`: keyed entries under their key, the others under their position.
    fn shape_entries(&mut self) -> Vec<(String, TypeExpr)> {
        let mut entries = Vec::new();
        let mut position = 0;
        loop {
            self.skip_spaces();
            if self.peek().is_none() || self.eat(b'}') {
                return entries;
            }
            // An unsealed shape, `...` or `...<K, V>`, says nothing more.
            if self.text[self.pos..].starts_with(b"...") {
                self.pos += 3;
                while self.peek().is_some_and(|b| b != b'}') {
                    if self.peek() == Some(b'<') {
                        self.skip_balanced(b'<', b'>');
                    } else {
                        self.pos += 1;
                    }
                }
                self.eat(b'}');
                return entries;
            }
            let key = self.shape_key().unwrap_or_else(|| {
                position += 1;
                (position - 1).to_string()
            });
            entries.push((key, finish(self.union())));
            if !self.eat(b',') {
                self.eat(b'}');
                return entries;
            }
        }
    }

    /// A shape key followed by `:` or `?:`, consumed; nothing consumed when the
    /// entry has no key.
    fn shape_key(&mut self) -> Option<String> {
        let start = self.pos;
        let key = match self.peek() {
            Some(quote @ (b'\'' | b'"')) => {
                let end = self.text[start + 1..]
                    .iter()
                    .position(|b| *b == quote)
                    .map(|i| start + 1 + i)?;
                self.pos = end + 1;
                String::from_utf8_lossy(&self.text[start + 1..end]).into_owned()
            }
            Some(b) if b.is_ascii_alphanumeric() || b == b'_' => {
                while self
                    .peek()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    self.pos += 1;
                }
                String::from_utf8_lossy(&self.text[start..self.pos]).into_owned()
            }
            _ => return None,
        };
        self.skip_spaces();
        if self.peek() == Some(b'?') {
            self.pos += 1;
            self.skip_spaces();
        }
        if self.peek() == Some(b':') && self.text.get(self.pos + 1) != Some(&b':') {
            self.pos += 1;
            return Some(key);
        }
        self.pos = start;
        None
    }

    /// Consumes `word` when it comes next as a whole word.
    fn keyword(&mut self, word: &str) -> bool {
        self.skip_spaces();
        let rest = &self.text[self.pos..];
        let whole = rest.starts_with(word.as_bytes())
            && rest
                .get(word.len())
                .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
        if whole {
            self.pos += word.len();
        }
        whole
    }

    fn generic_args(&mut self) -> Vec<Parsed> {
        self.generic_depth += 1;
        let mut args = vec![self.union()];
        while self.eat(b',') {
            args.push(self.union());
        }
        self.eat(b'>');
        self.generic_depth -= 1;
        args
    }

    fn word(&mut self) -> String {
        let start = self.pos;
        if self.peek() == Some(b'$') {
            self.pos += 1;
        }
        while self
            .peek()
            .is_some_and(|b| b == b'\\' || b == b'_' || b == b'-' || b.is_ascii_alphanumeric())
        {
            self.pos += 1;
        }
        if self.pos == start {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.text[start..self.pos]).into_owned()
    }

    fn skip_balanced(&mut self, open: u8, close: u8) {
        let mut depth = 0;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    fn skip_quoted(&mut self) {
        let quote = self.peek();
        self.pos += 1;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if Some(b) == quote {
                return;
            }
        }
    }
}

/// An array of `element`, or `Other` when the element is unknown.
pub(crate) fn array_of(element: TypeExpr) -> TypeExpr {
    match element {
        TypeExpr::Other => TypeExpr::Other,
        element => TypeExpr::ArrayOf(Box::new(element)),
    }
}

/// Folds a union: nested unions flatten, `null` and non-class members drop, and a
/// single remaining member stands alone.
fn merge(parts: Vec<Parsed>) -> Parsed {
    if parts.len() == 1 {
        return parts.into_iter().next().expect("one part");
    }
    let mut members: Vec<TypeExpr> = Vec::new();
    for part in parts {
        match part {
            Parsed::Null | Parsed::Type(TypeExpr::Other) => {}
            Parsed::Type(TypeExpr::Union(inner)) => members.extend(inner),
            Parsed::Type(ty) => members.push(ty),
        }
    }
    members.dedup();
    Parsed::Type(match members.len() {
        0 => TypeExpr::Other,
        1 => members.pop().expect("one member"),
        _ => TypeExpr::Union(members),
    })
}

/// Splits the leading type off a tag's text, keeping spaces inside brackets.
fn split_type(text: &str) -> (&str, &str) {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '<' | '{' | '(' | '[' => depth += 1,
            '>' | '}' | ')' | ']' => depth -= 1,
            // The return type of `callable(T): R` follows its colon.
            c if c.is_whitespace() && depth <= 0 && !text[..i].ends_with("):") => {
                return (&text[..i], text[i..].trim_start());
            }
            _ => {}
        }
    }
    (text, &text[text.len()..])
}

/// A `$name` at the start of `text`, without the `$`, `&` or `...`.
fn variable_name(text: &str) -> Option<String> {
    let text = text.trim_start_matches(['&', '.']);
    let name = text.strip_prefix('$')?;
    let end = name
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(name.len());
    (end > 0).then(|| name[..end].to_string())
}

/// Tags whose leading type names classes.
const TYPE_TAGS: &[&str] = &[
    "var",
    "param",
    "return",
    "throws",
    "property",
    "property-read",
    "property-write",
    "mixin",
    "extends",
    "implements",
    "use",
    "template-extends",
    "template-implements",
    "template-use",
    "assert",
];

/// Tags whose leading value may name a class, a member or a function.
const SEE_TAGS: &[&str] = &["see", "uses", "covers"];

/// Tags whose bound after `of` names classes.
const TEMPLATE_TAGS: &[&str] = &["template", "template-covariant", "template-contravariant"];

/// The class names a docblock writes in type positions and as `@see`-like values:
/// byte ranges relative to `text`, fully qualified names without a leading
/// backslash.
pub fn doc_class_names(text: &str, names: &dyn NameContext) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    for (start, line) in doc_lines(text) {
        let Some((tag, rest)) = doc_tag(line) else {
            continue;
        };
        let rest_start = start + offset_in(line, rest);
        for type_text in type_positions(tag, rest) {
            let type_start = rest_start + offset_in(rest, type_text);
            found.extend(
                type_class_names(type_text, names)
                    .into_iter()
                    .map(|(s, e, fqn)| (type_start + s, type_start + e, fqn)),
            );
        }
    }
    found
}

/// How a `@see`-like value names a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeeKind {
    /// `Class::name()`
    Method,
    /// `Class::$name`
    Property,
    /// `Class::NAME`: a method or a constant.
    MethodOrConstant,
}

/// A member or function a `@see`-like value names, as written, with byte ranges
/// relative to the docblock text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SeeName<'a> {
    /// The range covers the member name, `$` included.
    Member {
        class: &'a str,
        name: &'a str,
        start: usize,
        end: usize,
        kind: SeeKind,
    },
    /// `name()`, the range covering the name.
    Function {
        name: &'a str,
        start: usize,
        end: usize,
    },
}

/// The members and functions the `@see`-like values of a docblock name.
pub(crate) fn doc_see_names(text: &str) -> Vec<SeeName<'_>> {
    let mut found = Vec::new();
    for (start, line) in doc_lines(text) {
        let Some((tag, rest)) = doc_tag(line) else {
            continue;
        };
        if !SEE_TAGS.contains(&tag) {
            continue;
        }
        let Some(value) = see_value(rest) else {
            continue;
        };
        let value_start = start + offset_in(line, value);
        match value.split_once("::") {
            Some((class, member)) if is_name(class) => {
                let written = member.strip_suffix("()").unwrap_or(member);
                let (name, kind) = match written.strip_prefix('$') {
                    Some(name) => (name, SeeKind::Property),
                    None if written.len() < member.len() => (written, SeeKind::Method),
                    None => (written, SeeKind::MethodOrConstant),
                };
                if is_identifier(name) {
                    let start = value_start + offset_in(value, written);
                    found.push(SeeName::Member {
                        class,
                        name,
                        start,
                        end: start + written.len(),
                        kind,
                    });
                }
            }
            None => {
                if let Some(name) = value.strip_suffix("()").filter(|name| is_name(name)) {
                    found.push(SeeName::Function {
                        name,
                        start: value_start,
                        end: value_start + name.len(),
                    });
                }
            }
            Some(_) => {}
        }
    }
    found
}

/// The leading value of a `@see`-like tag when it is a name: `Name`, `name()`,
/// `Name::member`, `Name::member()` or `Name::$member`, without one trailing `,` or
/// `.`. A file name, a URL or any other text gives none.
fn see_value(rest: &str) -> Option<&str> {
    let value = split_type(rest).0;
    let value = value.strip_suffix([',', '.']).unwrap_or(value);
    let named = match value.split_once("::") {
        Some((class, member)) => {
            let member = member.strip_suffix("()").unwrap_or(member);
            is_name(class) && is_identifier(member.strip_prefix('$').unwrap_or(member))
        }
        None => is_name(value.strip_suffix("()").unwrap_or(value)),
    };
    named.then_some(value)
}

/// Whether `text` is a PHP identifier.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Whether `text` is a class or function name, qualified or not.
fn is_name(text: &str) -> bool {
    let text = text.strip_prefix('\\').unwrap_or(text);
    !text.is_empty() && text.split('\\').all(is_identifier)
}

/// The content of each docblock line with its byte offset in `text`: without the
/// leading `/**` or `*`, the closing `*/` and surrounding whitespace.
fn doc_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').map(move |raw| {
        let start = offset;
        offset += raw.len();
        let mut line = raw.trim_end_matches(['\n', '\r']);
        line = line.trim_start();
        if let Some(rest) = line.strip_prefix("/**") {
            line = rest;
        } else if !line.starts_with("*/")
            && let Some(rest) = line.strip_prefix('*')
        {
            line = rest;
        }
        line = line.trim_start();
        line = line.strip_suffix("*/").unwrap_or(line).trim_end();
        (start + offset_in(raw, line), line)
    })
}

/// A tag line's tag, without a `phpstan-` or `psalm-` prefix, and the text after it.
fn doc_tag(line: &str) -> Option<(&str, &str)> {
    let tag_line = line.strip_prefix('@')?;
    let (tag, rest) = tag_line
        .split_once(char::is_whitespace)
        .unwrap_or((tag_line, &tag_line[tag_line.len()..]));
    let tag = tag
        .strip_prefix("phpstan-")
        .or_else(|| tag.strip_prefix("psalm-"))
        .unwrap_or(tag);
    Some((tag, rest.trim_start()))
}

/// The slices of a tag's text that hold types naming classes.
fn type_positions<'a>(tag: &str, rest: &'a str) -> Vec<&'a str> {
    if TYPE_TAGS.contains(&tag) {
        return vec![split_type(rest).0];
    }
    if SEE_TAGS.contains(&tag) {
        return match see_value(rest) {
            Some(value) if !value.ends_with("()") || value.contains("::") => vec![value],
            _ => Vec::new(),
        };
    }
    if TEMPLATE_TAGS.contains(&tag) {
        return rest
            .split_once(" of ")
            .map(|(_, bound)| vec![split_type(bound.trim_start()).0])
            .unwrap_or_default();
    }
    if tag == "method" {
        return method_types(rest);
    }
    Vec::new()
}

/// The return type and parameter types of a `@method` tag's text.
fn method_types(rest: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let (mut first, mut after) = split_type(rest);
    // `static` is a modifier when a return type or the name follows it.
    if first == "static" && !after.is_empty() && !after.starts_with('(') {
        (first, after) = split_type(after);
    }
    let signature = if first.contains('(') {
        first
    } else {
        found.push(first);
        after
    };
    let Some(open) = signature.find('(') else {
        return found;
    };
    let mut depth = 0i32;
    let mut part_start = open + 1;
    for (i, c) in signature.char_indices().skip_while(|(i, _)| *i <= open) {
        match c {
            '<' | '{' | '(' | '[' => depth += 1,
            ')' if depth == 0 => {
                found.extend(parameter_type(&signature[part_start..i]));
                break;
            }
            '>' | '}' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                found.extend(parameter_type(&signature[part_start..i]));
                part_start = i + 1;
            }
            _ => {}
        }
    }
    found
}

/// The type of one `@method` parameter, none when it has no type.
fn parameter_type(parameter: &str) -> Option<&str> {
    let parameter = parameter.trim_start();
    let ty = split_type(parameter).0;
    (!ty.is_empty() && !ty.starts_with(['$', '&', '.'])).then_some(ty)
}

/// The byte offset of `inner`, a slice of `outer`, in `outer`.
fn offset_in(outer: &str, inner: &str) -> usize {
    inner.as_ptr() as usize - outer.as_ptr() as usize
}

pub fn parse_docblock(text: &str, names: &dyn NameContext) -> DocBlock {
    let body = text.trim().trim_start_matches("/**").trim_end_matches("*/");
    let lines = body.lines().map(|line| {
        let line = line.trim();
        line.strip_prefix('*').map(str::trim_start).unwrap_or(line)
    });

    let mut summary: Vec<&str> = Vec::new();
    let mut in_summary = true;
    // Plain tags first, then the analyzer-specific ones, which win.
    let mut plain = DocBlock::default();
    let mut specific = DocBlock::default();
    // Analyzers read `@template` and `@phpstan-template` as one list, in order.
    let mut templates: Vec<String> = Vec::new();
    for line in lines {
        let Some(tag_line) = line.strip_prefix('@') else {
            if in_summary && !line.is_empty() {
                summary.push(line);
            } else if in_summary && !summary.is_empty() {
                in_summary = false;
            }
            continue;
        };
        in_summary = false;
        let (tag, rest) = tag_line
            .split_once(char::is_whitespace)
            .unwrap_or((tag_line, ""));
        let (target, tag) = match tag
            .strip_prefix("phpstan-")
            .or_else(|| tag.strip_prefix("psalm-"))
        {
            Some(tag) => (&mut specific, tag),
            None => (&mut plain, tag),
        };
        let (type_text, after) = split_type(rest.trim());
        match tag {
            "var" => target
                .var
                .push((variable_name(after), parse_type(type_text, names))),
            "param" => {
                if let Some(name) = variable_name(after) {
                    target.params.push((name, parse_type(type_text, names)));
                }
            }
            "return" => target.returns = Some(parse_type(type_text, names)),
            "template" | "template-covariant" | "template-contravariant" => {
                let name = names.resolve_class(type_text);
                if !type_text.is_empty() && !templates.contains(&name) {
                    templates.push(name);
                }
            }
            "extends"
            | "implements"
            | "use"
            | "template-extends"
            | "template-implements"
            | "template-use" => {
                if let TypeExpr::Generic { class, args } = parse_type(type_text, names) {
                    target.generic_supertypes.push((class, args));
                }
            }
            _ => {}
        }
    }

    let mut params = plain.params;
    for (name, ty) in specific.params {
        params.retain(|(existing, _)| *existing != name);
        params.push((name, ty));
    }
    DocBlock {
        summary: summary.join(" "),
        var: if specific.var.is_empty() {
            plain.var
        } else {
            specific.var
        },
        params,
        returns: specific.returns.or(plain.returns),
        templates,
        generic_supertypes: merge_supertypes(plain.generic_supertypes, specific.generic_supertypes),
    }
}

/// The generic parents of both lists, the analyzer-specific one winning for a
/// class both name.
fn merge_supertypes(
    plain: Vec<(String, Vec<TypeExpr>)>,
    specific: Vec<(String, Vec<TypeExpr>)>,
) -> Vec<(String, Vec<TypeExpr>)> {
    let mut merged = plain;
    for (class, args) in specific {
        merged.retain(|(known, _)| !known.eq_ignore_ascii_case(&class));
        merged.push((class, args));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{DocBlock, NameContext, TypeExpr, parse_docblock, parse_type};

    struct Names;

    impl NameContext for Names {
        fn resolve_class(&self, name: &str) -> String {
            match name {
                "Alias" => "Lib\\Thing".to_string(),
                other => format!("App\\{other}"),
            }
        }
    }

    fn ty(text: &str) -> TypeExpr {
        parse_type(text, &Names)
    }

    fn named(fqn: &str) -> TypeExpr {
        TypeExpr::Named(fqn.to_string())
    }

    #[test]
    fn named_and_qualified() {
        assert_eq!(ty("Foo"), named("App\\Foo"));
        assert_eq!(ty("\\Lib\\Bar"), named("Lib\\Bar"));
        assert_eq!(ty("Alias"), named("Lib\\Thing"));
    }

    #[test]
    fn nullable_and_null_unions() {
        assert_eq!(ty("?Foo"), named("App\\Foo"));
        assert_eq!(ty("Foo|null"), named("App\\Foo"));
        assert_eq!(ty("null|Foo"), named("App\\Foo"));
    }

    #[test]
    fn unions_and_intersections() {
        let both = TypeExpr::Union(vec![named("App\\Foo"), named("Lib\\Bar")]);
        assert_eq!(ty("Foo|\\Lib\\Bar"), both);
        assert_eq!(ty("Foo&\\Lib\\Bar"), both);
        assert_eq!(ty("(Foo&\\Lib\\Bar)|null"), both);
    }

    #[test]
    fn arrays_of_classes() {
        let array = TypeExpr::ArrayOf(Box::new(named("App\\Foo")));
        for text in [
            "Foo[]",
            "array<Foo>",
            "array<int, Foo>",
            "iterable<Foo>",
            "list<Foo>",
            "?Foo[]",
        ] {
            assert_eq!(ty(text), array, "{text}");
        }
    }

    #[test]
    fn self_static_this_parent() {
        assert_eq!(ty("self"), TypeExpr::SelfType);
        assert_eq!(ty("static"), TypeExpr::StaticType);
        assert_eq!(ty("$this"), TypeExpr::StaticType);
        assert_eq!(ty("parent"), TypeExpr::ParentType);
    }

    #[test]
    fn scalars_and_shapes_are_other() {
        for text in ["int", "array", "mixed", "string|int", "class-string", ""] {
            assert_eq!(ty(text), TypeExpr::Other, "{text}");
        }
    }

    #[test]
    fn conditional_types_give_both_branches() {
        for text in [
            "(T is null ? Foo : Bar)",
            "($x is string ? Foo : Bar)",
            "(T is not null ? Foo : Bar)",
        ] {
            assert_eq!(ty(text), ty("Foo|Bar"), "{text}");
        }
    }

    #[test]
    fn callables_keep_their_parameter_types() {
        assert_eq!(
            ty("Closure(T, int): bool"),
            TypeExpr::Callable(vec![named("App\\T"), TypeExpr::Other])
        );
        assert_eq!(
            ty("callable(Foo $item): void"),
            TypeExpr::Callable(vec![named("App\\Foo")])
        );
        assert_eq!(ty("\\Closure"), named("Closure"));
    }

    #[test]
    fn callable_params_keep_their_names() {
        let doc = parse_docblock("/**\n * @param Closure(T): bool $p\n */", &Names);
        assert_eq!(
            doc.params,
            [("p".to_string(), TypeExpr::Callable(vec![named("App\\T")]))]
        );
    }

    #[test]
    fn shapes_keep_their_entries() {
        assert_eq!(
            ty("array{startedAt: \\DateTimeImmutable, id?: int}"),
            TypeExpr::Shape(vec![
                ("startedAt".to_string(), named("DateTimeImmutable")),
                ("id".to_string(), TypeExpr::Other),
            ])
        );
        assert_eq!(
            ty("list{Foo, 'x'}"),
            TypeExpr::Shape(vec![
                ("0".to_string(), named("App\\Foo")),
                ("1".to_string(), TypeExpr::Other),
            ])
        );
    }

    #[test]
    fn class_strings_keep_their_class() {
        assert_eq!(
            ty("class-string<Foo>"),
            TypeExpr::ClassString(Box::new(ty("Foo")))
        );
    }

    #[test]
    fn generics_keep_their_arguments() {
        assert_eq!(
            ty("Collection<Foo>"),
            TypeExpr::Generic {
                class: "App\\Collection".to_string(),
                args: vec![named("App\\Foo")],
            }
        );
        assert_eq!(
            ty("\\Lib\\Map<string, Foo>"),
            TypeExpr::Generic {
                class: "Lib\\Map".to_string(),
                args: vec![TypeExpr::Scalar("string".to_string()), named("App\\Foo")],
            }
        );
    }

    #[test]
    fn docblock_tags() {
        let doc = parse_docblock(
            "/**\n * Loads it.\n *\n * @param Foo $a\n * @param int $b\n * @return static\n * @var Foo|null $x\n */",
            &Names,
        );

        assert_eq!(
            doc,
            DocBlock {
                summary: "Loads it.".to_string(),
                var: vec![(Some("x".to_string()), named("App\\Foo"))],
                params: vec![
                    ("a".to_string(), named("App\\Foo")),
                    ("b".to_string(), TypeExpr::Other)
                ],
                returns: Some(TypeExpr::StaticType),
                ..DocBlock::default()
            }
        );
    }

    #[test]
    fn phpstan_tags_win() {
        let doc = parse_docblock(
            "/**\n * @return array\n * @phpstan-return Foo[]\n */",
            &Names,
        );

        assert_eq!(
            doc.returns,
            Some(TypeExpr::ArrayOf(Box::new(named("App\\Foo"))))
        );
    }
}
