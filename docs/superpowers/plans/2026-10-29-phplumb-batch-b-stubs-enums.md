# PHPlumb batch B: stubs and enums implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the open "To fix" issues about built-in stubs and enums: duplicate implicit interfaces, enum case accesses that give no type, references of polyfills, raw attributes in hover, and missing tests and docs.

**Architecture:** Enum declarations add an implicit interface only when the explicit list lacks it. Inference reads the class of `$var::` from the variable's type, and `self::` inside a trait reaches the enum cases of the classes using the trait. References of a class, function or constant are listed only when the name resolves to the declaration asked about. Stub summaries keep the PHP 8.5 type of each parameter and return that `LanguageLevelTypeAware` gives, and hover renders stub signatures without attributes.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (steps 5a built-in members and 5b enums) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals "Wrong jumps" and "False alarms"). The issues are the requirements for each task.

**Issues:** lussoluca/phplumb #4, #5, #7, #8, #9 and #10.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No lost results.** Member coverage on both reference projects stays at or above `main` (Drupal 42,929, Symfony 13,097 on today's checkouts); cases and diagnostics match `main`.
- **Branch** `fix/4-stubs-and-enums` from `main`. Conventional commits; `Closes: lussoluca/phplumb#N` for each issue a commit fixes, and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A trait used by several classes, only some of them enums,** gives `self::Low` the enum case of the enums that declare it, and nothing for the others. Test in Task 2.
- **`$x::from()` on a variable holding a class-name string** gives nothing, never a wrong class. Test in Task 2.
- **A project class with a built-in name, in project code,** still lists its own references: only a polyfill that loses to the stub lists none. Test in Task 3.
- **A stub parameter with both a native type and `LanguageLevelTypeAware`** keeps one type in hover. Test in Task 4.
- **Hover on a project method with parameter attributes** stays as before. Test in Task 4.

---

### Task 1: Implicit enum interfaces once (#9) and enum tests (#7, #8)

**Files:**

- Modify: `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/tests/enums.rs`, `crates/phplumb-index/tests/fixtures/enums/app/*.php`, `crates/phplumb-index/tests/fixtures/diagnostics/**`, `README.md`

- [ ] **Step 1: Write the tests.** In `tests/enums.rs`: `implicit_interfaces_are_listed_once` (`enum S implements \UnitEnum {}` gives `["UnitEnum"]`; `enum B: int implements \BackedEnum {}` gives `["BackedEnum", "UnitEnum"]`); `static_cases_resolve_inside_the_enum` (`static::Low->label()` in `Severity` leads to `label`); `pure_enum_cases_have_no_value` (`Plain::One->value` gives nothing). In the `diagnostics` fixture, a file calling `$unit->nowhere()`, `$backed->nowhere()` and `$labelled->nowhere()` (a project interface extending `\UnitEnum`), each marked `/*@broken*/`, and `$unit->cases()`, `$backed->from('x')` and `$labelled->cases()` unmarked.
- [ ] **Step 2: Run them.** Run: `cargo test -p phplumb-index --test enums --test fixtures`. Expected: `implicit_interfaces_are_listed_once` FAILS; the other new checks pass already and stay as guards.
- [ ] **Step 3: Implement** adding an implicit interface only when the explicit list does not name it, compared case-insensitively without a leading backslash; add one sentence to the README diagnostics bullet: members of built-in classes are never flagged, except on `UnitEnum` and `BackedEnum`, whose members PHP fixes.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `fix(enums): list implicit enum interfaces once` with `Closes:` #7, #8 and #9.

### Task 2: Enum case accesses through variables and traits (#10)

**Files:**

- Modify: `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/tests/enums.rs`, `crates/phplumb-index/tests/fixtures/enums/app/*.php`

**Interfaces:**

- Produces: `class_of` reads `$this` as the enclosing class and another variable as the single class its inferred type names (none for a union, a string or an unknown type). `pub(crate) fn Index::trait_users(&self, fqn: &str) -> Vec<String>`; an enum case lookup on a trait that does not declare the case unions the case over the trait's users; a definition on `self::Low` in such a trait leads to the users' cases.

- [ ] **Step 1: Write the failing tests.** Fixture: `Severity` gains `$this::Low->label()`; `Uses.php` gains `$severity::from('low')->label()` and `$name::from('low')->label()` with `string $name`; a trait `Labels` with `self::Low->label()`, used by `Severity` and by a plain class `Tagged` that declares no `Low`. Tests in `tests/enums.rs`: `this_cases_resolve` and `variable_classes_resolve` (each `label` leads to `Severity::label`); `string_class_names_give_nothing`; `trait_self_cases_reach_the_using_enums` (`label` in `Labels` leads to `Severity::label` only, and `Low` there to the `Severity` case).
- [ ] **Step 2: Run them and watch them fail.** Run: `cargo test -p phplumb-index --test enums`. Expected: the three positive tests FAIL.
- [ ] **Step 3: Implement** the variable and `$this` cases of `class_of` (with the caller's depth), and the trait fallback for enum cases in inference and in definitions.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(enums): type case accesses through variables and traits` with `Closes: lussoluca/phplumb#10`.

### Task 3: References only for the declaration names resolve to (#4)

**Files:**

- Modify: `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/tests/stubs.rs`, `crates/phplumb-index/tests/references.rs`

**Interfaces:**

- Produces: `Index::references` skips a class, function or constant target that the name itself does not resolve to (the holder's entries for its key do not include the target).

- [ ] **Step 1: Write the failing tests.** In `tests/stubs.rs`, `polyfill_references_are_empty`: references on `Stringable` in the polyfill file give nothing. In `tests/references.rs`, `project_classes_with_built_in_names_keep_their_references`: a global class `Stringable` declared in project code, which wins over the stub, lists its use in another project file.
- [ ] **Step 2: Run them and watch them fail.** Expected: `polyfill_references_are_empty` FAILS listing `app/Uses.php`.
- [ ] **Step 3: Implement** the resolution check.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(references): list only uses that resolve to the declaration` with `Closes: lussoluca/phplumb#4`.

### Task 4: Stub signatures in hover (#5)

**Files:**

- Modify: `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/src/stubs.rs`, `crates/phplumb-index/src/hover.rs`, `crates/phplumb-index/tests/stubs.rs`, `crates/phplumb-index/tests/hover.rs`

**Interfaces:**

- Produces: `Member` and `Declaration` gain `#[serde(default)] pub param_attributes: Vec<(u32, u32, Vec<Metadata>)>`, the range of each parameter's attribute lists relative to the signature start, with their metadata, empty when no parameter has attributes. `summarize_stub` rewrites stub signatures: each attribute list is dropped, `LanguageLevelTypeAware` puts its PHP 8.5 type text before an untyped parameter, and a return type that only `LanguageLevelTypeAware` gives is appended. Hover summarizes stub files with `summarize_stub`.

- [ ] **Step 1: Write the failing tests.** In `tests/stubs.rs`, `hover_shows_language_level_types`: hover on `modify` in `app/Uses.php` contains `DateTimeImmutable::modify(string $modifier): DateTimeImmutable` and no `#[`; `stub_signatures_keep_one_type` on a stub source with `function f(#[LanguageLevelTypeAware(['8.0' => 'int'], default: '')] int $a) {}` gives a signature with `int $a` once. In `tests/hover.rs`, `project_parameter_attributes_stay_in_hover`: a project method `f(#[\SensitiveParameter] string $p)` hovers with the attribute as written.
- [ ] **Step 2: Run them and watch them fail.** Expected: the two stub tests FAIL.
- [ ] **Step 3: Implement** the attribute ranges, the stub rewrite and hover's stub summary.
- [ ] **Step 4: Run every suite.** Expected: all pass.
- [ ] **Step 5: Commit** `fix(stubs): show language level types in hover` with `Closes: lussoluca/phplumb#5`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` in a scratch worktree and this branch; run both benchmarks with coverage, diagnostics and cases, alternating. Expected: coverage at or above `main`, cases and diagnostics equal, times within noise.
- [ ] **Step 2:** Record a section `2026-10-29, batch B stubs and enums` in `docs/benchmarks.md`, format it, and commit `docs: record batch b results`.
