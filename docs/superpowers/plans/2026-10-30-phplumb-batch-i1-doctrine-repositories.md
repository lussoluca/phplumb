# PHPlumb batch I1: Doctrine repositories and mapping attributes implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the repository declarations issue #53 lists, navigate the XML mapping attributes of #54, drop removed mapping files at once (#55), and add the tests of #56.

**Architecture:** Repositories are recorded for entities and mapped superclasses, from XML, attributes (named or positional) and `@ORM\Entity` annotations; an entity without its own takes the nearest mapped superclass's. The XML name reader learns `discriminator-mapping`, `entity-listener`, `custom-id-generator` and `order-by-field`. A removed path that holds listed mapping files refreshes the list.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6f Doctrine) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #53, #54, #55 and #56. Batch I2 takes #52.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No wrong jumps.** A repository the declaration names but the index lacks keeps `getRepository()` with its declared type, as today.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/53-doctrine-repositories` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **An entity with its own repository** keeps it over its mapped superclass's. Test in Task 1.
- **A plain parent class** (not a mapped superclass) gives no repository. Test in Task 1.
- **`order-by-field` outside an association** gives nothing. Test in Task 2.
- **A removed directory without mapping files** changes nothing. Test in Task 3.
- **An annotation in a comment that is not a docblock** gives nothing. Test in Task 1.

---

### Task 1: Repository declarations (#53)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/doctrine.rs`, `mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `Mappings::repository_of(&self, index, entity) -> Option<&str>`: the entity's own repository, else the nearest mapped superclass ancestor's.

- [ ] **Step 1: Write the failing tests.** `mapped_superclass_repositories_reach_their_entities` (XML and attribute forms; the entity's own wins; a plain parent gives none); `positional_entity_repositories_are_read`; `annotation_repositories_are_read`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(doctrine): read more repository declarations` with `Closes: lussoluca/phplumb#53`.

### Task 2: More mapping attributes navigate (#54)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/doctrine.rs`; tests in `tests/symfony.rs` or `doctrine.rs`.

- [ ] **Step 1: Write the failing tests.** `named_at` gives the class for `discriminator-mapping class`, `entity-listener class` and `custom-id-generator class` (qualified like `target-entity`), and the target entity's property for `order-by-field name` inside an association; nothing outside one.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(doctrine): navigate more mapping attributes` with `Closes: lussoluca/phplumb#54`.

### Task 3: Removed mapping files and tests (#55, #56)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `Symfony::mapping_files(&self) -> &[PathBuf]`.

- [ ] **Step 1: Write the tests.** `removed_dotted_mapping_directories_leave_the_list` (fails today); `added_and_removed_mapping_files_update_repositories`; `mapping_files_with_a_bom_or_encoding_are_read`; `manager_registry_repositories_are_typed`.
- [ ] **Step 2: Run them.** Expected: the first fails, the others pass and each fails when its behavior is removed.
- [ ] **Step 3: Implement** refreshing the list when a removed path holds a listed mapping file.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(doctrine): drop removed mapping directories at once` with `Closes: lussoluca/phplumb#55` and `Closes: lussoluca/phplumb#56`.

### Task 4: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target.
- [ ] **Step 2:** Record `2026-10-30, batch I1 Doctrine repositories` in `docs/benchmarks.md`, format it, commit `docs: record batch i1 results`.
