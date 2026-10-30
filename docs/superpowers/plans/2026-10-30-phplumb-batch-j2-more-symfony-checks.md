# PHPlumb batch J2: more Symfony names checked implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Check more Symfony names where the module can prove them missing: routes from XML route files, Symfony's logout routes, interpolated and listed Twig names, Twig files of project bundles, calls outside the `try` blocks that catch the error, and parameters of bundle families once every bundle source is read.

**Architecture:** An XML route reader joins the YAML and PHP ones, so XML files stop leaving routes unknown, in the project and in bundles. The logout import leaves `_` names unknown only when security configuration may be unread. Twig names with `#{}` become patterns matched against route names and template files; a list of templates is flagged when every element is missing. A call is left unchecked by an exception class only inside a `try` that catches it. Parameter names outside the project's families are checked once every bundle class, its extension code and the compiler passes it adds are read without a name computed in full.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #57 and #58. Open follow-ups read before planning: #93, #94 and #97 to #101 stay open.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A name is flagged only when every source that may define it was read.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`; Symfony diagnostics on the reference are inspected one by one.
- **Branch** `fix/57-more-symfony-checks` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **An XML route file with an element the reader does not know** (a glob import, an unknown `type`) leaves routes unknown. Test in Task 1.
- **A list of templates with one existing element** flags nothing. Test in Task 3.
- **`ignore missing`** never flags its template, since Twig skips it by design. Test in Task 3.
- **A call inside a `try` catching `\Throwable`** stays unchecked. Test in Task 4.
- **A bundle class the index lacks, or a bundle with YAML configuration,** keeps bundle-family parameters unchecked. Test in Task 5.

---

### Task 1: XML route files (#57, #58)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `xml_route_files_define_routes`: `config/routes/blog.xml` with `<route id="blog_list" path="/blog" controller="App\Controller\Blog::list"/>`, a route with `<default key="_controller">`, one inside `<when env="dev">`, an alias `<route id="old" alias="blog_list"/>` and `<import resource="admin.xml" name-prefix="admin_"/>` define `blog_list`, `old` and `admin_x`, each leading to its `id`; `missing_route` is flagged. `xml_bundle_route_imports_are_read`: an `@FrameworkBundle/Resources/config/routing/errors.xml` import defines `_preview_error`, and `_nowhere` is flagged. `unreadable_xml_routes_stay_unknown`: a glob import or an unknown element leaves `missing_route` unflagged.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** `read_xml` with the YAML reader's import rules.
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): read xml route files` with `Refs: lussoluca/phplumb#57`.

### Task 2: Logout routes (#58)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`, `mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing test.** `logout_routes_are_checked_when_security_is_read`: with the logout loader imported and a firewall `main` with `logout: { path: /logout }`, `_logout_main` leads to the firewall and `_logout_admin` is flagged; with a `config/packages/security.php` beside it, nothing is flagged.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): check logout route names` with `Refs: lussoluca/phplumb#58`.

### Task 3: Twig names (#58)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/twig.rs`, `broken.rs`, `templates.rs`, `routes.rs`; tests in `twig.rs` and `tests/projects.rs`.

**Interfaces:** `TwigName::Pattern(kind)` for a name with `#{}`, matched as its literal parts in order; `TwigName::Choice(group)` for an element of a template list; `Routes::matches(prefix, suffix) -> bool`; `Templates::matches(pattern) -> TemplateStatus`.

- [ ] **Step 1: Write the failing tests.** `interpolated_twig_names_are_checked`: `path("blog_#{x}")` with a route `blog_list` flags nothing, `path("shop_#{x}")` is flagged; `include("blocks/#{t}.html.twig")` with `templates/blocks/a.html.twig` flags nothing, `include("cards/#{t}.html.twig")` is flagged. `template_lists_are_checked`: `include(['a.html.twig', 'b.html.twig'])` with neither existing flags both, with `b` existing flags nothing; `{% include 'x.html.twig' ignore missing %}` flags nothing. `project_bundle_templates_are_checked`: a Twig file in `src/AdminBundle/templates` of a registered project bundle flags a missing template; one under `tests/` does not.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): check interpolated and listed twig names` with `Refs: lussoluca/phplumb#58`.

### Task 4: Exceptions silence their try blocks only (#58)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/broken.rs`, `crates/phplumb-index/src/infer.rs`; tests in `tests/projects.rs`.

**Interfaces:** `infer::caught_ranges(source) -> Vec<(u32, u32, Vec<String>)>`, each `try` body with the resolved classes its `catch` clauses name.

- [ ] **Step 1: Write the failing tests.** `exceptions_silence_only_their_try_blocks`: in a file that mentions `RouteNotFoundException`, a `generateUrl('missing_route')` outside any `try` is flagged, one inside `try { } catch (RouteNotFoundException $e)` is not, one inside `catch (\Throwable $e)` is not; the same for `LoaderError` with `render()` and `ParameterNotFoundException` with `getParameter()`. A file constructing `UrlGenerator` or `FilesystemLoader` stays unchecked.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement**: a caught class counts when it is the exception or one of its ancestors.
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): leave only try blocks unchecked` with `Closes: lussoluca/phplumb#58`.

### Task 5: Parameters of bundle families (#57)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `definitions.rs`, `crates/phplumb-index/src/infer.rs`; tests in `tests/projects.rs`.

**Interfaces:** `ExtensionParameters::complete: bool`: every class `bundles.php` names is indexed, its extension code and the compiler passes its class instantiates were read without a name computed in full, and no bundle holds YAML configuration; `parameter_missing` drops the family rule when `complete`; names starting with `container.` or `.` are never flagged, as the kernel sets them.

- [ ] **Step 1: Write the failing tests.** `bundle_parameters_are_checked_when_every_source_is_read`: `%router.nowhere%` with no project `router.*` parameter is flagged; a parameter a compiler pass the bundle class adds sets is not; with a bundle class missing from the index, or a bundle `Resources/config/services.yaml`, it is not flagged; `%container.build_id%` is never flagged.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): check parameters of bundle families` with `Closes: lussoluca/phplumb#57`.

### Task 6: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; every new Symfony diagnostic on the reference is a name that is truly missing.
- [ ] **Step 2:** Record `2026-10-30, batch J2 more Symfony checks` in `docs/benchmarks.md`, format it, commit `docs: record batch j2 results`.
