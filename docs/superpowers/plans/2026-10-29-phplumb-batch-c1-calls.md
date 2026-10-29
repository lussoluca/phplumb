# PHPlumb batch C1: call types and narrowing implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Type the calls and variables the "To fix" inference issues leave unknown: invokable objects, `instanceof` in `match (true)` arms, `static` returns on generic receivers, parameters documented only on the interface, and method-level templates bound from arguments.

**Architecture:** Each fix extends `infer.rs` where the type is computed today: function-call returns read `__invoke()` for an expression callee; `match (true)` arms add narrowing facts like `if`; `bind` keeps the receiver's arguments for `static`; `param_type` falls back to the same parameter of the nearest ancestor declaration; and method calls bind method-level `@template` names from their arguments before the class templates, which needs `class-string<T>` as a type and method templates in the summary.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 5c element types and generics) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goal "Wrong jumps": results come only from resolved types). The issues are the requirements for each task.

**Issues:** lussoluca/phplumb #12, #13, #14, #16 and #21. Batch C2 takes #11, #15, #17, #18, #19 and #20.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No lost results.** Member coverage on both reference projects stays at or above `main`, measured back to back on the same state; cases and diagnostics match `main`.
- **Branch** `fix/12-call-types` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.
- Tests are marker fixtures in a new `tests/fixtures/calls` workspace (`/*@cursor:ID*/` and `/*@target:ID*/`), run by a `calls` test in `tests/fixtures.rs`, unless a task says otherwise.

## Review focus

- **An object without `__invoke()` called as a function** gives nothing. Test in Task 1.
- **A `match (true)` arm whose condition is not an `instanceof`,** or that combines checks with `||`, does not narrow. Test in Task 2.
- **`static` reached through a forwarded call (`self::`, `parent::`)** keeps the forwarded class, not the receiver's arguments. Test in Task 3.
- **An implementation whose own parameter type names a class** keeps it, even when the interface documents another one. Test in Task 4.
- **A method template no argument binds** gives nothing, never the class template of the same name. Test in Task 5.

---

### Task 1: Invokable objects (#12)

**Files:** Modify `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/tests/fixtures.rs`; create `crates/phplumb-index/tests/fixtures/calls/app/Invoke.php`.

- [ ] **Step 1: Write the failing fixture.** A class `SaveTrack` with `__invoke(): Track`, a class `Track` with `toString()`, a property `private SaveTrack $saveTrack`, and `($this->saveTrack)(1)->/*@cursor:invoke*/toString()` with the target on `Track::toString`; a property of a class without `__invoke()` called the same way with a cursor expecting nothing. Add `fn calls() { run_fixture("calls"); }`.
- [ ] **Step 2: Run it and watch it fail.** Run: `cargo test -p phplumb-index --test fixtures calls`. Expected: FAIL on cursor `invoke`.
- [ ] **Step 3: Implement** `function_returns` for a callee that is not a name: the `__invoke` member type of the callee expression's type.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `fix(infer): type calls of invokable objects` with `Closes: lussoluca/phplumb#12`.

### Task 2: Narrowing in `match (true)` arms (#21)

**Files:** Modify `crates/phplumb-index/src/infer.rs`; create `crates/phplumb-index/tests/fixtures/calls/app/MatchTrue.php`.

- [ ] **Step 1: Write the failing fixture.** `match (true) { $message instanceof RunEvidence => $message->/*@cursor:arm*/ownerId, default => null }` with the target on `RunEvidence::$ownerId`; an arm `is_string($message) => $message->/*@cursor:other*/ownerId` and an arm `$message instanceof A || $message instanceof B => $message->/*@cursor:either*/ownerId` expecting nothing.
- [ ] **Step 2: Run it and watch it fail.** Expected: FAIL on cursor `arm`.
- [ ] **Step 3: Implement** a `walk_in_match` that, for a `match` whose subject is the literal `true`, adds a narrowing fact over each arm's expression for every `instanceof` its conditions require, with the rules `instanceof_checks` applies to `if`.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(infer): narrow variables in match true arms` with `Closes: lussoluca/phplumb#21`.

### Task 3: `static` keeps generic arguments (#16)

**Files:** Modify `crates/phplumb-index/src/infer.rs`; create `crates/phplumb-index/tests/fixtures/calls/app/StaticGeneric.php` and its library classes.

- [ ] **Step 1: Write the failing fixture.** A generic class `Bag<K, V>` with `@template K` and `@template V`, a method `filter(): static` and `first(): V`; a parameter `Bag<int, Item> $bag` and `$bag->filter()->first()->/*@cursor:static*/label()` with the target on `Item::label`; inside `Bag`, `self::make()->first()` where `make(): static` keeps nothing generic and expects nothing.
- [ ] **Step 2: Run it and watch it fail.** Expected: FAIL on cursor `static`.
- [ ] **Step 3: Implement** `bind` taking the type `static` stands for: the receiver written with its arguments when late static binding is not forwarded, else the forwarded class.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(infer): keep generic arguments on static returns` with `Closes: lussoluca/phplumb#16`.

### Task 4: Parameter types from the interface (#14)

**Files:** Modify `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/index.rs`; create `crates/phplumb-index/tests/fixtures/calls/app/Implements.php`.

**Interfaces:** Produces `Index::ancestor_param(&self, class: &str, method: &str, position: usize) -> Option<TypeExpr>`: the type of the parameter at `position` of the nearest declaration of `method` among the parents and interfaces of `class`, when it is known.

- [ ] **Step 1: Write the failing fixture.** An interface method `/** @param list<Item> $items */ public function handle(array $items): void;` and an implementation `handle(array $items)` whose body has `foreach ($items as $item) { $item->/*@cursor:inherited*/label(); }`, target `Item::label`; a second implementation `handle(Other $items)` whose `$items->/*@cursor:own*/name()` leads to `Other::name`.
- [ ] **Step 2: Run it and watch it fail.** Expected: FAIL on cursor `inherited`.
- [ ] **Step 3: Implement** `ancestor_param` and the fallback in `param_type` when the parameter's own type gives no class.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(infer): take parameter types from the interface` with `Closes: lussoluca/phplumb#14`.

### Task 5: Method-level templates (#13)

**Files:** Modify `crates/phplumb-index/src/phpdoc.rs`, `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/hover.rs`; create `crates/phplumb-index/tests/fixtures/calls/app/MethodTemplates.php` and library classes.

**Interfaces:** Produces `TypeExpr::ClassString(Box<TypeExpr>)` for `class-string<T>` (a plain `class-string` stays `Other`), `Member::templates: Vec<String>` (`#[serde(default)]`, a method's own `@template` names), and template binding in method calls: each method template takes the class of an `X::class` argument given to a `class-string<T>` parameter, or the type of an argument given to a parameter typed `T`; unbound method templates become `Other`, before class templates are substituted.

- [ ] **Step 1: Write the failing fixture.** A class `Manager` with `/** @template T @param class-string<T> $class @return Repository<T> */ public function getRepository(string $class)`, a generic `Repository<T>` with `/** @return list<T> */ findBy()`; `$manager->getRepository(Post::class)->findBy()[0]->/*@cursor:bound*/title()` with the target on `Post::title`; a class whose class template is `T` and a method with its own `@template T` given `Comment::class` leads to `Comment`, cursor `shadow`; a call with a non-literal argument expects nothing, cursor `unbound`. In `tests/phpdoc` unit tests (in `phpdoc.rs`), `class-string<Foo>` parses as `ClassString(Named("Foo"))`.
- [ ] **Step 2: Run them and watch them fail.** Expected: FAIL on cursors `bound` and `shadow`, and the parse test.
- [ ] **Step 3: Implement** the type, the member templates and the binding; hover shows `class-string<T>` as written.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(infer): bind method-level templates from arguments` with `Closes: lussoluca/phplumb#13`.

### Task 6: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run both reference benchmarks with coverage, diagnostics and cases, back to back, twice. Expected: coverage at or above `main`; cases and diagnostics equal; times within noise. Check a sample of the new resolutions by hand.
- [ ] **Step 2:** Record a section `2026-10-29, batch C1 call types` in `docs/benchmarks.md`, format it, and commit `docs: record batch c1 results`.
