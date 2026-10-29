# PHPlumb batch A: performance implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the warm start and the update of 1,000 files back under their targets on the Drupal reference project, and remove the repeated or wasted work the "To fix" performance issues describe, with no change to any navigation result or diagnostic.

**Architecture:** Framework detection walks once per start. The discovery walk runs in parallel, canonicalizes only paths reached through a symlink, and also records the YAML, Twig and Doctrine XML files it meets, so the framework modules stop walking the tree themselves. Registry updates test changed paths through a set instead of comparing every pair. Stubs keep no reference tables, are never stale, and a failed stub build can be retried. Framework lookups run only for modules that apply to the file and answer for the method name, and Symfony diagnostics parse each file once and re-read YAML only when YAML changes.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-26-php-navigator-design.md`, "Success targets" (warm start under 2 s, a 1,000-file change under 5 s, memory under 1 GB) and anti-goals "Wrong jumps" and "False alarms". The issues are the requirements for each task.

**Issues:** lussoluca/phplumb #1, #2, #3, #6, #24, #25, #26, #44 and #60. Profiling on the Drupal reference project (41,335 PHP files) before this plan: warm start 5.4 s, of which the discovery walk takes 1.6 s (0.6 s of it in `canonicalize`) and the Drupal rebuild 3.6 s (services and routes walks 2.3 s, hooks 0.8 s, literals 0.25 s); the update of 1,000 files takes 7.1 s, almost all of it in the Drupal plugins update (about 5 s) and literals update (1.9 s), both comparing every changed path with every known file. A parallel walk measured 0.7 s.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No result changes.** Member coverage (Drupal 42,929, Symfony 13,097), cases (Drupal 87 passed and 7 failed, Symfony 31 passed) and diagnostics (Drupal 94, Symfony 0) stay exactly as on `main`.
- **Targets** (spec): warm start from the cache under 2 s, update of 1,000 files under 5 s, update of one file under 0.5 s, peak resident memory under 1 GB, on the Drupal reference project.
- **Platforms:** host code runs on macOS and Linux. Tests that need symlinks or permissions are `#[cfg(unix)]`, and a permission test returns early when it runs as root.
- **Branch** `fix/1-indexing-performance` from `main`. Conventional commits; each commit carries `Closes: lussoluca/phplumb#N` for every issue it fixes (`Refs:` when it fixes part of one) and `Assisted-by: claude-code/claude-opus-5-5`. No client name or path in commits or docs.

## Review focus

- **A module symlinked into the project from inside or outside the root** (Composer path repositories) keeps one entry under the same chosen path as before, for PHP and YAML files alike. Test in Task 2.
- **A YAML file created in a new directory after start,** as a branch switch adds a module, gives its services, routes and hooks at once. Test in Task 3.
- **A deleted services or routing file** drops its definitions at once. Test in Task 3.
- **A services file matched by `phplumb.exclude`** is not read. Test in Task 3.
- **Both modules set to `off`** gives no framework, only the `php` extension, and no lock walk. Test in Task 1.

---

### Task 1: One framework detection per start (#25)

**Files:**

- Modify: `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/frameworks/project.rs`, `crates/phplumb-index/src/filter.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-server/src/lsp.rs`, `crates/phplumb-index/tests/frameworks.rs`

**Interfaces:**

- Produces: `pub struct Detection` in `frameworks/mod.rs`, from `pub fn detect(root: &Path, settings: &FrameworkSettings) -> Detection`, which walks for `composer.lock` files once and only when a module is not `Off`, and parses each lock at most once. `Detection::php_extensions(&self) -> Vec<&'static str>`; `pub(crate) fn Detection::frameworks(&self) -> Vec<Box<dyn Framework>>` (fresh modules with their projects); `pub(crate) walked: bool`.
- Produces: `Index::build_with(root: &Path, opts: &DiscoverOptions, cache: Option<&Path>, detection: &Detection) -> anyhow::Result<(Index, IndexStats)>`; `build` and `build_with_cache` detect once and call it. `frameworks::php_extensions` and `PathFilter::new` use `detect`.

- [ ] **Step 1: Write the failing tests.** In `frameworks/mod.rs` unit tests: `detection_skips_the_walk_when_every_module_is_off` (`detect` with both `Off` on a root holding a Drupal lock gives `walked == false` and `php_extensions() == ["php"]`); `detection_reads_each_lock_once_for_both_modules` (a root whose one `composer.lock` lists both `drupal/core` and `symfony/framework-bundle` gives both modules with that directory). In `tests/frameworks.rs`, `off_modules_give_only_php`: both `Off`, `active_frameworks()` empty.
- [ ] **Step 2: Run them and watch them fail.** Run: `cargo test -p phplumb-index detection`. Expected: compile error, `detect` not found.
- [ ] **Step 3: Implement** `detect`, `Detection` and `Index::build_with`; the LSP server detects once at start, announces extensions from that detection, and its first build uses it; a later rebuild detects again.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(frameworks): detect framework projects once per start` with `Closes: lussoluca/phplumb#25`.

### Task 2: Parallel discovery that records framework files (#1)

**Files:**

- Modify: `crates/phplumb-index/src/discover.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/tests/discover.rs`, `crates/phplumb-index/tests/update.rs`

**Interfaces:**

- Produces: `pub(crate) struct Walked { pub php: Vec<PathBuf>, pub framework: Vec<PathBuf> }` and `pub(crate) fn walk(filter: &mut PathFilter, start: &Path) -> Walked`, a parallel walk (`ignore` `build_parallel`) with the current rules; only a path that is a symlink or lies below a symlinked directory is canonicalized, and the dedup rule (one entry per canonical file, the smallest chosen path) is unchanged. `framework` holds the YAML, Twig and Doctrine XML files (`is_framework_file`) outside `vendor`, hidden directories, `node_modules` and excluded paths, sorted. `discover_in` returns `walk(..).php`.
- Produces: `pub fn Index::framework_files_under<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = &'a Path> + 'a`, sorted, kept in a `BTreeSet<PathBuf>` that `Index::update` refreshes before the frameworks update: a changed directory is walked again, a changed framework file is added when it exists and qualifies, a removed path drops every entry below it.

- [ ] **Step 1: Write the failing tests.** In `tests/discover.rs`, `dedupes_symlinks_from_outside_the_root`: a module directory outside the root symlinked into `web/modules/custom/foo` is listed once under the link path, and the existing `dedupes_symlinked_dirs` keeps passing. In `tests/update.rs`, `framework_files_follow_updates`: a project with `a/a.services.yml`, `vendor/p/p.services.yml` and `.hidden/h.yml` lists only `a/a.services.yml` under the root; after creating `b/b.routing.yml` and `update(&[b], &[])` it lists both; after deleting `a/a.services.yml` and `update(&[], &[a/a.services.yml])` only `b/b.routing.yml`.
- [ ] **Step 2: Run them and watch them fail.** Run: `cargo test -p phplumb-index --test discover --test update`. Expected: `framework_files_follow_updates` fails to compile.
- [ ] **Step 3: Implement** `walk`, the index's framework file set and its update.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(index): walk the workspace in parallel and record framework files` with `Refs: lussoluca/phplumb#1`.

### Task 3: Drupal reads the recorded files and updates in linear time (#1, #24)

**Files:**

- Modify: `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/frameworks/drupal/{mod,services,routes,extensions,hooks,plugins,literals}.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/tests/frameworks.rs`, `crates/phplumb-index/tests/projects.rs`

**Interfaces:**

- Consumes: `Index::framework_files_under` (Task 2).
- Produces: `drupal_files(index: &Index, filter: &PathFilter, start: &Path, suffix: &str, tests: Tests) -> Vec<PathBuf>`, the recorded files below `start` that `is_drupal_file` accepts, with no walk; every Drupal registry read and update uses it.
- Produces: `pub(crate) fn under_any(path: &Path, dirs: &HashSet<PathBuf>) -> bool` in `frameworks/mod.rs` (the path or one of its ancestors is in `dirs`); the services, routes, plugins and literals updates test known files against the changed and removed paths with it, once per known file.
- Produces: `Index::calls_named(&self, names: &[&str], project: &Project) -> Vec<FoundCall>`, reading only candidate files inside `project`; Drupal's hook invocation search and Symfony's dispatch search pass their own project.

- [ ] **Step 1: Write the failing tests.** In `tests/frameworks.rs`: `drupal_registries_follow_a_new_module_directory` (after start, create `web/modules/custom/late/` with `late.info.yml`, `late.services.yml`, `late.routing.yml` and `late.module` implementing `late_cron()`, then `update(&[late dir], &[])`: the service id, the route name and the hook implementation resolve); `drupal_registries_drop_deleted_files` (delete `late.services.yml` and update it as removed: the service id gives nothing); `excluded_services_files_are_not_read` (a services file under an `exclude` glob defines nothing). In `tests/projects.rs`, `hook_invocations_stay_in_the_drupal_project`: in the `projects` fixture, an `invokeAll('demo_hook')` written in the Symfony project is not listed among the invocations of a Drupal `demo_hook` implementation.
- [ ] **Step 2: Run them and watch them fail.** Run: `cargo test -p phplumb-index --test frameworks --test projects`. Expected: the tests that need the new behavior fail; any that pass already stay as guards.
- [ ] **Step 3: Implement** `drupal_files` on the recorded files, `under_any` in the four updates, and the project-scoped `calls_named`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(drupal): read recorded files and update registries in linear time` with `Refs: lussoluca/phplumb#1` and `Closes: lussoluca/phplumb#24`.

### Task 4: Lighter, sturdier stubs (#2, #3, #6)

**Files:**

- Modify: `crates/phplumb-index/src/stubs.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-index/tests/stubs.rs`

**Interfaces:**

- Produces: `stubs::load(dir) -> Option<Arc<Index>>` remembers only a built index with declarations; a failed or empty build is tried again on the next call. `Index::build_stubs` skips an unreadable subdirectory (its files only) and fails only when `dir` itself is unreadable. Stub files are inserted with no `ref_keys`, so the stub index has no candidate table.
- Produces: `fn Index::stale_paths(&self, found: &[(Location, String)]) -> Vec<PathBuf>`, the sorted, deduplicated non-stub paths whose name no longer sits at the range; `definition_with` and `implementations` use it.

- [ ] **Step 1: Write the failing tests.** In `tests/stubs.rs`: `unreadable_stub_directories_skip_only_their_files` (a temporary stub directory with `a.php` declaring class `StubA` and `locked/b.php`, `locked` set to mode `000`: `new \StubA()` in a project file resolves to `a.php`); `an_empty_stub_directory_is_read_again` (an empty temporary stub directory, one build, then `x.php` declaring `StubX` is written and a second build resolves `\StubX`). In `index.rs` unit tests: `stub_locations_are_never_stale` (with the default stubs, `stale_paths` of the stub location of constant `pcov\all` with the name `all` is empty) and `stub_index_has_no_candidates`.
- [ ] **Step 2: Run them and watch them fail.** Run: `cargo test -p phplumb-index --test stubs && cargo test -p phplumb-index --lib stub`. Expected: the four new tests fail.
- [ ] **Step 3: Implement** the retry, the per-directory tolerance, the empty `ref_keys` and `stale_paths`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(stubs): drop unused tables, never treat stubs as stale, retry failed builds` with `Closes: lussoluca/phplumb#2`, `#3` and `#6` (one trailer each).

### Task 5: Framework lookups only where a module answers (#26, #44)

**Files:**

- Modify: `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/frameworks/drupal/mod.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-server/src/bench.rs`, `crates/phplumb-server/tests/bench.rs`

**Interfaces:**

- Produces: `Framework::typed_methods(&self) -> &'static [&'static str]` and `Framework::targeted_methods(&self) -> &'static [&'static str]`, the lowercased method names whose calls `call_type` and `call_targets` may answer. Symfony: `["getrepository"]` and `["dispatch"]`; Drupal: every method its `call_type` reads (service, storage and entity loading calls) and none.
- Produces: the inference context computes the modules applying to its file once; `framework_type` and the union split in `method_call` run only when an applying module lists the method name. `Index::framework_call` returns before `infer::call_at` unless the name under the cursor is in an applying module's `targeted_methods`; `framework_strings` and `framework_keyed_strings` check `frameworks_for(path)` first.
- Produces: `Coverage` in the bench report gains `elapsed_ms`.

- [ ] **Step 1: Write the failing test.** In `crates/phplumb-server/tests/bench.rs`, `bench_reports_coverage_time`: a bench run with `--coverage` reports `coverage.elapsed_ms`. The existing Drupal service, storage and entity typing tests and the Symfony repository and dispatch tests guard the gates.
- [ ] **Step 2: Run it and watch it fail.** Run: `cargo test -p phplumb-server --test bench coverage_time`. Expected: FAIL, no `elapsed_ms`.
- [ ] **Step 3: Implement** the two lists, the per-file and per-name gates, and `elapsed_ms`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(frameworks): skip lookups no module answers for` with `Closes: lussoluca/phplumb#26` and `Closes: lussoluca/phplumb#44`.

### Task 6: Symfony diagnostics once per change (#60)

**Files:**

- Modify: `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/frameworks/symfony/{mod,broken}.rs`, `crates/phplumb-index/src/frameworks/drupal/{mod,broken}.rs`, `crates/phplumb-index/src/diagnostics.rs`, `crates/phplumb-index/src/index.rs`, `crates/phplumb-server/src/bench.rs`, `crates/phplumb-index/tests/symfony.rs`

**Interfaces:**

- Produces: `Framework::diagnostics(&self, index: &Index, path: &Path, source: &[u8], summary: Option<&FileSummary>)`, with the summary `Index::diagnostics` computed once for a PHP file and none for a framework file; `diagnostics::find` takes the same summary. `pub(crate) fn mentions_member(summary: &FileSummary, names: &[&str]) -> bool` in `frameworks/mod.rs`; the Symfony and Drupal diagnostics call `infer::calls_named` only when it is true for their method names.
- Produces: `Symfony::update` reads the YAML configuration, YAML routes and Twig paths again only when a YAML file or a directory changed; a PHP-only change reads again only what PHP declares (attribute routes, handlers, attribute mappings, the PHP configuration check and `setParameter()` calls).
- Produces: `bench --diagnostics <dir>` takes its YAML, Twig and XML files from `Index::framework_files_under(dir)`.

- [ ] **Step 1: Write the failing test.** In `tests/symfony.rs`, `php_updates_keep_the_yaml_read_before`: on a temporary copy of the Symfony fixture, rename parameter `app.name` to `app.renamed` in `config/services.yaml` on disk, then `update` only a PHP file: `getParameter('app.name')` still leads to its definition; then `update` the YAML file: `getParameter('app.renamed')` leads to it and `app.name` gives nothing.
- [ ] **Step 2: Run it and watch it fail.** Run: `cargo test -p phplumb-index --test symfony yaml_read_before`. Expected: FAIL on the first assertion.
- [ ] **Step 3: Implement** the shared summary, `mentions_member`, the split update and the bench change.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 5: Commit** `perf(symfony): parse once and read yaml only when yaml changes` with `Closes: lussoluca/phplumb#60`.

### Task 7: Benchmarks and issues

**Files:**

- Modify: `docs/benchmarks.md`

- [ ] **Step 1: Build the baseline** from `git merge-base main HEAD` in a worktree under the session scratchpad.
- [ ] **Step 2: Run both benchmarks,** baseline first, with `--cache`, `--coverage` and `--diagnostics` (Drupal `web/modules/custom`, Symfony `src`) and the case files. Expected: the global constraint's results unchanged; Drupal warm start under 2,000 ms and `update_1000_ms` under 5,000; peak memory lower than the baseline; Drupal `coverage.elapsed_ms` no higher than before step 6d (about 11.5 s for the whole run). Any miss is fixed with a test before going on.
- [ ] **Step 3: Record the results** in a section `2026-10-29, batch A performance` of `docs/benchmarks.md`, and run the Markdown formatter on it.
- [ ] **Step 4: Commit** `docs: record batch a performance results` with `Refs: lussoluca/phplumb#1`.
- [ ] **Step 5: Open issues** for anything this batch leaves unfixed, per `CLAUDE.md`.
