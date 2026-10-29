# PHPlumb batch C2: generic and element types implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the generic and element types the remaining "To fix" inference issues leave unknown: conditional return types, scalar generic arguments in hover, spread arrays given to callbacks, generic arguments through `parent::`, `@use` and non-template classes, callback parameters of generic methods, and array shapes with destructuring.

**Architecture:** The PHPDoc type parser gains conditional types (the union of both branches), `Scalar` arguments inside generics, `Callable` parameter lists and `Shape` entries. Inference reads them where it already reads arrays and generics: offsets, `foreach` and destructuring, callback parameters, `parent::` receivers and trait templates.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 5c element types and generics) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goal "Wrong jumps"). The issues are the requirements for each task.

**Issues:** lussoluca/phplumb #11, #15, #17, #18, #19 and #20. The method-template half of #17 was fixed in batch C1; this batch fixes its conditional types.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No lost results.** Member coverage on both reference projects stays at or above `main`, measured back to back on the same state; no existing target changes; cases and diagnostics match `main`.
- **Branch** `fix/11-generic-types` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.
- **New types never stand for objects.** `Scalar`, `Callable` and `Shape` give no class to `class_names`, `classes_with_args` or diagnostics; only their documented readers look inside them.
- Tests are cursor fixtures in a new `tests/fixtures/generics` workspace run by a `generics` test in `tests/fixtures.rs`, plus `phpdoc.rs` unit tests for the parser.

## Review focus

- **A conditional type whose branches name different classes** gives both, never only the subject. Test in Task 1.
- **A shape key that is not a literal** (`$record[$key]`) gives nothing, and a missing key gives nothing. Test in Task 6.
- **A spread of something that is not an array of arrays** gives nothing. Test in Task 3.
- **A callable parameter list longer than the closure's** binds only the closure's parameters, by position. Test in Task 5.
- **`@use` on one trait** does not leak its arguments to another trait of the same class. Test in Task 4.

---

### Task 1: Conditional return types (#17)

**Files:** Modify `crates/phplumb-index/src/phpdoc.rs`; create `tests/fixtures/generics/app/Conditional.php`; add `fn generics()` to `tests/fixtures.rs`.

- [ ] **Step 1: Write the failing tests.** Unit tests: `(T is null ? Foo : Bar)` parses as the union of `Foo` and `Bar`; `($x is string ? Foo : Bar)` and `(T is not null ? Foo : null)` likewise. Fixture: a generic `Holder<T>` with `@return (T is null ? Fallback : Item)` and a `Holder<Other>` receiver whose call leads to both `Fallback::name` and `Item::name`, never to `Other`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the conditional form in `primary`: after a parenthesized type, `is [not] Type ? A : B` gives `A | B`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(phpdoc): read both branches of conditional types` with `Closes: lussoluca/phplumb#17`.

### Task 2: Scalar generic arguments in hover (#18)

**Files:** Modify `crates/phplumb-index/src/phpdoc.rs`, `crates/phplumb-index/src/hover.rs`, `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/src/infer.rs`; `tests/hover.rs`.

**Interfaces:** Produces `TypeExpr::Scalar(String)`, made only for a scalar or pseudo-type name inside generic arguments, kept as written; everything but hover reads it like `Other`.

- [ ] **Step 1: Write the failing tests.** Unit: `Collection<int, Item>` parses with `Scalar("int")` first. `tests/hover.rs`, `hover_shows_scalar_generic_arguments`: a variable documented `Collection<int, Item>` hovers as `\Lib\Collection<int, \App\Item>`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the variant, its parser rule, hover display and the readers that treat unknown types (`names_no_class`, `names_a_class`).
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(hover): show scalar generic arguments as written` with `Closes: lussoluca/phplumb#18`.

### Task 3: Spread arrays given to callbacks (#20)

**Files:** Modify `crates/phplumb-index/src/infer.rs`; create `tests/fixtures/generics/app/Spread.php`.

- [ ] **Step 1: Write the failing fixture.** `/** @param list<list<Item>> $lists */` and `array_map(fn ($x) => $x->/*@cursor:spread*/name(), ...$lists)` with the target on `Item::name`; a spread of `list<Item>` (not an array of arrays) expects nothing.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** in `callback_param`: when a spread argument covers the array position, the element of the spread value's element.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(infer): type callback parameters from spread arrays` with `Closes: lussoluca/phplumb#20`.

### Task 4: Generic arguments through `parent::`, traits and plain classes (#19)

**Files:** Modify `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/generics.rs`; create `tests/fixtures/generics/app/Inherited.php`.

**Interfaces:** Produces `@use Trait<X>` docblocks on trait `use` statements (and in the class docblock) recorded in `ClassShape::generic_supertypes`; `parent::m()` typed on the parent written with the arguments the class's `@extends` gives it.

- [ ] **Step 1: Write the failing fixture.** `/** @template T */ class Base { /** @return T */ get() }` and `/** @extends Base<Item> */ class Sub extends Base { run() { parent::get()->/*@cursor:parent*/name(); } }`; `/** @template T */ trait Holds { /** @return T */ held() }` used by `class Shelf { /** @use Holds<Item> */ use Holds; }` with `$shelf->held()->/*@cursor:trait*/name()`; a second trait of `Shelf` with its own `T` gives nothing, cursor `leak`; a non-template class documented `Plain<int, Item>` whose `own()` call leads to `Plain::own`, cursor `plain`.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** reading `@use`, the `parent::` receiver, and whatever the `plain` cursor shows is missing.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(infer): keep generic arguments through parent and traits` with `Closes: lussoluca/phplumb#19`.

### Task 5: Callback parameters of generic methods (#15)

**Files:** Modify `crates/phplumb-index/src/phpdoc.rs`, `crates/phplumb-index/src/infer.rs`; create `tests/fixtures/generics/app/Callbacks.php`.

**Interfaces:** Produces `TypeExpr::Callable(Vec<TypeExpr>)` for `callable(A, B): R` and `Closure(A, B): R` (parameter types only); closures and arrow functions given positionally to a method call record a callback fact; an untyped closure parameter takes the method parameter's callable parameter type at its position, with the receiver's class templates substituted.

- [ ] **Step 1: Write the failing tests.** Unit: `Closure(T, int): bool` parses as `Callable([Named(T), Other])`. Fixture: `/** @template T */ class Items { /** @param Closure(T): bool $p @return static */ filter(Closure $p) }` and `/** @param Items<Item> $items */` with `$items->filter(fn ($i) => $i->/*@cursor:callback*/name())`; a closure with two parameters given to `Closure(T)` types only the first, cursor `extra` expects nothing on the second.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the parser rule, the method callback facts and the binding.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(infer): type callbacks of generic methods` with `Closes: lussoluca/phplumb#15`.

### Task 6: Array shapes and destructuring (#11)

**Files:** Modify `crates/phplumb-index/src/phpdoc.rs`, `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/generics.rs`; create `tests/fixtures/generics/app/Shapes.php`.

**Interfaces:** Produces `TypeExpr::Shape(Vec<(String, TypeExpr)>)` for `array{key: T, ...}`, `array{T, U}` and `list{T, U}` (positional keys `0`, `1`, ...; optional `key?` kept); `$value['key']` with a literal key gives the entry; destructuring `[$a, $b] = $pair`, `['k' => $a] = $pair` and `foreach ($rows as [$a, $b])` bind each variable to its entry, or to the element of an `ArrayOf`.

- [ ] **Step 1: Write the failing tests.** Unit: `array{startedAt: \DateTimeImmutable, id?: int}` parses into two entries. Fixture: `$record['startedAt']->/*@cursor:shape*/format()` on `array{startedAt: Clock}` with the target on `Clock::format`; `[$a, $b] = $pair` on `array{Item, Other}` leads `$a->name()` to `Item::name` and `$b->size()` to `Other::size`; `foreach ($rows as [$a, $b])` on `list<array{Item, Other}>` likewise; `$record[$key]` and `$record['missing']` give nothing.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the parser rule and the readers.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(infer): read array shapes and destructured variables` with `Closes: lussoluca/phplumb#11`.

### Task 7: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run both reference benchmarks with coverage, diagnostics and cases, back to back. Expected: coverage at or above `main`, cases and diagnostics as on `main`.
- [ ] **Step 2:** Record `2026-10-29, batch C2 generic and element types` in `docs/benchmarks.md`, format it, commit `docs: record batch c2 results`.
