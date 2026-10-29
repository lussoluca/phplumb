# PHPlumb batch F1: Symfony template names implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open the template names issue #39 lists (Twig lists, conditionals, parenthesized tags, `~%}`, `block()`, `form_theme`, names Twig normalizes, `TemplatedEmail`, environment `twig.yaml` paths) and add the template tests issue #40 asks for.

**Architecture:** The Twig reader learns more places where a literal names a template; names a template may lack (list elements, conditional branches) open their file but are never flagged, as list elements already are. Template lookup normalizes names as Twig's filesystem loader does. `TemplatedEmail` joins the template methods, and `twig.yaml` files in environment directories add their namespaced paths like `when@<env>` blocks.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6c templates, 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #39 and #40. Batch F2 takes #37 and #38. Open follow-ups read before planning: none touch templates.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A template name the template may lack (an element of a list, a branch of a conditional) is never flagged; a name Twig refuses (`..` above the root) stays unknown.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/39-template-names` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A conditional whose branches are not both literals** (`cond ? 'a' : name`) opens the literal branch and flags nothing. Test in Task 1.
- **A `~` that concatenates** (`{% include 'a' ~ b %}`) still makes the literal no template name, while `~%}` does not. Test in Task 1.
- **`block('name')` with one argument** is no template name. Test in Task 1.
- **`../` that climbs above a template directory** stays unknown, never missing. Test in Task 2.
- **A `twig.yaml` in an environment directory with a main-namespace path or `default_path`** keeps main-namespace names unknown. Test in Task 3.

---

### Task 1: Template names in more Twig forms (#39)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/twig.rs`; tests in `twig.rs` and `tests/symfony.rs`.

- [ ] **Step 1: Write the failing tests.** Unit tests: `template_name_at` gives `a` for `{{ include(['a', 'b']) }}`, `{% extends x ? 'a' : 'b' %}` (both branches), `{% extends('a') %}`, `{% include 'a' ~%}`, `{{ block('n', 'a') }}`, `{% form_theme form 'a' %}` and `{% form_theme form with ['a', 'b'] %}`; nothing for `{% include 'a' ~ b %}`, `{{ block('a') }}` and `{% extends x ? 'a' : y %}` on `y`. `names()` lists `extends('a')`, `~%}`, `block()` and single `form_theme` names, and leaves out list elements and conditional branches.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the forms in `template_name_at` and `names()`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): read more twig template names` with `Refs: lussoluca/phplumb#39`.

### Task 2: Names Twig normalizes, named arguments and `TemplatedEmail` (#39)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/templates.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`; fixture classes; tests in `tests/symfony.rs`.

**Interfaces:** `Templates::status` reads `\` as `/`, collapses `//`, and gives `Unknown` when `..` climbs above the directory, as `FilesystemLoader::validateName` refuses it; `TEMPLATE_METHODS` gains `Symfony\Bridge\Twig\Mime\TemplatedEmail::htmlTemplate` and `textTemplate`.

- [ ] **Step 1: Write the failing tests.** `normalized_template_names_open_their_file` (`extra/../templates/base.html.twig` style names relative to a configured directory, `a//b`); `climbing_template_names_stay_unknown` (`../x.html.twig`); `templated_emails_open_their_templates`; `named_view_arguments_open_their_template` (`render(view: 'x')` opens it and a missing one is flagged); `class_template_attributes_open_their_template`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): normalize template names and read templated emails` with `Refs: lussoluca/phplumb#39`.

### Task 3: `twig.yaml` in environment directories (#39)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/templates.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `environment_twig_files_add_namespaced_paths` (`config/packages/dev/twig.yaml` adds `@Dev`, and main-namespace names stay certain); `environment_twig_files_with_main_paths_keep_main_names_unknown`; `environment_twig_php_keeps_names_unknown`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** reading `config/packages/<env>/twig.yaml` and `twig.yml` like `when@<env>` blocks.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read twig paths of environment directories` with `Closes: lussoluca/phplumb#39`.

### Task 4: Template method and configuration tests (#40)

**Files:** fixture `vendor/symfony/framework-bundle/Controller/AbstractController.php`, `vendor/twig/...Environment.php`, controllers; tests in `tests/symfony.rs` and `tests/projects.rs`.

- [ ] **Step 1: Write the tests.** `render_block_and_stream_open_their_template` (`renderBlock`, `renderBlockView`, `stream`); `environment_display_and_load_open_their_template`; `list_and_relative_twig_paths_are_read`; `default_path_overrides_templates`; `twig_files_of_a_nested_drupal_project_get_no_symfony_answers` (on `symfony_with_drupal()`).
- [ ] **Step 2: Run them.** Expected: they pass. Remove each method from `TEMPLATE_METHODS`, the sequence branch of `read_paths`, the relative branch of `resolve`, the `default_path` read and the project gate in turn, and watch its test fail; a test that passes without its behavior is rewritten.
- [ ] **Step 3: Commit** `test(symfony): cover template methods and twig paths` with `Closes: lussoluca/phplumb#40`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0.
- [ ] **Step 2:** Record `2026-10-29, batch F1 template names` in `docs/benchmarks.md`, format it, commit `docs: record batch f1 results`.
