use std::path::{Path, PathBuf};

use phplumb_index::discover::DiscoverOptions;
use phplumb_index::phpdoc::TypeExpr;
use phplumb_index::summary::{ClassShape, MemberKind, summarize};
use phplumb_index::{Index, Location};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/elements")
        .canonicalize()
        .unwrap()
}

fn build() -> Index {
    Index::build(&fixture(), &DiscoverOptions::default())
        .unwrap()
        .0
}

/// The file `file` of the fixture, its content, and the offset of the `nth`
/// occurrence of `needle` in it.
fn at(file: &str, needle: &str, nth: usize) -> (PathBuf, Vec<u8>, u32) {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8(source.clone()).unwrap();
    let offset = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of {needle} in {file}"))
        .0;
    (path, source, offset as u32)
}

fn definition(index: &mut Index, file: &str, needle: &str, nth: usize) -> Vec<Location> {
    let (path, source, offset) = at(file, needle, nth);
    index.definition(&path, &source, offset)
}

/// Each location as its path relative to the fixture and the text at its range.
fn described(locations: &[Location]) -> Vec<(String, String)> {
    locations
        .iter()
        .map(|location| {
            let text = std::fs::read(&location.path).unwrap();
            let range = &text[location.start as usize..location.end as usize];
            let relative = location
                .path
                .strip_prefix(fixture())
                .unwrap_or(&location.path);
            (
                relative.display().to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

fn shape(source: &str, fqn: &str) -> ClassShape {
    summarize(source.as_bytes())
        .declarations
        .into_iter()
        .find(|d| d.fqn == fqn)
        .and_then(|d| d.class)
        .unwrap_or_else(|| panic!("no class {fqn}"))
}

fn named(fqn: &str) -> TypeExpr {
    TypeExpr::Named(fqn.to_string())
}

#[test]
fn generic_doc_types_keep_their_arguments() {
    let shape = shape(
        "<?php namespace App; use Lib\\Collection; class S { /** @var Collection<int, Item> */ public $c; }",
        "App\\S",
    );
    let property = shape
        .members
        .iter()
        .find(|m| m.kind == MemberKind::Property && m.name == "c")
        .expect("property c");
    assert_eq!(
        property.ty,
        Some(TypeExpr::Generic {
            class: "Lib\\Collection".to_string(),
            args: vec![TypeExpr::Scalar("int".to_string()), named("App\\Item")],
        })
    );
}

#[test]
fn class_templates_and_generic_supertypes() {
    let shape = shape(
        "<?php namespace Lib;\n\
         /**\n\
          * @template TKey\n\
          * @template-covariant T\n\
          * @template-extends \\IteratorAggregate<TKey, T>\n\
          */\n\
         interface C extends \\IteratorAggregate {}",
        "Lib\\C",
    );
    assert_eq!(shape.templates, ["Lib\\TKey", "Lib\\T"]);
    assert_eq!(
        shape.generic_supertypes,
        [(
            "IteratorAggregate".to_string(),
            vec![named("Lib\\TKey"), named("Lib\\T")]
        )]
    );
}

#[test]
fn hover_shows_generic_arguments() {
    let mut index = build();
    let (path, source, offset) = at("app/Hover.php", "$c;", 0);
    let hover = index.hover(&path, &source, offset).expect("a hover");
    assert!(
        hover
            .markdown
            .contains("\\Lib\\Collection<int, \\App\\Item> $c"),
        "{}",
        hover.markdown
    );
}

/// The `label` method of `App\Item`.
fn item_label() -> [(String, String); 1] {
    [("app/Item.php".to_string(), "label".to_string())]
}

/// Go to definition on the `nth` `label();` of `file`.
fn label(index: &mut Index, file: &str, nth: usize) -> Vec<(String, String)> {
    described(&definition(index, file, "label();", nth))
}

#[test]
fn foreach_over_a_generic_collection() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 0), item_label());
}

#[test]
fn foreach_over_a_concrete_collection_subclass() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Uses.php", 0), item_label());
}

#[test]
fn foreach_over_single_argument_iterables() {
    let mut index = build();
    assert_eq!(
        label(&mut index, "app/Uses.php", 1),
        item_label(),
        "Iterator"
    );
    assert_eq!(
        label(&mut index, "app/Uses.php", 2),
        item_label(),
        "Generator"
    );
}

#[test]
fn foreach_over_a_nullable_typed_array() {
    let mut index = build();
    assert_eq!(
        label(&mut index, "app/Uses.php", 3),
        item_label(),
        "foreach"
    );
    assert_eq!(label(&mut index, "app/Uses.php", 4), item_label(), "access");
}

#[test]
fn array_access_on_typed_arrays() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 1), item_label());
    assert_eq!(label(&mut index, "app/Shelf.php", 2), item_label());
}

#[test]
fn array_access_on_a_generic_collection() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 3), item_label());
}

#[test]
fn undeclared_elements_give_nothing() {
    let mut index = build();
    for nth in [4, 5, 6] {
        assert!(label(&mut index, "app/Shelf.php", nth).is_empty(), "{nth}");
    }
}

#[test]
fn cyclic_generic_supertypes_end() {
    let mut index = build();
    assert!(label(&mut index, "app/Uses.php", 5).is_empty());
}

#[test]
fn template_returns_take_the_receiver_arguments() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 7), item_label());
}

#[test]
fn template_arrays_take_the_receiver_arguments() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 8), item_label());
}

#[test]
fn generic_returns_continue_the_chain() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Shelf.php", 9), item_label());
}

#[test]
fn fewer_arguments_leave_templates_unknown() {
    let mut index = build();
    assert_eq!(label(&mut index, "app/Uses.php", 6), item_label());
    assert!(label(&mut index, "app/Uses.php", 7).is_empty());
}

#[test]
fn unbound_templates_give_nothing() {
    let mut index = build();
    assert!(label(&mut index, "app/Shelf.php", 10).is_empty());
}

/// Go to definition on the `nth` `label()` of `app/Arrays.php`.
fn in_arrays(index: &mut Index, nth: usize) -> Vec<(String, String)> {
    described(&definition(index, "app/Arrays.php", "label()", nth))
}

#[test]
fn array_map_callbacks_receive_elements() {
    let mut index = build();
    assert_eq!(in_arrays(&mut index, 0), item_label());
}

#[test]
fn array_map_feeds_each_array_to_its_parameter() {
    let mut index = build();
    assert_eq!(in_arrays(&mut index, 1), item_label());
}

#[test]
fn array_filter_modes() {
    let mut index = build();
    assert_eq!(in_arrays(&mut index, 2), item_label(), "default");
    assert_eq!(in_arrays(&mut index, 3), item_label(), "both");
    assert!(in_arrays(&mut index, 4).is_empty(), "key");
}

#[test]
fn sort_and_reduce_callbacks() {
    let mut index = build();
    for nth in [5, 6, 7] {
        assert_eq!(in_arrays(&mut index, nth), item_label(), "{nth}");
    }
    assert!(in_arrays(&mut index, 8).is_empty(), "carry");
}

#[test]
fn typed_callback_parameters_keep_their_type() {
    let mut index = build();
    assert_eq!(
        in_arrays(&mut index, 9),
        [("app/Other.php".to_string(), "label".to_string())]
    );
}

#[test]
fn callbacks_on_undeclared_arrays_give_nothing() {
    let mut index = build();
    assert!(in_arrays(&mut index, 10).is_empty());
}

#[test]
fn named_arguments_give_nothing() {
    let mut index = build();
    assert!(in_arrays(&mut index, 11).is_empty());
}

#[test]
fn project_functions_named_like_built_ins_give_nothing() {
    let mut index = build();
    let found = definition(&mut index, "app/Shadow.php", "label()", 0);
    assert!(found.is_empty());
}

#[test]
fn array_functions_keep_element_types() {
    let mut index = build();
    for nth in [12, 13, 14] {
        assert_eq!(in_arrays(&mut index, nth), item_label(), "{nth}");
    }
}

#[test]
fn mixed_template_prefixes_keep_their_order() {
    let shape = shape(
        "<?php namespace Lib;\n\
         /**\n\
          * @phpstan-template TKey of array-key\n\
          * @template-covariant T\n\
          * @template-extends \\IteratorAggregate<TKey, T>\n\
          * @phpstan-extends \\IteratorAggregate<TKey, T>\n\
          */\n\
         interface C extends \\IteratorAggregate {}",
        "Lib\\C",
    );
    assert_eq!(shape.templates, ["Lib\\TKey", "Lib\\T"]);
    assert_eq!(shape.generic_supertypes.len(), 1);
}

/// Go to definition on the `nth` `label()` of `app/Review.php`.
fn in_review(index: &mut Index, nth: usize) -> Vec<(String, String)> {
    described(&definition(index, "app/Review.php", "label()", nth))
}

#[test]
fn one_template_iterables_walk_their_parents() {
    let mut index = build();
    assert_eq!(
        in_review(&mut index, 0),
        [("lib/Box.php".to_string(), "label".to_string())]
    );
}

#[test]
fn iterators_with_one_argument_bind_the_value() {
    let mut index = build();
    assert_eq!(in_review(&mut index, 1), item_label(), "current");
    assert!(in_review(&mut index, 2).is_empty(), "key");
}

#[test]
fn generators_with_one_argument_bind_the_value() {
    let mut index = build();
    assert_eq!(in_review(&mut index, 3), item_label(), "current");
    assert!(in_review(&mut index, 4).is_empty(), "key");
}

#[test]
fn collections_with_one_argument_give_only_the_element() {
    let mut index = build();
    assert_eq!(in_review(&mut index, 5), item_label(), "foreach");
    assert!(in_review(&mut index, 6).is_empty(), "key");
}

#[test]
fn spread_arguments_give_nothing() {
    let mut index = build();
    assert!(in_review(&mut index, 7).is_empty());
}

#[test]
fn variadic_callback_parameters_get_nothing() {
    let mut index = build();
    let (path, source, offset) = at("app/Review.php", "$vs, $items", 0);
    let hover = index.hover(&path, &source, offset);
    assert!(
        hover.as_ref().is_none_or(|h| !h.markdown.contains("Item")),
        "{hover:?}"
    );
}

#[test]
fn objects_get_no_array_elements() {
    let mut index = build();
    assert!(in_review(&mut index, 8).is_empty(), "current");
    assert!(in_review(&mut index, 9).is_empty(), "array_walk");
}
