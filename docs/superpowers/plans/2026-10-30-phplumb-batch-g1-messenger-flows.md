# PHPlumb batch G1: Messenger flows implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Follow more Messenger flows: messages wrapped in an `Envelope`, messages passed by name, `HandleTrait::handle()`, handler methods inherited or taken from a trait, interface-typed messages, and a bare class-level `#[AsMessageHandler]`; add the dispatch tests issue #46 asks for.

**Architecture:** Calls record, for each argument that is `new C(x)` or `C::f(x)`, the type of `x`, so the Symfony module reads the message inside an `Envelope`. Argument types are placed by parameter like the strings of batch E2. Handlers are read from every method reflection lists for a concrete class, own, from traits and from parents. A message typed by an interface or an abstract class also reaches the handlers of its subtypes, both ways. The name of a bare attribute counts as the attribute for framework declarations.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6e Messenger) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #41 (part), #43, #46 and #47. Batch G2 takes the rest of #41 (Scheduler, YAML and tag handlers, wildcards), #42 and #45.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No wrong jumps.** A handler is listed only when its message class can be the dispatched one: a supertype of it, or, for an interface or abstract class, a subtype.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/41-messenger-flows` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **An `Envelope` held in a variable** (`$e = new Envelope($m); $bus->dispatch($e)`) lists no handler of `Envelope` itself. Test in Task 1.
- **A concrete message class** does not reach the handlers of its subclasses. Test in Task 3.
- **A handler method of an abstract parent** counts for each concrete subclass, never for the parent itself. Test in Task 2.
- **A bare attribute other than `#[AsMessageHandler]`** still opens its class. Test in Task 4.
- **A `handle()` call on a class without `HandleTrait`** lists nothing. Test in Task 1.

---

### Task 1: Envelopes, named messages and `HandleTrait` (#41)

**Files:** Modify `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `crates/phplumb-index/src/frameworks/drupal/mod.rs`, `crates/phplumb-index/src/index.rs`; fixtures and tests in `tests/symfony.rs`.

**Interfaces:** `FoundCall::wrapped` and `CallAt::wrapped: Vec<Option<TypeExpr>>`, by parameter position: the type of the first argument of an argument written `new C(x, ...)` or `C::f(x, ...)`; `Framework::call_targets(index, call, arguments, wrapped)`; argument types placed by parameter position; `DISPATCH` gains `Symfony\Component\Messenger\HandleTrait::handle`.

- [ ] **Step 1: Write the failing tests.** Fixture lines `dispatch(new Envelope(new SendEmail()))` (already there), `dispatch(Envelope::wrap(new SendSms()))`, `dispatch(message: new SendEmail())`, a `$this->handle(new SendSms())` in a class using `HandleTrait`, and a `handle()` on a class without it. Tests: `envelopes_reach_the_handlers_of_their_message`, `named_messages_reach_their_handlers`, `handle_trait_calls_reach_their_handlers` (both directions), `handle_without_the_trait_lists_nothing`, `envelope_variables_list_nothing`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): follow messages in envelopes, by name and through handle()` with `Refs: lussoluca/phplumb#41`.

### Task 2: Inherited and trait handler methods (#41)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/messenger.rs`, `routes.rs` (shared `methods_of`); fixtures and tests in `tests/symfony.rs`.

- [ ] **Step 1: Write the failing tests.** Fixture: `abstract class AbstractSmsHandler { #[AsMessageHandler] public function onSms(SendSms $m) }` with a concrete `ChildSmsHandler`; a trait with `__invoke(SendEmail $m)` used by a class with a class-level `#[AsMessageHandler]`. Tests: `inherited_handler_methods_handle_for_concrete_classes` (the dispatch lists `AbstractSmsHandler::onSms` once, and the method lists the dispatch), `trait_handler_methods_handle_for_their_class`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** reading every method of a concrete class, own first, with the method's file, and the classes whose ancestors declare handler methods.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read inherited and trait handler methods` with `Refs: lussoluca/phplumb#41`.

### Task 3: Interface-typed messages (#43)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/messenger.rs`, `mod.rs`; tests in `tests/symfony.rs`.

- [ ] **Step 1: Write the failing tests.** Fixture line `dispatch($notify)` with `Notify $notify`. Tests: `interface_typed_dispatches_reach_the_handlers_of_implementations` (lists the `SendEmail` and `SendSms` handlers and `NotifyLogger::handle`), `handlers_of_implementations_list_interface_typed_dispatches`, `concrete_messages_skip_handlers_of_subclasses`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** in `Handlers::of` and `dispatches`.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): match interface-typed messages with their handlers` with `Closes: lussoluca/phplumb#43`.

### Task 4: Bare handler attributes and dispatch tests (#47, #46)

**Files:** Modify `crates/phplumb-index/src/index.rs`; fixtures and tests in `tests/symfony.rs`.

- [ ] **Step 1: Write the tests.** `bare_class_handler_attributes_list_their_dispatches` (definition on `AsMessageHandler` of `SendEmailHandler`'s bare attribute); `bare_attributes_of_other_classes_open_their_class`; `dispatch_on_other_objects_lists_nothing`; `multi_line_and_null_safe_dispatches_list_their_handlers`.
- [ ] **Step 2: Run them.** Expected: the bare-attribute test fails; the #46 tests pass, and each fails when `calls_one_of` or null-safe calls are removed from the reading.
- [ ] **Step 3: Implement** in `framework_declaration`: on the name of an attribute with no arguments, the attribute counts as the declaration; the frameworks' answer wins only when not empty.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): list dispatches from a bare handler attribute` with `Closes: lussoluca/phplumb#47` and `Closes: lussoluca/phplumb#46`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target.
- [ ] **Step 2:** Record `2026-10-30, batch G1 Messenger flows` in `docs/benchmarks.md`, format it, commit `docs: record batch g1 results`.
