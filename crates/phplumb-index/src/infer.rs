//! Type inference for the expression under the cursor.
//!
//! One walk of the cursor file collects the facts inference needs, as references
//! into the syntax tree: member accesses, function scopes, control-flow branches,
//! assignments and other rebindings, `foreach` and `catch` bindings, `instanceof`
//! checks and inline `@var` docblocks. Types are inferred on demand, before the
//! tree's arena is dropped.
//!
//! A variable's facts count only in the function, closure or arrow function that
//! holds them. Among them the latest before the use decides, except that a binding
//! inside a branch the use is not in either cannot reach it (a sibling branch) or
//! merges with what came before (code after the branch statement). A binding whose
//! type is unknown makes the merged type unknown: the result is then nothing rather
//! than a guess.
//!
//! A property's type is its declared type plus the types its class and the class's
//! parents assign to it; an assigned value of unknown type, other than a literal or
//! an array, leaves the declared type alone. Those are computed per class, from the
//! class's own code;
//! while one is computed, only the class itself and its ancestors are read, so a
//! result never depends on which class happened to be computed first.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;

use mago_allocator::LocalArena;
use mago_database::file::FileId;
use mago_span::HasSpan;
use mago_syntax::comments::docblock::get_docblock_before_position;
use mago_syntax::cst::{
    Access, AnonymousClass, Argument, ArgumentList, ArrayElement, ArrowFunction, Assignment,
    AssignmentOperator, Attribute, Binary, BinaryOperator, Call, ClassConstantAccess,
    ClassLikeConstantSelector, ClassLikeMemberSelector, Closure, CompositeString, DoWhile,
    Expression, For, Foreach, ForeachTarget, Function, FunctionCall, FunctionLikeParameterList,
    Global, Hint, If, IfBody, InterpolatedString, KeyValueArrayElement, Literal, LiteralString,
    Match, MatchArm, Method, MethodCall, NullSafeMethodCall, NullSafePropertyAccess,
    PartialArgument, Program, PropertyAccess, Static, StaticItem, StaticMethodCall, StringPart,
    Switch, TriviaKind, Try, TryCatchClause, Variable, While,
};
use mago_syntax::parser::parse_file_content;
use mago_syntax::walker::Walker;

use crate::calls::{self, CallKind};
use crate::frameworks::{AttributeString, CallTarget, FoundCall, Framework, FrameworkCall};
use crate::generics::{array_element, element_type, entry_type, substitute, supertype_args};
use crate::index::{Index, Location};
use crate::phpdoc::{TypeExpr, array_of, parse_docblock, parse_type};
use crate::summary::{
    FileSummary, Member, MemberKind, NameKind, NameRef, SymbolKind, pick_type, summarize,
};

const MAX_DEPTH: u32 = 16;

/// How many times a class's assignments are evaluated when properties are assigned
/// from the class's own properties.
const MAX_PASSES: usize = 3;

/// The types a class's code assigns to each of its properties, by property name;
/// `Other` for a property assigned a value of unknown type.
pub(crate) type PropertyTypes = HashMap<String, TypeExpr>;

/// A class whose property assignments are being computed, with the types found by
/// the previous pass, inside the computations it is nested in.
pub(crate) struct Assigning<'s> {
    /// Lowercased.
    class: String,
    current: &'s PropertyTypes,
    outer: Option<&'s Assigning<'s>>,
    /// Whether this pass read the class's own assigned types.
    read_self: Cell<bool>,
    /// Whether the value being evaluated read a property of the class that the
    /// previous pass found no assignment for.
    pending: Cell<bool>,
}

impl Assigning<'_> {
    fn holds(&self, class: &str) -> bool {
        self.class == class || self.outer.is_some_and(|outer| outer.holds(class))
    }
}

/// What a member access's receiver is.
pub(crate) enum Receiver {
    /// The classes the receiver can be; empty when its type is unknown.
    Classes(Vec<String>),
    /// `$this` inside the anonymous class at this index of the summary's class ranges.
    Anonymous(usize),
}

/// The receiver, kind and name of the instance member access under `offset`, when
/// the cursor is on the member name of `$x->name` or `$x?->name()`.
pub(crate) fn member_at(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<(Receiver, MemberKind, String)> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let access = facts
        .accesses
        .iter()
        .chain(&facts.variable_statics)
        .find(|a| a.start <= offset && offset <= a.end)?;
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    let receiver = match ctx.anonymous_this(access.object, access.start) {
        Some(range) => Receiver::Anonymous(range),
        None => Receiver::Classes(class_names(&ctx.expr(access.object, access.start, 0))),
    };
    Some((receiver, access.kind, access.name.clone()))
}

/// A call whose string literal argument covers an offset: what the call targets,
/// its string arguments, the argument's position and its value.
#[derive(Clone)]
pub(crate) struct StringArgument {
    pub targets: Vec<CallTarget>,
    pub strings: Vec<Option<String>>,
    pub receiver: TypeExpr,
    pub position: usize,
    pub value: String,
}

/// The innermost method or static call with a string literal argument covering
/// `offset`.
pub(crate) fn string_argument_at(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<StringArgument> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    calls
        .iter()
        .rev()
        .find_map(|call| ctx.string_argument(call, offset))
}

/// Every attribute of a file.
struct Attributes;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<&'ast Attribute<'arena>>> for Attributes {
    fn walk_in_attribute(
        &self,
        attribute: &'ast Attribute<'arena>,
        found: &mut Vec<&'ast Attribute<'arena>>,
    ) {
        found.push(attribute);
    }
}

/// The string literal under `offset` that is an argument of an attribute, with the
/// attribute's class as the file's names resolve it.
pub(crate) fn attribute_string_at(
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<AttributeString> {
    attribute_strings(summary, source)
        .into_iter()
        .find(|(found, end)| found.start <= offset && offset <= *end)
        .map(|(found, _)| found)
}

/// Every string literal that is an argument of an attribute, with the attribute's
/// class as the file's names resolve it, and where the literal's text ends.
pub(crate) fn attribute_strings(
    summary: &FileSummary,
    source: &[u8],
) -> Vec<(AttributeString, u32)> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut attributes: Vec<&Attribute<'_>> = Vec::new();
    Attributes.walk_program(program, &mut attributes);
    let mut found = Vec::new();
    for attribute in attributes {
        let Some(list) = attribute.argument_list.as_ref() else {
            continue;
        };
        let name_start = attribute.name.span().start.offset;
        let Some(attribute_name) = summary.names.iter().find(|n| n.start == name_start) else {
            continue;
        };
        for (position, argument) in list.arguments.iter().enumerate() {
            let (key, value) = match argument {
                PartialArgument::Positional(argument) => (None, argument.value),
                PartialArgument::Named(argument) => (
                    Some(String::from_utf8_lossy(argument.name.value).into_owned()),
                    argument.value,
                ),
                _ => continue,
            };
            let Expression::Literal(Literal::String(literal)) = value else {
                continue;
            };
            let Some(text) = literal.value else {
                continue;
            };
            let span = literal.span();
            found.push((
                AttributeString {
                    attribute: attribute_name.fqn.clone(),
                    key,
                    position,
                    value: String::from_utf8_lossy(text).into_owned(),
                    start: span.start.offset + 1,
                },
                span.end.offset - 1,
            ));
        }
    }
    found
}

/// A method call under the cursor: what it targets, its literal arguments and
/// receiver, and the type of each positional argument.
pub(crate) struct CallAt {
    pub parts: StringArgument,
    pub arguments: Vec<TypeExpr>,
}

/// The method or null-safe call whose name holds `offset`; static calls resolve
/// before this is asked.
pub(crate) fn call_at(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<CallAt> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    calls.iter().rev().find_map(|call| {
        let (selector, arguments) = match call {
            CallRef::Method(c) => (&c.method, &c.argument_list),
            CallRef::NullSafe(c) => (&c.method, &c.argument_list),
            CallRef::Static(c) => (&c.method, &c.argument_list),
        };
        let ClassLikeMemberSelector::Identifier(name) = selector else {
            return None;
        };
        if !(name.span.start.offset <= offset && offset <= name.span.end.offset) {
            return None;
        }
        let parts = ctx.call_parts(call)?;
        let arguments = ctx.argument_types(arguments);
        Some(CallAt { parts, arguments })
    })
}

/// The first positional argument of every call to a method named one of `names`
/// (case-insensitively) in a file, when it is a string literal or `X::class`.
pub(crate) fn first_arguments(source: &[u8], names: &[&str]) -> Vec<String> {
    let summary = summarize(source);
    let classes: HashMap<u32, &str> = summary
        .names
        .iter()
        .filter(|n| n.kind == NameKind::ClassLike)
        .map(|n| (n.start, n.fqn.as_str()))
        .collect();
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    calls
        .iter()
        .filter_map(|call| {
            let (selector, arguments) = match call {
                CallRef::Method(c) => (&c.method, &c.argument_list),
                CallRef::NullSafe(c) => (&c.method, &c.argument_list),
                CallRef::Static(c) => (&c.method, &c.argument_list),
            };
            let ClassLikeMemberSelector::Identifier(name) = selector else {
                return None;
            };
            let method = String::from_utf8_lossy(name.value);
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&method)) {
                return None;
            }
            let Some(Argument::Positional(first)) = arguments.arguments.first() else {
                return None;
            };
            match first.value {
                Expression::Literal(Literal::String(literal)) => literal
                    .value
                    .map(|v| String::from_utf8_lossy(v).into_owned()),
                Expression::Access(Access::ClassConstant(access)) => {
                    let start = access.class.span().start.offset;
                    classes.get(&start).map(|fqn| fqn.to_string())
                }
                _ => None,
            }
        })
        .collect()
}

/// The shapes of the strings a file builds: concatenations and interpolated strings
/// with `*` for each part that is not literal, and literals with `%s` or `%d`
/// placeholders (numbered or not), as for `sprintf()`, with `*` for each placeholder. Only shapes with
/// a `*` are kept.
pub(crate) fn string_shapes(source: &[u8]) -> Vec<String> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut found = Vec::new();
    StringShapes.walk_program(program, &mut found);
    found.retain(|shape| shape.contains('*'));
    found.sort();
    found.dedup();
    found
}

struct StringShapes;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<String>> for StringShapes {
    fn walk_in_binary(&self, binary: &'ast Binary<'arena>, found: &mut Vec<String>) {
        if binary.operator.is_concatenation() {
            found.push(shape(&Expression::Binary(binary.clone())));
        }
    }

    fn walk_in_interpolated_string(
        &self,
        string: &'ast InterpolatedString<'arena>,
        found: &mut Vec<String>,
    ) {
        found.push(shape(&Expression::CompositeString(
            CompositeString::Interpolated(string.clone()),
        )));
    }

    fn walk_in_literal_string(
        &self,
        literal: &'ast LiteralString<'arena>,
        found: &mut Vec<String>,
    ) {
        let Some(value) = literal.value else {
            return;
        };
        if let Some(shape) = placeholder_shape(&String::from_utf8_lossy(value)) {
            found.push(shape);
        }
    }
}

/// `text` with `*` for each `%s` or `%d` placeholder, numbered (`%1$s`) or not;
/// none without a placeholder.
fn placeholder_shape(text: &str) -> Option<String> {
    let mut shape = String::new();
    let mut found = false;
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        shape.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(after) = after.strip_prefix('%') {
            shape.push('%');
            rest = after;
            continue;
        }
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let spec = match after[digits..].strip_prefix('$') {
            Some(spec) if digits > 0 => spec,
            _ => after,
        };
        if let Some(tail) = spec.strip_prefix(['s', 'd']) {
            shape.push('*');
            found = true;
            rest = tail;
        } else {
            shape.push('%');
            rest = after;
        }
    }
    shape.push_str(rest);
    found.then_some(shape)
}

/// The values of every string literal in a file.
pub(crate) fn string_literals(source: &[u8]) -> Vec<String> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut found = Vec::new();
    StringLiterals.walk_program(program, &mut found);
    found
}

struct StringLiterals;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<String>> for StringLiterals {
    fn walk_in_literal_string(
        &self,
        literal: &'ast LiteralString<'arena>,
        found: &mut Vec<String>,
    ) {
        if let Some(value) = literal.value {
            found.push(String::from_utf8_lossy(value).into_owned());
        }
    }
}

/// A string literal that is the value of an array key, and the call whose
/// positional argument is the array holding it, if any.
pub(crate) struct KeyedString {
    pub key: String,
    pub value: String,
    pub call: Option<StringArgument>,
}

/// The innermost `'key' => 'value'` array element whose key and value are string
/// literals and whose value covers `offset`; failing that, an assignment
/// `$a[...]['key'] = 'value'` with a string literal key and value.
pub(crate) fn keyed_string_at(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<KeyedString> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    if let Some(((key, value), mut call)) = calls
        .iter()
        .rev()
        .find_map(|call| ctx.argument_with(call, |argument| keyed_element_at(argument, offset)))
    {
        call.value = value.clone();
        return Some(KeyedString {
            key,
            value,
            call: Some(call),
        });
    }
    let mut elements: Vec<&KeyValueArrayElement<'_>> = Vec::new();
    KeyValues.walk_program(program, &mut elements);
    let mut assignments: Vec<&Assignment<'_>> = Vec::new();
    OffsetAssignments.walk_program(program, &mut assignments);
    elements
        .iter()
        .rev()
        .find_map(|element| keyed_pair(element, offset))
        .or_else(|| {
            assignments
                .iter()
                .find_map(|assignment| offset_pair(assignment, offset))
        })
        .map(|(key, value)| KeyedString {
            key,
            value,
            call: None,
        })
}

/// The key and value of `$a[...]['key'] = 'value'` when the value covers `offset`.
fn offset_pair(assignment: &Assignment<'_>, offset: u32) -> Option<(String, String)> {
    let Expression::ArrayAccess(access) = assignment.lhs else {
        return None;
    };
    let Expression::Literal(Literal::String(key)) = access.index else {
        return None;
    };
    let Expression::Literal(Literal::String(_)) = assignment.rhs else {
        return None;
    };
    let value = string_literal_at(assignment.rhs, offset)?;
    Some((String::from_utf8_lossy(key.value?).into_owned(), value))
}

struct OffsetAssignments;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<&'ast Assignment<'arena>>> for OffsetAssignments {
    fn walk_in_assignment(
        &self,
        assignment: &'ast Assignment<'arena>,
        found: &mut Vec<&'ast Assignment<'arena>>,
    ) {
        if assignment.operator.is_assign() && matches!(assignment.lhs, Expression::ArrayAccess(_)) {
            found.push(assignment);
        }
    }
}

/// A keyed string literal and the range of its value's text.
pub(crate) struct KeyedStringAt {
    pub keyed: KeyedString,
    pub start: u32,
    pub end: u32,
}

/// Every `'key' => 'value'` array element and `$a[...]['key'] = 'value'` assignment
/// of a file whose key and value are string literals, in source order, each with the
/// call whose positional argument is the array holding it, if any.
pub(crate) fn keyed_strings(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
) -> Vec<KeyedStringAt> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    // The call around each element that sits directly in an array argument, by the
    // start of the element's value.
    let mut around: HashMap<u32, StringArgument> = HashMap::new();
    for call in &calls {
        let arguments = match call {
            CallRef::Method(c) => &c.argument_list,
            CallRef::NullSafe(c) => &c.argument_list,
            CallRef::Static(c) => &c.argument_list,
        };
        let mut parts: Option<Option<StringArgument>> = None;
        for (position, argument) in arguments.arguments.iter().enumerate() {
            let Argument::Positional(argument) = argument else {
                continue;
            };
            let elements = match argument.value {
                Expression::Array(array) => &array.elements,
                Expression::LegacyArray(array) => &array.elements,
                _ => continue,
            };
            for element in elements.iter() {
                let ArrayElement::KeyValue(element) = element else {
                    continue;
                };
                let Some(call) = parts.get_or_insert_with(|| ctx.call_parts(call)).clone() else {
                    continue;
                };
                around.insert(
                    element.value.span().start.offset,
                    StringArgument { position, ..call },
                );
            }
        }
    }
    let mut elements: Vec<&KeyValueArrayElement<'_>> = Vec::new();
    KeyValues.walk_program(program, &mut elements);
    let mut assignments: Vec<&Assignment<'_>> = Vec::new();
    OffsetAssignments.walk_program(program, &mut assignments);
    let pairs = elements
        .iter()
        .map(|element| (element.key, element.value))
        .chain(
            assignments
                .iter()
                .filter_map(|assignment| match assignment.lhs {
                    Expression::ArrayAccess(access) => Some((access.index, assignment.rhs)),
                    _ => None,
                }),
        );
    let mut found: Vec<KeyedStringAt> = pairs
        .filter_map(|(key, value)| {
            let (
                Expression::Literal(Literal::String(key)),
                Expression::Literal(Literal::String(literal)),
            ) = (key, value)
            else {
                return None;
            };
            let (start, end) = inside_quotes(literal);
            let call = around
                .get(&value.span().start.offset)
                .map(|argument| StringArgument {
                    value: String::from_utf8_lossy(literal.value.unwrap_or_default()).into_owned(),
                    ..argument.clone()
                });
            Some(KeyedStringAt {
                keyed: KeyedString {
                    key: String::from_utf8_lossy(key.value?).into_owned(),
                    value: String::from_utf8_lossy(literal.value?).into_owned(),
                    call,
                },
                start,
                end,
            })
        })
        .collect();
    found.sort_by_key(|keyed| keyed.start);
    found
}

/// The key and value of an element of the array `expression` whose value covers
/// `offset`, when both are string literals.
fn keyed_element_at(expression: &Expression<'_>, offset: u32) -> Option<(String, String)> {
    let elements = match expression {
        Expression::Array(array) => &array.elements,
        Expression::LegacyArray(array) => &array.elements,
        _ => return None,
    };
    elements.iter().find_map(|element| match element {
        ArrayElement::KeyValue(element) => keyed_pair(element, offset),
        _ => None,
    })
}

fn keyed_pair(element: &KeyValueArrayElement<'_>, offset: u32) -> Option<(String, String)> {
    let Expression::Literal(Literal::String(key)) = element.key else {
        return None;
    };
    let Expression::Literal(Literal::String(_)) = element.value else {
        return None;
    };
    let value = string_literal_at(element.value, offset)?;
    Some((String::from_utf8_lossy(key.value?).into_owned(), value))
}

struct KeyValues;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<&'ast KeyValueArrayElement<'arena>>> for KeyValues {
    fn walk_in_key_value_array_element(
        &self,
        element: &'ast KeyValueArrayElement<'arena>,
        found: &mut Vec<&'ast KeyValueArrayElement<'arena>>,
    ) {
        found.push(element);
    }
}

/// The method calls in a file named one of `names`, case-insensitively, in source
/// order, with what they target and their literal arguments.
pub(crate) fn calls_named(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    names: &[&str],
) -> Vec<FoundCall> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut calls: Vec<CallRef<'_, '_>> = Vec::new();
    CallsWithArguments.walk_program(program, &mut calls);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    let mut found: Vec<FoundCall> = calls
        .iter()
        .filter_map(|call| {
            let (selector, arguments, start) = match call {
                CallRef::Method(c) => (&c.method, &c.argument_list, c.span().start.offset),
                CallRef::NullSafe(c) => (&c.method, &c.argument_list, c.span().start.offset),
                CallRef::Static(c) => (&c.method, &c.argument_list, c.span().start.offset),
            };
            let ClassLikeMemberSelector::Identifier(name) = selector else {
                return None;
            };
            let method = String::from_utf8_lossy(name.value).into_owned();
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&method)) {
                return None;
            }
            let receiver = match call {
                CallRef::Method(c) => ctx.expr(c.object, start, 0),
                CallRef::NullSafe(c) => ctx.expr(c.object, start, 0),
                CallRef::Static(c) => ctx
                    .class_of(c.class, start, 0)
                    .map_or(TypeExpr::Other, TypeExpr::Named),
            };
            let caller =
                calls::caller_at(path, summary, name.span.start.offset, source.len() as u32);
            Some(FoundCall {
                location: Location {
                    path: path.to_path_buf(),
                    start: name.span.start.offset,
                    end: name.span.end.offset,
                },
                targets: ctx.call_targets(&receiver, &method),
                strings: ctx.string_arguments(arguments, start),
                lists: list_arguments(arguments),
                shapes: argument_shapes(arguments),
                ranges: argument_ranges(arguments),
                receiver: receiver.clone(),
                arguments: ctx.argument_types(arguments),
                caller: match caller.kind {
                    CallKind::Method => Some(format!("{}::{}", caller.detail, caller.name)),
                    CallKind::Function if caller.detail.is_empty() => Some(caller.name),
                    CallKind::Function => Some(format!("{}\\{}", caller.detail, caller.name)),
                    CallKind::File => None,
                },
            })
        })
        .collect();
    found.sort_by_key(|call| call.location.start);
    found
}

/// The value of the string literal covering `offset`: `expression` itself, or one
/// of its elements when it is an array.
fn string_literal_at(expression: &Expression<'_>, offset: u32) -> Option<String> {
    let elements = match expression {
        Expression::Literal(Literal::String(literal)) => {
            let span = literal.span();
            let inside = span.start.offset < offset && offset < span.end.offset;
            return inside
                .then(|| {
                    literal
                        .value
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                })
                .flatten();
        }
        Expression::Array(array) => &array.elements,
        Expression::LegacyArray(array) => &array.elements,
        _ => return None,
    };
    elements.iter().find_map(|element| match element {
        ArrayElement::Value(element) => match element.value {
            Expression::Literal(Literal::String(_)) => string_literal_at(element.value, offset),
            _ => None,
        },
        _ => None,
    })
}

/// The byte range of each positional argument that is a string literal (inside its
/// quotes) or a class constant fetch, by position.
fn argument_ranges(arguments: &ArgumentList<'_>) -> Vec<Option<(u32, u32)>> {
    arguments
        .arguments
        .iter()
        .map(|argument| match argument {
            Argument::Positional(argument) => match argument.value {
                Expression::Literal(Literal::String(literal)) => Some(inside_quotes(literal)),
                Expression::Access(Access::ClassConstant(access)) => {
                    let span = access.span();
                    Some((span.start.offset, span.end.offset))
                }
                _ => None,
            },
            Argument::Named(_) => None,
        })
        .collect()
}

/// The range of a string literal's text, quotes left out.
fn inside_quotes(literal: &LiteralString<'_>) -> (u32, u32) {
    let span = literal.span();
    (
        span.start.offset + 1,
        span.end.offset.saturating_sub(1).max(span.start.offset + 1),
    )
}

/// The text shape of each positional argument, by position.
fn argument_shapes(arguments: &ArgumentList<'_>) -> Vec<Vec<String>> {
    arguments
        .arguments
        .iter()
        .map(|argument| match argument {
            Argument::Positional(argument) => match argument.value {
                Expression::Array(array) => element_shapes(array.elements.iter()),
                Expression::LegacyArray(array) => element_shapes(array.elements.iter()),
                value => vec![shape(value)],
            },
            Argument::Named(_) => Vec::new(),
        })
        .collect()
}

fn element_shapes<'a>(elements: impl Iterator<Item = &'a ArrayElement<'a>>) -> Vec<String> {
    elements
        .filter_map(|element| match element {
            ArrayElement::Value(element) => Some(shape(element.value)),
            _ => None,
        })
        .collect()
}

/// A string expression's text with `*` for each part that is not literal: a
/// literal, a concatenation or an interpolated string; `*` for anything else.
fn shape(expression: &Expression<'_>) -> String {
    let text = match expression {
        Expression::Literal(Literal::String(literal)) => literal
            .value
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_else(|| "*".to_string()),
        Expression::Parenthesized(inner) => shape(inner.expression),
        Expression::Binary(binary) if binary.operator.is_concatenation() => {
            shape(binary.lhs) + &shape(binary.rhs)
        }
        Expression::CompositeString(CompositeString::Interpolated(string)) => string
            .parts
            .iter()
            .map(|part| match part {
                StringPart::Literal(literal) => literal
                    .value
                    .map(|v| String::from_utf8_lossy(v).into_owned())
                    .unwrap_or_default(),
                _ => "*".to_string(),
            })
            .collect(),
        _ => "*".to_string(),
    };
    let mut collapsed = String::with_capacity(text.len());
    for c in text.chars() {
        if !(c == '*' && collapsed.ends_with('*')) {
            collapsed.push(c);
        }
    }
    collapsed
}

/// The string literal elements of each positional array argument, by position.
fn list_arguments(arguments: &ArgumentList<'_>) -> Vec<Vec<String>> {
    arguments
        .arguments
        .iter()
        .map(|argument| {
            let Argument::Positional(argument) = argument else {
                return Vec::new();
            };
            let elements = match argument.value {
                Expression::Array(array) => &array.elements,
                Expression::LegacyArray(array) => &array.elements,
                _ => return Vec::new(),
            };
            elements
                .iter()
                .filter_map(|element| match element {
                    ArrayElement::Value(element) => match element.value {
                        Expression::Literal(Literal::String(literal)) => literal
                            .value
                            .map(|v| String::from_utf8_lossy(v).into_owned()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// A call that can carry framework string arguments.
enum CallRef<'ast, 'arena> {
    Method(&'ast MethodCall<'arena>),
    NullSafe(&'ast NullSafeMethodCall<'arena>),
    Static(&'ast StaticMethodCall<'arena>),
}

struct CallsWithArguments;

impl<'ast, 'arena> Walker<'ast, 'arena, Vec<CallRef<'ast, 'arena>>> for CallsWithArguments {
    fn walk_in_method_call(
        &self,
        call: &'ast MethodCall<'arena>,
        calls: &mut Vec<CallRef<'ast, 'arena>>,
    ) {
        calls.push(CallRef::Method(call));
    }

    fn walk_in_null_safe_method_call(
        &self,
        call: &'ast NullSafeMethodCall<'arena>,
        calls: &mut Vec<CallRef<'ast, 'arena>>,
    ) {
        calls.push(CallRef::NullSafe(call));
    }

    fn walk_in_static_method_call(
        &self,
        call: &'ast StaticMethodCall<'arena>,
        calls: &mut Vec<CallRef<'ast, 'arena>>,
    ) {
        calls.push(CallRef::Static(call));
    }
}

/// The variable whose name covers `offset` and its inferred type.
pub(crate) fn variable_at(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    offset: u32,
) -> Option<(String, TypeExpr)> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let (name, start) = facts
        .variables
        .iter()
        .find(|(_, start, end)| *start <= offset && offset <= *end)
        .map(|(name, start, _)| (name.clone(), *start))?;
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    let ty = ctx.var(&name, start, 0);
    Some((name, ty))
}

/// Every instance member access named `name` in the file, resolved with one parse:
/// start and end of the member name, receiver, and kind.
pub(crate) fn member_accesses_named(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    name: &str,
) -> Vec<(u32, u32, Receiver, MemberKind)> {
    accesses(index, path, summary, source, Some(name))
        .into_iter()
        .map(|(start, end, _, receiver, kind)| (start, end, receiver, kind))
        .collect()
}

/// Every instance member access in the file, resolved with one parse: start and
/// end of the member name, the name, receiver, and kind.
pub(crate) fn member_accesses(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
) -> Vec<(u32, u32, String, Receiver, MemberKind)> {
    accesses(index, path, summary, source, None)
}

fn accesses(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    name: Option<&str>,
) -> Vec<(u32, u32, String, Receiver, MemberKind)> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let ctx = Ctx::new(index, path, summary, program, source, &facts);
    facts
        .accesses
        .iter()
        .filter(|a| name.is_none_or(|name| crate::references::same_name(&a.name, name, a.kind)))
        .map(|access| {
            let receiver = match ctx.anonymous_this(access.object, access.start) {
                Some(range) => Receiver::Anonymous(range),
                None => Receiver::Classes(class_names(&ctx.expr(access.object, access.start, 0))),
            };
            (
                access.start,
                access.end,
                access.name.clone(),
                receiver,
                access.kind,
            )
        })
        .collect()
}

/// Every instance member access in the file: start and end of the name, and kind.
pub(crate) fn member_access_spans(source: &[u8]) -> Vec<(u32, u32, MemberKind)> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    facts
        .accesses
        .iter()
        .map(|a| (a.start, a.end, a.kind))
        .collect()
}

/// The start offsets of every instance member name accessed in the file.
pub(crate) fn member_access_starts(source: &[u8]) -> Vec<u32> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    facts.accesses.iter().map(|a| a.start).collect()
}

struct AccessFact<'arena> {
    start: u32,
    end: u32,
    kind: MemberKind,
    name: String,
    object: &'arena Expression<'arena>,
}

enum ScopeKind {
    Function,
    /// A closure sees the variables of its `use` clause from the enclosing scope.
    Closure(Vec<String>),
    /// An arrow function sees every variable of the enclosing scope.
    Arrow,
}

struct ScopeFact<'ast, 'arena> {
    start: u32,
    end: u32,
    kind: ScopeKind,
    params: &'ast FunctionLikeParameterList<'arena>,
}

/// `$var = rhs`, or with no `rhs` a rebinding to an unknown value: destructuring,
/// `global`, `static`.
struct AssignFact<'arena> {
    start: u32,
    end: u32,
    var: String,
    rhs: Option<&'arena Expression<'arena>>,
    /// For a destructured variable, the keys from `rhs` to its value; `None` for a
    /// key that is not a literal.
    path: Vec<Option<String>>,
}

/// `$object->name = rhs` or `$object->name ??= rhs`.
struct PropertyAssignFact<'arena> {
    start: u32,
    name: String,
    object: &'arena Expression<'arena>,
    rhs: &'arena Expression<'arena>,
}

/// A `foreach`: its value variable takes the iterable's element type; its key and
/// destructured variables become unknown.
struct ForeachFact<'arena> {
    start: u32,
    body_start: u32,
    value: Option<String>,
    /// Variables a destructured value assigns, with the keys from the value to each.
    entries: Vec<(String, Vec<Option<String>>)>,
    unknown: Vec<String>,
    iterable: &'arena Expression<'arena>,
}

struct CatchFact<'ast, 'arena> {
    start: u32,
    var: String,
    hint: &'ast Hint<'arena>,
}

/// `$var instanceof Class` holds between `start` and `end`; `class` is the offset of
/// the class name, which the name resolver resolved.
struct NarrowFact {
    start: u32,
    end: u32,
    var: String,
    class: u32,
}

/// One branch of a control-flow statement, and where that statement ends. Loops,
/// `switch` and anonymous class bodies count as a single branch spanning the whole
/// statement.
struct BranchFact {
    start: u32,
    end: u32,
    statement_end: u32,
}

/// A closure or arrow function written as a positional argument of a function call.
struct CallbackFact<'ast, 'arena> {
    /// Where the closure starts, as its scope does.
    start: u32,
    call: &'ast FunctionCall<'arena>,
    position: usize,
}

/// A closure or arrow function given positionally to a method call.
struct MethodCallbackFact<'ast, 'arena> {
    /// Where the closure starts, as its scope does.
    start: u32,
    object: &'arena Expression<'arena>,
    method: &'ast ClassLikeMemberSelector<'arena>,
    position: usize,
}

#[derive(Default)]
struct Facts<'ast, 'arena> {
    accesses: Vec<AccessFact<'arena>>,
    scopes: Vec<ScopeFact<'ast, 'arena>>,
    assignments: Vec<AssignFact<'arena>>,
    properties: Vec<PropertyAssignFact<'arena>>,
    foreaches: Vec<ForeachFact<'arena>>,
    catches: Vec<CatchFact<'ast, 'arena>>,
    narrowings: Vec<NarrowFact>,
    branches: Vec<BranchFact>,
    callbacks: Vec<CallbackFact<'ast, 'arena>>,
    method_callbacks: Vec<MethodCallbackFact<'ast, 'arena>>,
    /// Static calls and constants reached through a variable, such as
    /// `$severity::from()`, with the variable as the object; navigation reads them.
    variable_statics: Vec<AccessFact<'arena>>,
    /// Every direct variable: name without `$`, start, end.
    variables: Vec<(String, u32, u32)>,
}

impl<'ast, 'arena> Facts<'ast, 'arena> {
    fn access(
        &mut self,
        object: &'arena Expression<'arena>,
        selector: &ClassLikeMemberSelector<'_>,
        kind: MemberKind,
    ) {
        if let ClassLikeMemberSelector::Identifier(name) = selector {
            self.accesses.push(AccessFact {
                start: name.span.start.offset,
                end: name.span.end.offset,
                kind,
                name: String::from_utf8_lossy(name.value).into_owned(),
                object,
            });
        }
    }

    /// Records each closure or arrow function given positionally to a method call.
    fn method_callbacks(
        &mut self,
        object: &'arena Expression<'arena>,
        method: &'ast ClassLikeMemberSelector<'arena>,
        arguments: &'ast ArgumentList<'arena>,
    ) {
        for (position, argument) in arguments.arguments.iter().enumerate() {
            let Argument::Positional(argument) = argument else {
                continue;
            };
            let start = match argument.value {
                Expression::Closure(closure) => closure.span().start.offset,
                Expression::ArrowFunction(arrow) => arrow.span().start.offset,
                _ => continue,
            };
            self.method_callbacks.push(MethodCallbackFact {
                start,
                object,
                method,
                position,
            });
        }
    }

    fn scope(
        &mut self,
        node: &impl HasSpan,
        kind: ScopeKind,
        params: &'ast FunctionLikeParameterList<'arena>,
    ) {
        let span = node.span();
        self.scopes.push(ScopeFact {
            start: span.start.offset,
            end: span.end.offset,
            kind,
            params,
        });
    }

    fn branch(&mut self, node: &impl HasSpan, statement_end: u32) {
        let span = node.span();
        self.branches.push(BranchFact {
            start: span.start.offset,
            end: span.end.offset,
            statement_end,
        });
    }

    /// A whole statement as its own single branch.
    fn whole(&mut self, node: &impl HasSpan) {
        let end = node.span().end.offset;
        self.branch(node, end);
    }

    fn unknown(&mut self, start: u32, end: u32, var: String) {
        self.assignments.push(AssignFact {
            start,
            end,
            var,
            rhs: None,
            path: Vec::new(),
        });
    }
}

struct Collect;

impl<'ast, 'arena> Walker<'ast, 'arena, Facts<'ast, 'arena>> for Collect {
    fn walk_in_function_call(&self, call: &'ast FunctionCall<'arena>, f: &mut Facts<'ast, 'arena>) {
        for (position, argument) in call.argument_list.arguments.iter().enumerate() {
            let Argument::Positional(argument) = argument else {
                continue;
            };
            let start = match argument.value {
                Expression::Closure(closure) => closure.span().start.offset,
                Expression::ArrowFunction(arrow) => arrow.span().start.offset,
                _ => continue,
            };
            f.callbacks.push(CallbackFact {
                start,
                call,
                position,
            });
        }
    }

    fn walk_in_method_call(&self, call: &'ast MethodCall<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.access(call.object, &call.method, MemberKind::Method);
        f.method_callbacks(call.object, &call.method, &call.argument_list);
    }

    fn walk_in_null_safe_method_call(
        &self,
        call: &'ast NullSafeMethodCall<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        f.access(call.object, &call.method, MemberKind::Method);
        f.method_callbacks(call.object, &call.method, &call.argument_list);
    }

    fn walk_in_property_access(
        &self,
        access: &'ast PropertyAccess<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        f.access(access.object, &access.property, MemberKind::Property);
    }

    fn walk_in_null_safe_property_access(
        &self,
        access: &'ast NullSafePropertyAccess<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        f.access(access.object, &access.property, MemberKind::Property);
    }

    fn walk_in_static_method_call(
        &self,
        call: &'ast StaticMethodCall<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        if let (Expression::Variable(_), ClassLikeMemberSelector::Identifier(name)) =
            (call.class, &call.method)
        {
            f.variable_statics.push(AccessFact {
                start: name.span.start.offset,
                end: name.span.end.offset,
                kind: MemberKind::Method,
                name: String::from_utf8_lossy(name.value).into_owned(),
                object: call.class,
            });
        }
    }

    fn walk_in_class_constant_access(
        &self,
        access: &'ast ClassConstantAccess<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        if let (Expression::Variable(_), ClassLikeConstantSelector::Identifier(name)) =
            (access.class, &access.constant)
        {
            f.variable_statics.push(AccessFact {
                start: name.span.start.offset,
                end: name.span.end.offset,
                kind: MemberKind::Constant,
                name: String::from_utf8_lossy(name.value).into_owned(),
                object: access.class,
            });
        }
    }

    fn walk_in_method(&self, method: &'ast Method<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.scope(method, ScopeKind::Function, &method.parameter_list);
    }

    fn walk_in_function(&self, function: &'ast Function<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.scope(function, ScopeKind::Function, &function.parameter_list);
    }

    fn walk_in_closure(&self, closure: &'ast Closure<'arena>, f: &mut Facts<'ast, 'arena>) {
        let uses = closure
            .use_clause
            .as_ref()
            .map(|clause| {
                clause
                    .variables
                    .iter()
                    .map(|v| variable_name(v.variable.name))
                    .collect()
            })
            .unwrap_or_default();
        f.scope(closure, ScopeKind::Closure(uses), &closure.parameter_list);
    }

    fn walk_in_arrow_function(
        &self,
        arrow: &'ast ArrowFunction<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        f.scope(arrow, ScopeKind::Arrow, &arrow.parameter_list);
    }

    fn walk_in_anonymous_class(
        &self,
        class: &'ast AnonymousClass<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        f.whole(class);
    }

    fn walk_in_assignment(
        &self,
        assignment: &'ast Assignment<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        let span = assignment.span();
        if let Expression::Access(Access::Property(access)) = assignment.lhs
            && let ClassLikeMemberSelector::Identifier(name) = &access.property
            && matches!(
                assignment.operator,
                AssignmentOperator::Assign(_) | AssignmentOperator::Coalesce(_)
            )
        {
            f.properties.push(PropertyAssignFact {
                start: span.start.offset,
                name: String::from_utf8_lossy(name.value).into_owned(),
                object: access.object,
                rhs: assignment.rhs,
            });
            return;
        }
        if !assignment.operator.is_assign() {
            return;
        }
        let (start, end) = (span.start.offset, span.end.offset);
        match assignment.lhs {
            Expression::Variable(Variable::Direct(variable)) => {
                f.assignments.push(AssignFact {
                    start,
                    end,
                    var: variable_name(variable.name),
                    rhs: Some(assignment.rhs),
                    path: Vec::new(),
                });
            }
            destructuring => {
                for (var, path) in destructured_paths(destructuring) {
                    f.assignments.push(AssignFact {
                        start,
                        end,
                        var,
                        rhs: Some(assignment.rhs),
                        path,
                    });
                }
            }
        }
    }

    fn walk_in_global(&self, global: &'ast Global<'arena>, f: &mut Facts<'ast, 'arena>) {
        let span = global.span();
        for variable in global.variables.iter() {
            if let Variable::Direct(direct) = variable {
                f.unknown(
                    span.start.offset,
                    span.end.offset,
                    variable_name(direct.name),
                );
            }
        }
    }

    fn walk_in_static(&self, r#static: &'ast Static<'arena>, f: &mut Facts<'ast, 'arena>) {
        let span = r#static.span();
        for item in r#static.items.iter() {
            let variable = match item {
                StaticItem::Abstract(item) => &item.variable,
                StaticItem::Concrete(item) => &item.variable,
            };
            f.unknown(
                span.start.offset,
                span.end.offset,
                variable_name(variable.name),
            );
        }
    }

    fn walk_in_foreach(&self, foreach: &'ast Foreach<'arena>, f: &mut Facts<'ast, 'arena>) {
        let (key, value) = match &foreach.target {
            ForeachTarget::Value(target) => (None, target.value),
            ForeachTarget::KeyValue(target) => (Some(target.key), target.value),
        };
        let unknown: Vec<String> = key.into_iter().flat_map(destructured).collect();
        let (value, entries) = match value {
            Expression::Variable(Variable::Direct(variable)) => {
                (Some(variable_name(variable.name)), Vec::new())
            }
            destructuring => (None, destructured_paths(destructuring)),
        };
        f.foreaches.push(ForeachFact {
            start: foreach.span().start.offset,
            body_start: foreach.right_parenthesis.end.offset,
            value,
            entries,
            unknown,
            iterable: foreach.expression,
        });
        f.whole(foreach);
    }

    fn walk_in_for(&self, r#for: &'ast For<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.whole(r#for);
    }

    fn walk_in_while(&self, r#while: &'ast While<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.whole(r#while);
    }

    fn walk_in_do_while(&self, do_while: &'ast DoWhile<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.whole(do_while);
    }

    fn walk_in_switch(&self, switch: &'ast Switch<'arena>, f: &mut Facts<'ast, 'arena>) {
        f.whole(switch);
    }

    fn walk_in_try(&self, r#try: &'ast Try<'arena>, f: &mut Facts<'ast, 'arena>) {
        let end = r#try.span().end.offset;
        f.branch(&r#try.block, end);
        for clause in r#try.catch_clauses.iter() {
            f.branch(&clause.block, end);
        }
        if let Some(finally) = &r#try.finally_clause {
            f.branch(&finally.block, end);
        }
    }

    fn walk_in_try_catch_clause(
        &self,
        clause: &'ast TryCatchClause<'arena>,
        f: &mut Facts<'ast, 'arena>,
    ) {
        if let Some(variable) = &clause.variable {
            f.catches.push(CatchFact {
                start: clause.block.span().start.offset,
                var: variable_name(variable.name),
                hint: &clause.hint,
            });
        }
    }

    fn walk_in_if(&self, r#if: &'ast If<'arena>, f: &mut Facts<'ast, 'arena>) {
        let end = r#if.span().end.offset;
        let then = match &r#if.body {
            IfBody::Statement(body) => {
                f.branch(body.statement, end);
                for clause in body.else_if_clauses.iter() {
                    f.branch(clause.statement, end);
                }
                if let Some(clause) = &body.else_clause {
                    f.branch(clause.statement, end);
                }
                let span = body.statement.span();
                (span.start.offset, span.end.offset)
            }
            IfBody::ColonDelimited(body) => {
                f.whole(r#if);
                let start = body.colon.end.offset;
                let end = body
                    .statements
                    .iter()
                    .last()
                    .map_or(start, |s| s.span().end.offset);
                (start, end)
            }
        };
        let mut checks = Vec::new();
        instanceof_checks(r#if.condition, &mut checks);
        for (var, class) in checks {
            f.narrowings.push(NarrowFact {
                start: then.0,
                end: then.1,
                var,
                class,
            });
        }
    }

    /// In `match (true)`, an arm with one condition narrows its expression as an
    /// `if` with that condition narrows its body.
    fn walk_in_match(&self, r#match: &'ast Match<'arena>, f: &mut Facts<'ast, 'arena>) {
        if !matches!(r#match.expression, Expression::Literal(Literal::True(_))) {
            return;
        }
        for arm in r#match.arms.iter() {
            let MatchArm::Expression(arm) = arm else {
                continue;
            };
            let [condition] = arm.conditions.as_slice() else {
                continue;
            };
            let mut checks = Vec::new();
            instanceof_checks(condition, &mut checks);
            // From the arrow, so the narrowing comes before the arm's first use.
            let start = arm.arrow.start.offset;
            let end = arm.expression.span().end.offset;
            for (var, class) in checks {
                f.narrowings.push(NarrowFact {
                    start,
                    end,
                    var,
                    class,
                });
            }
        }
    }

    fn walk_in_variable(&self, variable: &'ast Variable<'arena>, f: &mut Facts<'ast, 'arena>) {
        if let Variable::Direct(direct) = variable {
            f.variables.push((
                variable_name(direct.name),
                direct.span.start.offset,
                direct.span.end.offset,
            ));
        }
    }
}

/// Each direct variable a destructuring target assigns, with the keys that lead
/// from the destructured value to it: `[$a, $b]` gives `$a` key `0` and `$b` key
/// `1`, `['k' => $c]` gives `$c` key `k`; a key that is not a literal is `None`.
fn destructured_paths(target: &Expression<'_>) -> Vec<(String, Vec<Option<String>>)> {
    let elements = match target {
        Expression::Variable(Variable::Direct(variable)) => {
            return vec![(variable_name(variable.name), Vec::new())];
        }
        Expression::Array(array) => &array.elements,
        Expression::List(list) => &list.elements,
        Expression::LegacyArray(array) => &array.elements,
        _ => return Vec::new(),
    };
    let mut found = Vec::new();
    let mut position = 0;
    for element in elements.iter() {
        let (key, value) = match element {
            ArrayElement::KeyValue(element) => (literal_key(element.key), element.value),
            ArrayElement::Value(element) => {
                position += 1;
                (Some((position - 1).to_string()), element.value)
            }
            ArrayElement::Missing(_) => {
                position += 1;
                continue;
            }
            _ => continue,
        };
        for (var, mut path) in destructured_paths(value) {
            path.insert(0, key.clone());
            found.push((var, path));
        }
    }
    found
}

/// A string or integer literal used as an array key, as text.
fn literal_key(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Literal(Literal::String(literal)) => literal
            .value
            .map(|v| String::from_utf8_lossy(v).into_owned()),
        Expression::Literal(Literal::Integer(literal)) => literal.value.map(|v| v.to_string()),
        _ => None,
    }
}

/// The direct variables a destructuring target (`[$a, $b]`, `list($a)`, nested)
/// assigns, or a lone variable.
fn destructured(target: &Expression<'_>) -> Vec<String> {
    let elements = match target {
        Expression::Variable(Variable::Direct(variable)) => {
            return vec![variable_name(variable.name)];
        }
        Expression::Array(array) => &array.elements,
        Expression::List(list) => &list.elements,
        Expression::LegacyArray(array) => &array.elements,
        _ => return Vec::new(),
    };
    elements
        .iter()
        .flat_map(|element| match element {
            ArrayElement::KeyValue(element) => destructured(element.value),
            ArrayElement::Value(element) => destructured(element.value),
            _ => Vec::new(),
        })
        .collect()
}

/// `$v instanceof C` checks that hold when `condition` is true: alone, inside
/// parentheses, or in an `&&` chain.
fn instanceof_checks(condition: &Expression<'_>, out: &mut Vec<(String, u32)>) {
    match condition {
        Expression::Parenthesized(inner) => instanceof_checks(inner.expression, out),
        Expression::Binary(binary) => match binary.operator {
            BinaryOperator::Instanceof(_) => {
                if let (
                    Expression::Variable(Variable::Direct(variable)),
                    Expression::Identifier(class),
                ) = (binary.lhs, binary.rhs)
                {
                    out.push((variable_name(variable.name), class.span().start.offset));
                }
            }
            BinaryOperator::And(_) | BinaryOperator::LowAnd(_) => {
                instanceof_checks(binary.lhs, out);
                instanceof_checks(binary.rhs, out);
            }
            _ => {}
        },
        _ => {}
    }
}

fn variable_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name)
        .trim_start_matches('$')
        .to_string()
}

/// How a binding affects a use of the variable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The binding is certainly in effect.
    Replace,
    /// The binding may or may not have happened: merge it with what came before.
    Merge,
    /// The binding cannot reach the use.
    Ignore,
}

/// A source of a variable's type.
enum Binding<'f, 'ast, 'arena> {
    Param(&'f ScopeFact<'ast, 'arena>, usize),
    Assign(&'f AssignFact<'arena>),
    ForeachValue(&'f ForeachFact<'arena>),
    /// A variable a destructured `foreach` value assigns, with the keys to it.
    ForeachEntry(&'f ForeachFact<'arena>, &'f [Option<String>]),
    Unknown,
    Catch(&'f CatchFact<'ast, 'arena>),
    Narrow(u32),
    Doc(TypeExpr),
}

/// A variable's type while folding its bindings in order.
enum State {
    Unset,
    Null,
    Known(TypeExpr),
    Unknown,
}

impl State {
    fn of(ty: TypeExpr) -> State {
        match ty {
            TypeExpr::Other | TypeExpr::Scalar(_) => State::Unknown,
            ty => State::Known(ty),
        }
    }

    fn merge(self, other: State) -> State {
        match (self, other) {
            (State::Unknown, _) | (_, State::Unknown) => State::Unknown,
            (State::Unset | State::Null, other) => other,
            (known, State::Unset | State::Null) => known,
            (State::Known(a), State::Known(b)) => State::Known(union(vec![a, b])),
        }
    }

    fn into_type(self) -> TypeExpr {
        match self {
            State::Known(ty) => ty,
            _ => TypeExpr::Other,
        }
    }
}

struct Ctx<'a, 'ast, 'arena> {
    index: &'a Index,
    summary: &'a FileSummary,
    program: &'arena Program<'arena>,
    source: &'a [u8],
    facts: &'a Facts<'ast, 'arena>,
    names: HashMap<u32, &'a NameRef>,
    /// Inline `@var Type $name` docblocks: position, variable, type.
    var_docs: Vec<(u32, String, TypeExpr)>,
    /// Variable types already inferred, by name and position.
    memo: RefCell<HashMap<(String, u32), TypeExpr>>,
    /// The property assignment computation this context serves.
    assigning: Option<&'a Assigning<'a>>,
    /// The framework modules whose project holds the file being read: only they
    /// answer for it.
    frameworks: Vec<&'a dyn Framework>,
}

impl<'a, 'ast, 'arena> Ctx<'a, 'ast, 'arena> {
    fn new(
        index: &'a Index,
        path: &'a Path,
        summary: &'a FileSummary,
        program: &'arena Program<'arena>,
        source: &'a [u8],
        facts: &'a Facts<'ast, 'arena>,
    ) -> Self {
        let names = summary.names.iter().map(|n| (n.start, n)).collect();
        let var_docs = program
            .trivia
            .iter()
            .filter(|t| t.kind == TriviaKind::DocBlockComment)
            .flat_map(|t| {
                let start = t.span.start.offset;
                let doc = parse_docblock(
                    &String::from_utf8_lossy(t.value),
                    &summary.file_names.at(start),
                );
                doc.var
                    .into_iter()
                    .filter_map(move |(var, ty)| Some((start, var?, ty)))
            })
            .collect();
        Ctx {
            index,
            summary,
            program,
            source,
            facts,
            names,
            var_docs,
            memo: RefCell::new(HashMap::new()),
            assigning: None,
            frameworks: index.frameworks_for(path).collect(),
        }
    }

    fn text(&self, start: u32, end: u32) -> String {
        String::from_utf8_lossy(
            self.source
                .get(start as usize..end as usize)
                .unwrap_or_default(),
        )
        .into_owned()
    }

    fn hint_type(&self, hint: &Hint<'_>) -> TypeExpr {
        let span = hint.span();
        parse_type(
            &self.text(span.start.offset, span.end.offset),
            &self.summary.file_names.at(span.start.offset),
        )
    }

    /// The named class enclosing `at`; nothing inside an anonymous class.
    fn enclosing(&self, at: u32) -> Option<String> {
        self.summary.enclosing_class(at).and_then(|c| c.fqn.clone())
    }

    /// `parent`, the parent of the class around `at`, written with the arguments the
    /// class's `@extends` gives it.
    fn parent_type(&self, parent: String, at: u32) -> TypeExpr {
        // The class's own templates are unknown inside it.
        let args = self.enclosing(at).and_then(|class| {
            let shape = self
                .summary
                .declarations
                .iter()
                .find(|d| d.fqn == class)?
                .class
                .as_ref()?;
            shape
                .generic_supertypes
                .iter()
                .find(|(supertype, _)| supertype.eq_ignore_ascii_case(&parent))
                .map(|(_, args)| {
                    args.iter()
                        .map(|arg| substitute(arg, &shape.templates, &[]))
                        .collect::<Vec<_>>()
                })
        });
        match args {
            Some(args) if !args.is_empty() => TypeExpr::Generic {
                class: parent,
                args,
            },
            _ => TypeExpr::Named(parent),
        }
    }

    fn enclosing_parent(&self, at: u32) -> Option<String> {
        self.summary.enclosing_class(at)?.parent.clone()
    }

    /// The index of the anonymous class enclosing `at` when `object` is `$this`.
    fn anonymous_this(&self, object: &Expression<'_>, at: u32) -> Option<usize> {
        let Expression::Variable(Variable::Direct(variable)) = object else {
            return None;
        };
        if variable.name != b"$this" {
            return None;
        }
        let (i, range) = self
            .summary
            .class_ranges
            .iter()
            .enumerate()
            .filter(|(_, r)| r.start <= at && at <= r.end)
            .max_by_key(|(_, r)| r.start)?;
        range.fqn.is_none().then_some(i)
    }

    /// The class an expression before `::` or after `new` names.
    /// The class `expression` names before `::`: a class name, `self`, `static` or
    /// `parent`, or a variable holding an object of one class.
    fn class_of(&self, expression: &Expression<'_>, at: u32, depth: u32) -> Option<String> {
        match expression {
            Expression::Identifier(identifier) => self
                .names
                .get(&identifier.span().start.offset)
                .filter(|n| n.kind == NameKind::ClassLike)
                .map(|n| n.fqn.clone()),
            Expression::Self_(_) | Expression::Static(_) => self.enclosing(at),
            Expression::Parent(_) => self.enclosing_parent(at),
            Expression::Variable(Variable::Direct(variable)) => match self.var(
                &variable_name(variable.name),
                variable.span.start.offset,
                depth + 1,
            ) {
                TypeExpr::Named(class)
                | TypeExpr::Tagged { class, .. }
                | TypeExpr::Generic { class, .. } => Some(class),
                _ => None,
            },
            _ => None,
        }
    }

    fn expr(&self, expression: &Expression<'_>, at: u32, depth: u32) -> TypeExpr {
        if depth > MAX_DEPTH {
            return TypeExpr::Other;
        }
        let depth = depth + 1;
        match expression {
            Expression::Variable(Variable::Direct(variable)) => self.var(
                &variable_name(variable.name),
                variable.span.start.offset,
                depth,
            ),
            Expression::Parenthesized(inner) => self.expr(inner.expression, at, depth),
            Expression::Clone(clone) => self.expr(clone.object, at, depth),
            Expression::Assignment(assignment) => self.expr(assignment.rhs, at, depth),
            Expression::Instantiation(instantiation) => self
                .class_of(instantiation.class, at, depth)
                .map_or(TypeExpr::Other, TypeExpr::Named),
            Expression::Binary(binary) if binary.operator.is_null_coalesce() => union(vec![
                self.expr(binary.lhs, at, depth),
                self.expr(binary.rhs, at, depth),
            ]),
            Expression::Conditional(conditional) => union(vec![
                self.expr(conditional.then.unwrap_or(conditional.condition), at, depth),
                self.expr(conditional.r#else, at, depth),
            ]),
            Expression::Call(Call::Method(call)) => {
                let receiver = self.expr(call.object, at, depth);
                self.method_call(
                    &receiver,
                    &call.method,
                    &call.argument_list,
                    at,
                    depth,
                    None,
                )
            }
            Expression::Call(Call::NullSafeMethod(call)) => {
                let receiver = self.expr(call.object, at, depth);
                self.method_call(
                    &receiver,
                    &call.method,
                    &call.argument_list,
                    at,
                    depth,
                    None,
                )
            }
            Expression::Call(Call::StaticMethod(call)) => {
                let Some(class) = self.class_of(call.class, at, depth) else {
                    return TypeExpr::Other;
                };
                // `self::`, `static::` and `parent::` forward late static binding to
                // the calling class.
                let forwarded = match call.class {
                    Expression::Self_(_) | Expression::Static(_) | Expression::Parent(_) => {
                        self.enclosing(at)
                    }
                    _ => None,
                };
                let receiver = match call.class {
                    Expression::Parent(_) => self.parent_type(class, at),
                    _ => TypeExpr::Named(class),
                };
                self.method_call(
                    &receiver,
                    &call.method,
                    &call.argument_list,
                    at,
                    depth,
                    forwarded.as_deref(),
                )
            }
            Expression::Call(Call::Function(call)) => self.function_call(call, at, depth),
            Expression::Access(Access::Property(access)) => self.member_on(
                &self.expr(access.object, at, depth),
                &access.property,
                MemberKind::Property,
                None,
            ),
            Expression::Access(Access::NullSafeProperty(access)) => self.member_on(
                &self.expr(access.object, at, depth),
                &access.property,
                MemberKind::Property,
                None,
            ),
            Expression::Access(Access::StaticProperty(access)) => {
                match (self.class_of(access.class, at, depth), &access.property) {
                    (Some(class), Variable::Direct(variable)) => self.member_type(
                        &TypeExpr::Named(class),
                        MemberKind::Property,
                        &variable_name(variable.name),
                        None,
                        None,
                    ),
                    _ => TypeExpr::Other,
                }
            }
            Expression::ArrayAccess(access) => {
                let key = literal_key(access.index);
                entry_type(
                    self.index,
                    &self.expr(access.array, at, depth),
                    key.as_deref(),
                )
            }
            Expression::Access(Access::ClassConstant(access)) => {
                match (self.class_of(access.class, at, depth), &access.constant) {
                    (Some(class), ClassLikeConstantSelector::Identifier(name)) => {
                        self.enum_case(&class, &String::from_utf8_lossy(name.value))
                    }
                    _ => TypeExpr::Other,
                }
            }
            _ => TypeExpr::Other,
        }
    }

    /// The enum that declares case `name` reached from `class`; any other constant
    /// has no type.
    /// In a trait that does not declare the case, `self::` reaches the cases of the
    /// classes using the trait.
    fn enum_case(&self, class: &str, name: &str) -> TypeExpr {
        let mut found = self.index.find_member(class, MemberKind::Constant, name);
        if found.is_empty() && self.index.is_trait(class) {
            found = self
                .index
                .trait_users(class)
                .iter()
                .flat_map(|user| self.index.find_member(user, MemberKind::Constant, name))
                .collect();
        }
        union(
            found
                .into_iter()
                .filter(|(_, member)| member.kind == MemberKind::EnumCase)
                .map(|(found, _)| TypeExpr::Named(found.declaration.fqn.clone()))
                .collect(),
        )
    }

    /// The type a method call on `receiver` returns: what the active frameworks give,
    /// or the declared return type. Each member of a union is answered on its own, so
    /// a framework answer for one member never hides the others.
    fn method_call(
        &self,
        receiver: &TypeExpr,
        selector: &ClassLikeMemberSelector<'_>,
        arguments: &ArgumentList<'_>,
        at: u32,
        depth: u32,
        static_class: Option<&str>,
    ) -> TypeExpr {
        if let TypeExpr::Union(members) = receiver
            && self.framework_types(selector)
        {
            return union(
                members
                    .iter()
                    .map(|member| {
                        self.method_call(member, selector, arguments, at, depth, static_class)
                    })
                    .collect(),
            );
        }
        self.framework_type(receiver, selector, arguments, at)
            .unwrap_or_else(|| match selector {
                ClassLikeMemberSelector::Identifier(name) => self.member_type(
                    receiver,
                    MemberKind::Method,
                    &String::from_utf8_lossy(name.value),
                    static_class,
                    Some((arguments, at, depth)),
                ),
                _ => TypeExpr::Other,
            })
    }

    /// The type the active frameworks give a method call on `receiver`, when one of
    /// them knows better than the method's declaration.
    fn framework_type(
        &self,
        receiver: &TypeExpr,
        selector: &ClassLikeMemberSelector<'_>,
        arguments: &ArgumentList<'_>,
        at: u32,
    ) -> Option<TypeExpr> {
        if !self.framework_types(selector) {
            return None;
        }
        let ClassLikeMemberSelector::Identifier(name) = selector else {
            return None;
        };
        let targets = self.call_targets(receiver, &String::from_utf8_lossy(name.value));
        if targets.is_empty() {
            return None;
        }
        let strings = self.string_arguments(arguments, at);
        let call = FrameworkCall {
            targets: &targets,
            strings: &strings,
            receiver: Some(receiver),
        };
        self.frameworks
            .iter()
            .find_map(|framework| framework.call_type(self.index, &call))
    }

    /// Whether a framework module of the file may type calls to the method `selector`
    /// names.
    fn framework_types(&self, selector: &ClassLikeMemberSelector<'_>) -> bool {
        let ClassLikeMemberSelector::Identifier(name) = selector else {
            return false;
        };
        let name = String::from_utf8_lossy(name.value).to_ascii_lowercase();
        self.frameworks
            .iter()
            .any(|framework| framework.typed_methods().contains(&name.as_str()))
    }

    /// `Class::method` for every declaration method `name` resolves to on `receiver`.
    fn call_targets(&self, receiver: &TypeExpr, name: &str) -> Vec<CallTarget> {
        class_names(receiver)
            .iter()
            .flat_map(|class| self.index.find_member(class, MemberKind::Method, name))
            .map(|(declaring, member)| CallTarget::Method {
                class: declaring.declaration.fqn.clone(),
                name: member.name.clone(),
            })
            .collect()
    }

    /// `call`'s string literal argument covering `offset`, with what the call targets.
    fn string_argument(&self, call: &CallRef<'_, '_>, offset: u32) -> Option<StringArgument> {
        self.argument_with(call, |value| string_literal_at(value, offset))
            .map(|(value, argument)| StringArgument { value, ..argument })
    }

    /// The first positional argument of `call` for which `find` gives something,
    /// with what the call targets; the argument's `value` is left empty.
    fn argument_with<T>(
        &self,
        call: &CallRef<'_, '_>,
        find: impl Fn(&Expression<'_>) -> Option<T>,
    ) -> Option<(T, StringArgument)> {
        let arguments = match call {
            CallRef::Method(c) => &c.argument_list,
            CallRef::NullSafe(c) => &c.argument_list,
            CallRef::Static(c) => &c.argument_list,
        };
        let (position, found) =
            arguments
                .arguments
                .iter()
                .enumerate()
                .find_map(|(i, argument)| {
                    let Argument::Positional(argument) = argument else {
                        return None;
                    };
                    find(argument.value).map(|found| (i, found))
                })?;
        let mut argument = self.call_parts(call)?;
        argument.position = position;
        Some((found, argument))
    }

    /// What `call` targets, its literal arguments and its receiver, as a string
    /// argument at position 0 with no value.
    fn call_parts(&self, call: &CallRef<'_, '_>) -> Option<StringArgument> {
        let (arguments, start) = match call {
            CallRef::Method(c) => (&c.argument_list, c.span().start.offset),
            CallRef::NullSafe(c) => (&c.argument_list, c.span().start.offset),
            CallRef::Static(c) => (&c.argument_list, c.span().start.offset),
        };
        let (receiver, selector) = match call {
            CallRef::Method(c) => (self.expr(c.object, start, 0), &c.method),
            CallRef::NullSafe(c) => (self.expr(c.object, start, 0), &c.method),
            CallRef::Static(c) => (
                self.class_of(c.class, start, 0)
                    .map_or(TypeExpr::Other, TypeExpr::Named),
                &c.method,
            ),
        };
        let ClassLikeMemberSelector::Identifier(name) = selector else {
            return None;
        };
        Some(StringArgument {
            targets: self.call_targets(&receiver, &String::from_utf8_lossy(name.value)),
            strings: self.string_arguments(arguments, start),
            receiver,
            position: 0,
            value: String::new(),
        })
    }

    /// The type of each positional argument, up to the first named or spread one.
    fn argument_types(&self, arguments: &ArgumentList<'_>) -> Vec<TypeExpr> {
        arguments
            .arguments
            .iter()
            .map_while(|argument| match argument {
                Argument::Positional(argument) if argument.ellipsis.is_none() => {
                    Some(self.expr(argument.value, argument.value.span().start.offset, 0))
                }
                _ => None,
            })
            .collect()
    }

    /// Positional arguments that are string literals or `X::class`, by position.
    fn string_arguments(&self, arguments: &ArgumentList<'_>, at: u32) -> Vec<Option<String>> {
        arguments
            .arguments
            .iter()
            .map(|argument| match argument {
                Argument::Positional(argument) => self.string_value(argument.value, at),
                Argument::Named(_) => None,
            })
            .collect()
    }

    fn string_value(&self, expression: &Expression<'_>, at: u32) -> Option<String> {
        match expression {
            Expression::Literal(Literal::String(literal)) => literal
                .value
                .map(|v| String::from_utf8_lossy(v).into_owned()),
            Expression::Access(Access::ClassConstant(access)) => match &access.constant {
                ClassLikeConstantSelector::Identifier(name)
                    if name.value.eq_ignore_ascii_case(b"class") =>
                {
                    self.class_of(access.class, at, 0)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn member_on(
        &self,
        receiver: &TypeExpr,
        selector: &ClassLikeMemberSelector<'_>,
        kind: MemberKind,
        static_class: Option<&str>,
    ) -> TypeExpr {
        match selector {
            ClassLikeMemberSelector::Identifier(name) => self.member_type(
                receiver,
                kind,
                &String::from_utf8_lossy(name.value),
                static_class,
                None,
            ),
            _ => TypeExpr::Other,
        }
    }

    /// The type of member `name` on every class of `receiver`. `static` and `$this`
    /// become the receiver class (or `static_class` when late static binding is
    /// forwarded), `self` the declaring class (the using class for a trait), and
    /// `parent` the declaring class's parent.
    fn member_type(
        &self,
        receiver: &TypeExpr,
        kind: MemberKind,
        name: &str,
        static_class: Option<&str>,
        call: Option<(&ArgumentList<'_>, u32, u32)>,
    ) -> TypeExpr {
        let mut types = Vec::new();
        for (class, args) in classes_with_args(receiver) {
            // `static` is the receiver with its arguments, unless late static binding
            // is forwarded from another class.
            let static_type = match static_class {
                Some(forwarded) => TypeExpr::Named(forwarded.to_string()),
                None if !args.is_empty() => TypeExpr::Generic {
                    class: class.clone(),
                    args: args.clone(),
                },
                None => TypeExpr::Named(class.clone()),
            };
            for (found, member) in self.index.find_member(&class, kind, name) {
                let typed = match &member.ty {
                    Some(ty) if !names_no_class(ty) => Some((found, member)),
                    Some(_) => self
                        .index
                        .typed_ancestor(&found.declaration.fqn, kind, name)
                        .or(Some((found, member))),
                    None => self
                        .index
                        .typed_ancestor(&found.declaration.fqn, kind, name),
                };
                let Some((declaring, member)) = typed else {
                    continue;
                };
                let Some(ty) = &member.ty else {
                    continue;
                };
                // A method's own templates take what the call's arguments give them,
                // before the class's templates of the same name could.
                let ty = match call {
                    Some((arguments, at, depth)) if !member.templates.is_empty() => {
                        let bound = self.method_templates(member, arguments, at, depth);
                        substitute(ty, &member.templates, &bound)
                    }
                    _ => ty.clone(),
                };
                let declaration = declaring.declaration;
                let self_class = if declaration.kind == SymbolKind::Trait {
                    class.as_str()
                } else {
                    declaration.fqn.as_str()
                };
                let parent = declaration.class.as_ref().and_then(|s| s.parent.as_deref());
                // The declaring class's templates take the arguments the receiver
                // gives it; without any, they are unknown.
                let ty = match &declaration.class {
                    Some(shape) if !shape.templates.is_empty() => {
                        let given = supertype_args(self.index, &class, &args, &declaration.fqn)
                            .unwrap_or_default();
                        substitute(&ty, &shape.templates, &given)
                    }
                    _ => ty,
                };
                types.push(bind(&ty, &static_type, self_class, parent));
            }
            if kind == MemberKind::Property {
                types.extend(self.assigned_types(&class, name));
            }
        }
        union(types)
    }

    /// The type each of `member`'s templates takes in a call with `arguments`: the
    /// class of an `X::class` argument given to a `class-string<T>` parameter, or the
    /// type of an argument given to a parameter typed `T`; `Other` when unbound.
    fn method_templates(
        &self,
        member: &Member,
        arguments: &ArgumentList<'_>,
        at: u32,
        depth: u32,
    ) -> Vec<TypeExpr> {
        member
            .templates
            .iter()
            .map(|template| {
                member
                    .params
                    .iter()
                    .enumerate()
                    .find_map(|(position, (_, ty))| {
                        let argument = || positional(arguments, position);
                        match ty.as_ref()? {
                            TypeExpr::ClassString(class) if matches!(&**class, TypeExpr::Named(name) if name == template) => {
                                match argument()? {
                                    Expression::Access(Access::ClassConstant(access))
                                        if matches!(&access.constant, ClassLikeConstantSelector::Identifier(name) if name.value.eq_ignore_ascii_case(b"class")) =>
                                    {
                                        self.class_of(access.class, at, depth).map(TypeExpr::Named)
                                    }
                                    _ => None,
                                }
                            }
                            TypeExpr::Named(name) if name == template => {
                                Some(self.expr(argument()?, at, depth + 1))
                            }
                            _ => None,
                        }
                    })
                    .unwrap_or(TypeExpr::Other)
            })
            .collect()
    }

    /// The types the classes of `class`'s parent chain assign to property `name`,
    /// or none when one of them assigns a value of unknown type. An ancestor's
    /// private property is another property. While assignments are being computed,
    /// only the class being computed and its ancestors are read, as their
    /// assignments do not depend on the computation in progress.
    fn assigned_types(&self, class: &str, name: &str) -> Vec<TypeExpr> {
        let mut types = Vec::new();
        for (i, key) in self.index.parent_chain(class).iter().enumerate() {
            if i > 0 && self.index.has_private_property(key, name) {
                continue;
            }
            let assigned = match self.assigning {
                None => self.index.property_types(key, None).get(name).cloned(),
                Some(assigning) if assigning.class == *key => {
                    assigning.read_self.set(true);
                    let found = assigning.current.get(name).cloned();
                    if found.is_none() {
                        assigning.pending.set(true);
                    }
                    found
                }
                Some(assigning)
                    if !assigning.holds(key)
                        && self.index.parent_chain(&assigning.class).contains(key) =>
                {
                    self.index
                        .property_types(key, Some(assigning))
                        .get(name)
                        .cloned()
                }
                Some(_) => None,
            };
            match assigned {
                Some(TypeExpr::Other) => return Vec::new(),
                Some(ty) => types.push(ty),
                None => {}
            }
        }
        types
    }

    /// What calling `function` returns: a named function's return type, or for any
    /// other callee, what its object's `__invoke()` returns.
    fn function_returns(&self, call: &FunctionCall<'_>, at: u32, depth: u32) -> TypeExpr {
        let Expression::Identifier(identifier) = call.function else {
            let callee = self.expr(call.function, at, depth);
            return self.member_type(
                &callee,
                MemberKind::Method,
                "__invoke",
                None,
                Some((&call.argument_list, at, depth)),
            );
        };
        let Some(name) = self
            .names
            .get(&identifier.span().start.offset)
            .filter(|n| n.kind == NameKind::Function)
        else {
            return TypeExpr::Other;
        };
        union(self.index.function_returns(name))
    }

    /// The innermost function-like scope containing `at`, as an index into the scopes.
    fn scope_at(&self, at: u32) -> Option<usize> {
        self.facts
            .scopes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.start <= at && at <= s.end)
            .max_by_key(|(_, s)| s.start)
            .map(|(i, _)| i)
    }

    /// How a binding at `position` affects a use at `at`: decided by the innermost
    /// branch that holds the binding but not the use.
    fn mode(&self, position: u32, at: u32) -> Mode {
        let branch = self
            .facts
            .branches
            .iter()
            .filter(|b| b.start <= position && position <= b.end)
            .filter(|b| !(b.start <= at && at <= b.end))
            .max_by_key(|b| b.start);
        match branch {
            None => Mode::Replace,
            Some(branch) if at <= branch.statement_end => Mode::Ignore,
            Some(_) => Mode::Merge,
        }
    }

    /// The type of variable `name` used at `at`.
    fn var(&self, name: &str, at: u32, depth: u32) -> TypeExpr {
        if depth > MAX_DEPTH {
            return TypeExpr::Other;
        }
        if name == "this" {
            return self.enclosing(at).map_or(TypeExpr::Other, TypeExpr::Named);
        }
        let key = (name.to_string(), at);
        if let Some(ty) = self.memo.borrow().get(&key) {
            return ty.clone();
        }
        let ty = self.var_uncached(name, at, depth);
        self.memo.borrow_mut().insert(key, ty.clone());
        ty
    }

    fn var_uncached(&self, name: &str, at: u32, depth: u32) -> TypeExpr {
        let scope = self.scope_at(at);
        let same_scope = |position: u32| self.scope_at(position) == scope;

        let mut bindings: Vec<(u32, Mode, Binding<'_, 'ast, 'arena>)> = Vec::new();
        if let Some(i) = scope {
            let fact = &self.facts.scopes[i];
            for (p, param) in fact.params.parameters.iter().enumerate() {
                if variable_name(param.variable.name) == name {
                    bindings.push((fact.start, Mode::Replace, Binding::Param(fact, p)));
                }
            }
        }
        for assignment in &self.facts.assignments {
            if assignment.var == name && assignment.end <= at && same_scope(assignment.start) {
                let binding = match assignment.rhs {
                    Some(_) => Binding::Assign(assignment),
                    None => Binding::Unknown,
                };
                bindings.push((assignment.start, self.mode(assignment.start, at), binding));
            }
        }
        for foreach in &self.facts.foreaches {
            if foreach.body_start > at || !same_scope(foreach.start) {
                continue;
            }
            let mode = self.mode(foreach.start, at);
            if foreach.value.as_deref() == Some(name) {
                bindings.push((foreach.start, mode, Binding::ForeachValue(foreach)));
            } else if let Some((_, path)) = foreach.entries.iter().find(|(v, _)| v == name) {
                bindings.push((foreach.start, mode, Binding::ForeachEntry(foreach, path)));
            } else if foreach.unknown.iter().any(|v| v == name) {
                bindings.push((foreach.start, mode, Binding::Unknown));
            }
        }
        for catch in &self.facts.catches {
            if catch.var == name && catch.start <= at && same_scope(catch.start) {
                bindings.push((
                    catch.start,
                    self.mode(catch.start, at),
                    Binding::Catch(catch),
                ));
            }
        }
        for narrowing in &self.facts.narrowings {
            if narrowing.var == name
                && narrowing.start <= at
                && at <= narrowing.end
                && same_scope(narrowing.start)
            {
                bindings.push((
                    narrowing.start,
                    Mode::Replace,
                    Binding::Narrow(narrowing.class),
                ));
            }
        }
        for (position, var, ty) in &self.var_docs {
            if var == name && *position < at && same_scope(*position) {
                bindings.push((
                    *position,
                    self.mode(*position, at),
                    Binding::Doc(ty.clone()),
                ));
            }
        }

        if bindings.iter().all(|(_, mode, _)| *mode == Mode::Ignore) {
            return self.captured(name, scope, depth);
        }
        bindings.sort_by_key(|(position, _, _)| *position);
        let last_replace = bindings
            .iter()
            .rposition(|(_, mode, _)| *mode == Mode::Replace);
        let mut state = State::Unset;
        for (i, (_, mode, binding)) in bindings.iter().enumerate() {
            match last_replace {
                Some(r) if i < r => {}
                Some(r) if i == r => state = self.binding_state(binding, depth),
                _ if *mode == Mode::Merge => {
                    state = state.merge(self.binding_state(binding, depth));
                }
                _ => {}
            }
        }
        let ty = state.into_type();
        match self.enclosing(at) {
            Some(class) => {
                let parent = self.enclosing_parent(at);
                bind(
                    &ty,
                    &TypeExpr::Named(class.clone()),
                    &class,
                    parent.as_deref(),
                )
            }
            None => ty,
        }
    }

    /// A variable with no binding in its own scope: arrow functions see the enclosing
    /// scope, closures see the variables of their `use` clause.
    fn captured(&self, name: &str, scope: Option<usize>, depth: u32) -> TypeExpr {
        let Some(scope) = scope.map(|i| &self.facts.scopes[i]) else {
            return TypeExpr::Other;
        };
        let visible = match &scope.kind {
            ScopeKind::Arrow => true,
            ScopeKind::Closure(uses) => uses.iter().any(|u| u == name),
            ScopeKind::Function => false,
        };
        if visible {
            // Just before the closure starts is the enclosing scope.
            self.var(name, scope.start.saturating_sub(1), depth + 1)
        } else {
            TypeExpr::Other
        }
    }

    fn binding_state(&self, binding: &Binding<'_, '_, '_>, depth: u32) -> State {
        match binding {
            Binding::Param(scope, i) => State::of(self.param_type(scope, *i, depth)),
            Binding::Assign(assignment) if !assignment.path.is_empty() => match assignment.rhs {
                Some(rhs) => State::of(self.entry_at(
                    self.expr(rhs, assignment.start, depth + 1),
                    &assignment.path,
                )),
                None => State::Unknown,
            },
            Binding::Assign(assignment) => {
                if let Some(ty) = self.assignment_doc(assignment) {
                    return State::of(ty);
                }
                match assignment.rhs {
                    Some(Expression::Literal(Literal::Null(_))) => State::Null,
                    Some(rhs) => State::of(self.expr(rhs, assignment.start, depth + 1)),
                    None => State::Unknown,
                }
            }
            Binding::ForeachEntry(foreach, path) => State::of(self.entry_at(
                element_type(
                    self.index,
                    &self.expr(foreach.iterable, foreach.start, depth + 1),
                ),
                path,
            )),
            Binding::ForeachValue(foreach) => State::of(element_type(
                self.index,
                &self.expr(foreach.iterable, foreach.start, depth + 1),
            )),
            Binding::Unknown => State::Unknown,
            Binding::Catch(catch) => State::of(self.hint_type(catch.hint)),
            Binding::Narrow(class) => match self.names.get(class) {
                Some(name) => State::Known(TypeExpr::Named(name.fqn.clone())),
                None => State::Unknown,
            },
            Binding::Doc(ty) => State::of(ty.clone()),
        }
    }

    /// A parameter's PHPDoc `@param` type when it names a class, else its declared type.
    fn param_type(&self, scope: &ScopeFact<'_, '_>, i: usize, depth: u32) -> TypeExpr {
        let Some(param) = scope.params.parameters.iter().nth(i) else {
            return TypeExpr::Other;
        };
        let name = variable_name(param.variable.name);
        let doc_type = get_docblock_before_position(self.program.trivia.as_slice(), scope.start)
            .and_then(|doc| {
                parse_docblock(
                    &String::from_utf8_lossy(doc.value),
                    &self.summary.file_names.at(scope.start),
                )
                .params
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, ty)| ty)
            });
        // A variadic parameter collects several elements into an array.
        let own = if param.hint.is_none() && doc_type.is_none() && param.ellipsis.is_none() {
            self.callback_param(scope, i, depth)
        } else {
            let declared = param.hint.as_ref().map(|hint| self.hint_type(hint));
            pick_type(doc_type, declared).unwrap_or(TypeExpr::Other)
        };
        // Only a parameter without a type or with a container type can take a
        // class from its ancestor: a scalar one cannot.
        let open = param.hint.as_ref().is_none_or(|hint| {
            let span = hint.span();
            let text = self
                .text(span.start.offset, span.end.offset)
                .trim_start_matches('?')
                .to_ascii_lowercase();
            matches!(text.as_str(), "array" | "iterable" | "mixed" | "object")
        });
        if open
            && names_no_class(&own)
            && let Some(inherited) = self.inherited_param(scope, i)
        {
            return inherited;
        }
        own
    }

    /// The value `keys` lead to inside a value of type `ty`.
    fn entry_at(&self, ty: TypeExpr, keys: &[Option<String>]) -> TypeExpr {
        keys.iter()
            .fold(ty, |ty, key| entry_type(self.index, &ty, key.as_deref()))
    }

    /// The type an ancestor's declaration of the method at `scope` gives its
    /// parameter `i`, as an interface documents it for its implementations.
    /// Closures, arrow functions and constructors are not tied to an ancestor's
    /// signature.
    fn inherited_param(&self, scope: &ScopeFact<'_, '_>, i: usize) -> Option<TypeExpr> {
        if !matches!(scope.kind, ScopeKind::Function) {
            return None;
        }
        let fqn = self.summary.enclosing_class(scope.start)?.fqn.as_deref()?;
        let method = self
            .summary
            .declarations
            .iter()
            .filter(|d| d.fqn == fqn)
            .filter_map(|d| d.class.as_ref())
            .flat_map(|shape| shape.members.iter())
            .find(|member| {
                member.kind == MemberKind::Method
                    && member.range.0 <= scope.start
                    && scope.start <= member.range.1
            })?;
        if method.name.eq_ignore_ascii_case("__construct") {
            return None;
        }
        self.index.ancestor_param(fqn, &method.name, i)
    }

    /// The type a method's callable parameter gives parameter `i` of a closure
    /// written at that argument, such as `T` of `Closure(T): bool` with the
    /// receiver's argument for `T`.
    fn method_callback_param(&self, scope: &ScopeFact<'_, '_>, i: usize, depth: u32) -> TypeExpr {
        let Some(callback) = self
            .facts
            .method_callbacks
            .iter()
            .find(|c| c.start == scope.start)
        else {
            return TypeExpr::Other;
        };
        let ClassLikeMemberSelector::Identifier(name) = callback.method else {
            return TypeExpr::Other;
        };
        let name = String::from_utf8_lossy(name.value);
        let receiver = self.expr(callback.object, callback.start, depth + 1);
        let mut types = Vec::new();
        for (class, args) in classes_with_args(&receiver) {
            for (found, member) in self.index.find_member(&class, MemberKind::Method, &name) {
                let Some((_, Some(ty))) = member.params.get(callback.position) else {
                    continue;
                };
                let Some(param) = callable_params(ty).and_then(|params| params.get(i)) else {
                    continue;
                };
                let declaration = found.declaration;
                // The method's own templates are unknown here; the class's take the
                // receiver's arguments.
                let param = substitute(param, &member.templates, &[]);
                let param = match &declaration.class {
                    Some(shape) if !shape.templates.is_empty() => {
                        let given = supertype_args(self.index, &class, &args, &declaration.fqn)
                            .unwrap_or_default();
                        substitute(&param, &shape.templates, &given)
                    }
                    _ => param,
                };
                let static_type = if args.is_empty() {
                    TypeExpr::Named(class.clone())
                } else {
                    TypeExpr::Generic {
                        class: class.clone(),
                        args: args.clone(),
                    }
                };
                let self_class = if declaration.kind == SymbolKind::Trait {
                    class.as_str()
                } else {
                    declaration.fqn.as_str()
                };
                let parent = declaration.class.as_ref().and_then(|s| s.parent.as_deref());
                types.push(bind(&param, &static_type, self_class, parent));
            }
        }
        union(types)
    }

    /// The element type that PHP passes to parameter `i` of a callback written at
    /// a positional argument of a built-in array function.
    fn callback_param(&self, scope: &ScopeFact<'_, '_>, i: usize, depth: u32) -> TypeExpr {
        let Some(callback) = self.facts.callbacks.iter().find(|c| c.start == scope.start) else {
            return self.method_callback_param(scope, i, depth);
        };
        let Some(function) = self.builtin_function(callback.call.function) else {
            return TypeExpr::Other;
        };
        let arguments = &callback.call.argument_list;
        let array = match (function.as_str(), callback.position, i) {
            ("array_map", 0, i) => i + 1,
            ("array_filter", 1, 0) if self.filters_values(arguments) => 0,
            ("array_reduce", 1, 1) => 0,
            ("usort" | "uasort", 1, 0 | 1) => 0,
            ("array_walk" | "array_find" | "array_find_key" | "array_any" | "array_all", 1, 0) => 0,
            _ => return TypeExpr::Other,
        };
        if let Some(value) = positional(arguments, array) {
            return array_element(&self.expr(value, value.span().start.offset, depth + 1));
        }
        // `...$lists` unpacks an array of arrays into the array arguments.
        match spread_covering(arguments, array) {
            Some(value) => array_element(&array_element(&self.expr(
                value,
                value.span().start.offset,
                depth + 1,
            ))),
            None => TypeExpr::Other,
        }
    }

    /// Whether an `array_filter` call passes values to its callback: no mode, or
    /// `ARRAY_FILTER_USE_BOTH`.
    fn filters_values(&self, arguments: &ArgumentList<'_>) -> bool {
        match arguments.arguments.iter().nth(2) {
            None => true,
            Some(Argument::Positional(mode)) => match mode.value {
                Expression::ConstantAccess(access) => {
                    let span = access.name.span();
                    self.text(span.start.offset, span.end.offset)
                        .trim_start_matches('\\')
                        == "ARRAY_FILTER_USE_BOTH"
                }
                _ => false,
            },
            Some(Argument::Named(_)) => false,
        }
    }

    /// The global name of the built-in PHP function `function` calls, if it does.
    fn builtin_function(&self, function: &Expression<'_>) -> Option<String> {
        let Expression::Identifier(identifier) = function else {
            return None;
        };
        let name = self
            .names
            .get(&identifier.span().start.offset)
            .filter(|n| n.kind == NameKind::Function)?;
        self.index.builtin_function(name)
    }

    /// The type a function call returns: the element types built-in array
    /// functions keep, or the declared return type.
    fn function_call(&self, call: &FunctionCall<'_>, at: u32, depth: u32) -> TypeExpr {
        let Some(function) = self.builtin_function(call.function) else {
            return self.function_returns(call, at, depth);
        };
        let first = || {
            positional(&call.argument_list, 0)
                .map_or(TypeExpr::Other, |value| self.expr(value, at, depth))
        };
        match function.as_str() {
            "array_filter" | "array_values" | "array_reverse" | "array_slice" | "array_unique"
            | "array_diff" | "array_intersect" => array_of(array_element(&first())),
            "iterator_to_array" => array_of(element_type(self.index, &first())),
            "reset" | "end" | "current" | "array_pop" | "array_shift" | "array_first"
            | "array_last" => array_element(&first()),
            "array_map" => {
                let hint = match positional(&call.argument_list, 0) {
                    Some(Expression::Closure(closure)) => closure.return_type_hint.as_ref(),
                    Some(Expression::ArrowFunction(arrow)) => arrow.return_type_hint.as_ref(),
                    _ => None,
                };
                hint.map_or(TypeExpr::Other, |hint| array_of(self.hint_type(&hint.hint)))
            }
            _ => self.function_returns(call, at, depth),
        }
    }

    /// The `@var` type of a docblock right before an assignment, when it names a
    /// class and either names this variable or no variable.
    fn assignment_doc(&self, assignment: &AssignFact<'_>) -> Option<TypeExpr> {
        let doc = get_docblock_before_position(self.program.trivia.as_slice(), assignment.start)?;
        parse_docblock(
            &String::from_utf8_lossy(doc.value),
            &self.summary.file_names.at(assignment.start),
        )
        .var
        .into_iter()
        .find(|(var, _)| var.as_deref().is_none_or(|v| v == assignment.var))
        .map(|(_, ty)| ty)
        .and_then(|ty| pick_type(Some(ty), None))
        .filter(|ty| !names_no_class(ty))
    }
}

/// The types the code of `class`, a lowercased class name, assigns to its
/// properties: `$this->p = ...`, and `$x->p = ...` where `$x` is an instance of the
/// class, as in `create()`. `outer` is the computation this one is nested in.
pub(crate) fn property_assignments(
    index: &Index,
    class: &str,
    outer: Option<&Assigning<'_>>,
) -> PropertyTypes {
    let mut paths: Vec<&Path> = index.classes_named(class).iter().map(|c| c.path).collect();
    paths.sort();
    paths.dedup();
    let sources: Vec<(&Path, Vec<u8>)> = paths
        .iter()
        .filter_map(|p| std::fs::read(p).ok().map(|source| (*p, source)))
        .collect();
    let summaries: Vec<FileSummary> = sources.iter().map(|(_, s)| summarize(s)).collect();
    let mut current = PropertyTypes::new();
    for _ in 0..MAX_PASSES {
        let assigning = Assigning {
            class: class.to_string(),
            current: &current,
            outer,
            read_self: Cell::new(false),
            pending: Cell::new(false),
        };
        let mut next = PropertyTypes::new();
        for ((path, source), summary) in sources.iter().zip(&summaries) {
            assignments_in(index, path, summary, source, &assigning, &mut next);
        }
        let settled = !assigning.read_self.get() || next == current;
        current = next;
        if settled {
            break;
        }
    }
    current
}

/// Adds to `found` the types one file assigns to the properties of the class
/// `assigning` computes.
fn assignments_in(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    source: &[u8],
    assigning: &Assigning<'_>,
    found: &mut PropertyTypes,
) {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let mut facts = Facts::default();
    Collect.walk_program(program, &mut facts);
    let mut ctx = Ctx::new(index, path, summary, program, source, &facts);
    ctx.assigning = Some(assigning);
    let is_class = |name: &str| name.eq_ignore_ascii_case(&assigning.class);
    for fact in &facts.properties {
        let inside = summary
            .enclosing_class(fact.start)
            .and_then(|range| range.fqn.as_deref())
            .is_some_and(is_class);
        if !inside
            || !class_names(&ctx.expr(fact.object, fact.start, 0))
                .iter()
                .any(|name| is_class(name))
        {
            continue;
        }
        assigning.pending.set(false);
        let ty = ctx.expr(fact.rhs, fact.start, 0);
        if ty == TypeExpr::Other && (assigning.pending.get() || is_plain_value(fact.rhs)) {
            continue;
        }
        let merged = match found.remove(&fact.name) {
            None => ty,
            Some(TypeExpr::Other) => TypeExpr::Other,
            Some(_) if ty == TypeExpr::Other => TypeExpr::Other,
            Some(known) => union(vec![known, ty]),
        };
        found.insert(fact.name.clone(), merged);
    }
}

/// A literal or an array: a value that is not an object.
fn is_plain_value(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::Literal(_)
        | Expression::CompositeString(_)
        | Expression::Array(_)
        | Expression::LegacyArray(_) => true,
        Expression::Parenthesized(inner) => is_plain_value(inner.expression),
        _ => false,
    }
}

/// The parameter types of the callable in `ty`, alone or in a union.
fn callable_params(ty: &TypeExpr) -> Option<&Vec<TypeExpr>> {
    match ty {
        TypeExpr::Callable(params) => Some(params),
        TypeExpr::Union(members) => members.iter().find_map(callable_params),
        _ => None,
    }
}

/// Whether `ty` says nothing about classes: unknown, or an array of nothing known.
pub(crate) fn names_no_class(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Other | TypeExpr::Scalar(_) | TypeExpr::Callable(_) => true,
        TypeExpr::Shape(entries) => entries.iter().all(|(_, ty)| names_no_class(ty)),
        TypeExpr::ArrayOf(element) => names_no_class(element),
        TypeExpr::Union(members) => members.iter().all(names_no_class),
        _ => false,
    }
}

/// Replaces `static`/`$this` with `static_type`, `self` with `self_class` and
/// `parent` with `parent`.
fn bind(ty: &TypeExpr, static_type: &TypeExpr, self_class: &str, parent: Option<&str>) -> TypeExpr {
    match ty {
        TypeExpr::StaticType => static_type.clone(),
        TypeExpr::SelfType => TypeExpr::Named(self_class.to_string()),
        TypeExpr::ParentType => parent.map_or(TypeExpr::Other, |p| TypeExpr::Named(p.to_string())),
        TypeExpr::ArrayOf(element) => {
            TypeExpr::ArrayOf(Box::new(bind(element, static_type, self_class, parent)))
        }
        TypeExpr::Generic { class, args } => TypeExpr::Generic {
            class: class.clone(),
            args: args
                .iter()
                .map(|arg| bind(arg, static_type, self_class, parent))
                .collect(),
        },
        TypeExpr::Union(members) => union(
            members
                .iter()
                .map(|m| bind(m, static_type, self_class, parent))
                .collect(),
        ),
        TypeExpr::Shape(entries) => TypeExpr::Shape(
            entries
                .iter()
                .map(|(key, ty)| (key.clone(), bind(ty, static_type, self_class, parent)))
                .collect(),
        ),
        TypeExpr::Callable(params) => TypeExpr::Callable(
            params
                .iter()
                .map(|param| bind(param, static_type, self_class, parent))
                .collect(),
        ),
        TypeExpr::ClassString(class) => {
            TypeExpr::ClassString(Box::new(bind(class, static_type, self_class, parent)))
        }
        other => other.clone(),
    }
}

/// The classes a type can be, each with the generic arguments it is written with.
fn classes_with_args(ty: &TypeExpr) -> Vec<(String, Vec<TypeExpr>)> {
    match ty {
        TypeExpr::Generic { class, args } => vec![(class.clone(), args.clone())],
        TypeExpr::Union(members) => members.iter().flat_map(classes_with_args).collect(),
        _ => class_names(ty)
            .into_iter()
            .map(|class| (class, Vec::new()))
            .collect(),
    }
}

/// The class names a type can be: named classes alone or in a union.
pub(crate) fn class_names(ty: &TypeExpr) -> Vec<String> {
    match ty {
        TypeExpr::Named(name)
        | TypeExpr::Tagged { class: name, .. }
        | TypeExpr::Generic { class: name, .. } => vec![name.clone()],
        TypeExpr::Union(members) => members.iter().flat_map(class_names).collect(),
        _ => Vec::new(),
    }
}

/// Flattens and deduplicates; `Other` members drop and a single member stands alone.
pub(crate) fn union(types: Vec<TypeExpr>) -> TypeExpr {
    let mut members: Vec<TypeExpr> = Vec::new();
    for ty in types {
        match ty {
            TypeExpr::Other => {}
            TypeExpr::Union(inner) => {
                for member in inner {
                    if !members.contains(&member) {
                        members.push(member);
                    }
                }
            }
            ty => {
                if !members.contains(&ty) {
                    members.push(ty);
                }
            }
        }
    }
    match members.len() {
        0 => TypeExpr::Other,
        1 => members.pop().expect("one member"),
        _ => TypeExpr::Union(members),
    }
}

/// The value of the argument at `position`, when it and every argument before it
/// are positional and none is spread. PHP puts every positional argument before
/// the named ones.
/// The value of the spread argument whose elements fill `position`: the first
/// spread at or before it, after positional arguments only.
fn spread_covering<'a, 'arena>(
    arguments: &'a ArgumentList<'arena>,
    position: usize,
) -> Option<&'a Expression<'arena>> {
    for argument in arguments.arguments.iter().take(position + 1) {
        match argument {
            Argument::Positional(argument) if argument.ellipsis.is_some() => {
                return Some(argument.value);
            }
            Argument::Positional(_) => {}
            Argument::Named(_) => return None,
        }
    }
    None
}

fn positional<'a, 'arena>(
    arguments: &'a ArgumentList<'arena>,
    position: usize,
) -> Option<&'a Expression<'arena>> {
    // A spread argument before or at `position` moves every later one.
    let spread = arguments
        .arguments
        .iter()
        .take(position + 1)
        .any(|argument| matches!(argument, Argument::Positional(a) if a.ellipsis.is_some()));
    if spread {
        return None;
    }
    match arguments.arguments.iter().nth(position)? {
        Argument::Positional(argument) => Some(argument.value),
        Argument::Named(_) => None,
    }
}
