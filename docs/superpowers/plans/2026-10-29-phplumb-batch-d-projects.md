# PHPlumb batch D: framework projects implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the "To fix" issues about framework projects: modules symlinked into a nested Drupal project, annotations in files the index does not hold, the server log naming each module's folder, and tests for project gating.

**Architecture:** A framework project also covers the directories that symlinks inside it lead to, found by the discovery walk at build time; the outer project leaves them out. Annotation lookups ask the modules of the file being read, not of an indexed class. The server log names each module with its folder relative to the root.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6a, project scoping) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md`. The issues are the requirements for each task.

**Issues:** lussoluca/phplumb #22, #23, #27 and #28.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No lost results.** Coverage, cases and diagnostics on both reference projects match `main`.
- **Branch** `fix/22-framework-projects` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A symlink inside the Drupal project that leads into the Symfony project's own code** gives the linked files to Drupal only, never to both modules. Test in Task 1.
- **A symlink loop or a link to a directory outside the root** changes nothing. Test in Task 1.
- **An excluded Drupal file** gets annotation navigation, but a file outside the Drupal project does not. Test in Task 2.
- **A module whose project is the root** is logged by name alone. Test in Task 3.

---

### Task 1: Symlinked modules belong to the project that links them (#22)

**Files:** Modify `crates/phplumb-index/src/discover.rs`, `crates/phplumb-index/src/frameworks/project.rs`, `crates/phplumb-index/src/filter.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/src/frameworks/drupal/services.rs`; `crates/phplumb-index/tests/projects.rs`.

**Interfaces:** Produces `Walked::links: Vec<(PathBuf, PathBuf)>` (each symlinked directory and its canonical target); `Project::linked: Vec<PathBuf>`, the canonical targets of symlinks inside the project that lie outside it, which `Project::contains` accepts and the other projects exclude; `PathFilter::starts(&self) -> Vec<&Path>`, the project directory and its linked directories, which the Drupal registries read files under.

- [ ] **Step 1: Write the failing tests.** In `tests/projects.rs`, `symlinked_modules_belong_to_the_linking_project`: a workspace with a Symfony lock at the root, a Drupal lock in `drupal/`, a module in `packages/foo` (`foo.info.yml`, `foo.services.yml` defining `foo.svc` with class `Foo\Svc`, `foo.module` with `foo_cron()`, `src/Svc.php`) and the link `drupal/web/modules/custom/foo -> ../../../../packages/foo`: Drupal defines `foo.svc` and lists `foo_cron` for `cron`, and `Symfony::project()` excludes `packages/foo`; `links_outside_the_root_change_nothing`: a link to a directory outside the root and a link loop leave both projects unchanged.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the links in the walk, the linked directories and exclusions after the walk in `Index::build_with`, `Project::contains`, and the registries reading under `starts()`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(frameworks): count symlinked modules in the linking project` with `Closes: lussoluca/phplumb#22`.

### Task 2: Annotations in any file of the Drupal project (#23)

**Files:** Modify `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/src/references.rs`; `crates/phplumb-index/tests/frameworks.rs`.

**Interfaces:** Produces `Index::annotation_classes(&self, path: &Path, summary: &FileSummary, name: &NameRef) -> Vec<String>`, asking the modules whose project holds `path`.

- [ ] **Step 1: Write the failing tests.** In `tests/frameworks.rs`, `annotations_navigate_in_unindexed_files`: in the Drupal fixture, a block plugin source given to `definition` under a new path inside the project that the index has not read leads `@Block` to the `Block` annotation class; `annotations_navigate_in_excluded_files`: the same with a build whose `exclude` covers the plugin's file. A file outside the Drupal project in the `projects` fixture gets nothing.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the path-based gate.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(drupal): navigate annotations in any file of the project` with `Closes: lussoluca/phplumb#23`.

### Task 3: The server log names module projects (#27)

**Files:** Modify `crates/phplumb-index/src/index.rs`, `crates/phplumb-server/src/lsp.rs`; `crates/phplumb-server/tests/lsp_drupal.rs` or `lsp_root.rs`.

**Interfaces:** Produces `Index::framework_projects(&self) -> Vec<(&'static str, PathBuf)>`; the indexing log line ends with `; frameworks: drupal (codebase/drupal), symfony`, a module whose project is the root shown by name alone.

- [ ] **Step 1: Write the failing test.** An LSP test on a workspace with a Drupal lock in `codebase/drupal` and a Symfony lock at the root reads the `window/logMessage` and finds `frameworks: drupal (codebase/drupal), symfony`.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** the projects and the log text.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `feat(lsp): log the folder each framework module reads` with `Closes: lussoluca/phplumb#27`.

### Task 4: Tests for project gating (#28)

**Files:** `crates/phplumb-index/tests/projects.rs`.

- [ ] **Step 1: Write the tests.** In the `projects` fixture, each expecting no Drupal answer outside the Drupal project: a Drupal-shaped service id in the Symfony `config/services.yaml` (YAML file targets); a Symfony function named like a hook implementation, `demo_cron()` (declaration targets); `'#type' => 'demo_element'` in a Symfony file (keyed strings); and an update of a `*.services.yml` file outside the Drupal project, which leaves Drupal's services unchanged (the update filter).
- [ ] **Step 2: Run them.** Expected: they pass; any that fails is a bug to fix with the test kept.
- [ ] **Step 3: Commit** `test(frameworks): cover project gating` with `Closes: lussoluca/phplumb#28`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run both reference benchmarks back to back with coverage, diagnostics and cases, and the per-access dump comparison. Expected: unchanged.
- [ ] **Step 2:** Record a short section `2026-10-29, batch D framework projects` in `docs/benchmarks.md`, format it, commit `docs: record batch d results`.
