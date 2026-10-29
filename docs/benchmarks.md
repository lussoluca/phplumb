# Benchmarks

Results of `phplumb bench` on the reference Drupal project and, from step 5a, on the reference Symfony project. The case files live outside the repository because they point into project code.

## 2026-10-29, batch F1 template names

Same machine and checkouts, measured with this branch and with `main` back to back. The batch fixes issues #39 and #40. Every member access keeps its targets on both reference projects, and cases (Drupal 85 passed and 9 failed, Symfony 31 passed) and diagnostics (94 and 0) are unchanged. Template names stay certain on the Symfony reference, and the sweep of `src`, `templates` and `config` still finds no broken template name with the new forms read.

## 2026-10-29, batch E2 route references

Same machine and checkouts, measured with this branch and with `main` back to back. The batch fixes issues #33, #35 and #36. Every member access keeps its targets on both reference projects, and cases (Drupal 85 passed and 9 failed, Symfony 31 passed) and diagnostics (94 and 0) are unchanged.

The Symfony reference's bundle routes now lead to their definitions: `_preview_error`, `_wdt` and `_profiler` to the bundles' route files, and `_logout_main` to the firewall's `logout` setting. The 156 route names used in `src` and `templates` keep their targets, and route names stay certain.

## 2026-10-29, batch E1 route definitions

Same machine and checkouts, measured with this branch and with `main` back to back. The batch fixes issues #29, #30, #31, #32 and #34. Every member access keeps its targets on both reference projects, and cases (Drupal 85 passed and 9 failed, Symfony 31 passed) and diagnostics (94 and 0) are unchanged.

The two Symfony reference routes named by class constants, such as `name: self::ROUTE` on a typed constant, now lead to their controller methods from `path()` in Twig, where they gave nothing. With them read, the project's route names are certain, and the sweep of `src`, `templates` and `config` still finds no broken route name.

## 2026-10-29, batch D framework projects

Same machine and checkouts, measured with this branch and with `main` back to back. The batch fixes issues #22, #23, #27 and #28. Neither reference project links a module from outside its framework project, so every member access keeps the same targets on both builds, and cases (Drupal 85 passed and 9 failed, Symfony 31 passed) and diagnostics (94 and 0) are unchanged. The Drupal warm start measured 1.36 s (1.32 s on `main`) and the update of 1,000 files 0.22 s on both.

## 2026-10-29, batch C2 generic and element types

Same machine and checkouts, measured with this branch and with `main` back to back, twice each. The batch fixes issues #11, #15, #17, #18, #19 and #20.

| Metric                    | Drupal reference                  | Symfony reference                   |
| ------------------------- | --------------------------------- | ----------------------------------- |
| Member coverage           | 42,805 (42,731 on `main`)         | 13,147 of 13,211 (13,126 on `main`) |
| Warm start from the cache | 1.37 to 1.41 s (1.31 to 1.36 s)   | 0.19 s (0.20 to 0.22 s)             |
| Update of one file        | 95 to 96 ms (96 to 104 ms)        | 20 to 21 ms (21 to 27 ms)           |
| Update of 1,000 files     | 0.23 to 0.28 s (0.22 to 0.23 s)   | 82 to 92 ms (81 to 83 ms)           |
| Peak resident memory      | 677 to 684 MB (675 to 679 MB)     | 270 to 274 MB (257 to 269 MB)       |
| All cases                 | 85 passed, 9 failed, as on `main` | 31 passed, 0 failed, as on `main`   |
| Diagnostics               | 94, the same entries as on `main` | 0, as on `main`                     |

A per-access comparison with `main` on both projects finds no target lost or changed. The 74 Drupal gains include closures given to `Crawler::each()`, variables destructured from shapes, and conditional return types such as `access(..., TRUE)`; the coverage row was measured after the final review's fixes.

## 2026-10-29, batch C1 call types

Same machine and checkouts, measured with this branch and with `main` back to back, twice each. The batch fixes issues #12, #13, #14, #16 and #21.

| Metric                    | Drupal reference                  | Symfony reference                   |
| ------------------------- | --------------------------------- | ----------------------------------- |
| Member coverage           | 42,717 (42,705 on `main`)         | 13,126 of 13,211 (13,097 on `main`) |
| Warm start from the cache | 1.41 to 1.58 s (1.44 to 1.45 s)   | 0.24 to 0.45 s (0.27 to 0.29 s)     |
| Update of one file        | 102 to 108 ms (96 to 105 ms)      | 21 to 41 ms (33 to 36 ms)           |
| Update of 1,000 files     | 0.23 to 0.25 s (0.22 to 0.23 s)   | 85 to 169 ms (194 to 227 ms)        |
| Peak resident memory      | 676 to 686 MB (643 to 687 MB)     | 258 to 265 MB (267 to 281 MB)       |
| All cases                 | 85 passed, 9 failed, as on `main` | 31 passed, 0 failed                 |
| Diagnostics               | 94, as on `main`                  | 0, as on `main`                     |

The Symfony case for a parameter documented only on the interface now expects the method of the interface's element type, `Id::toString()`, where it expected nothing. Times vary from run to run on both builds.

## 2026-10-29, batch B stubs and enums

Same machine. The Drupal reference project is now on another feature branch, so both projects were measured with this branch and with `main` on the same checkouts, alternating, twice each. The batch fixes issues #4, #5, #7, #8, #9 and #10.

| Metric                    | Drupal reference                  | Symfony reference                 |
| ------------------------- | --------------------------------- | --------------------------------- |
| Member coverage           | 43,008 (42,929 on `main`)         | 13,097, the same as `main`        |
| Cold index                | 2.4 s (2.6 to 2.7 s on `main`)    | 0.5 to 0.6 s (0.5 to 0.9 s)       |
| Warm start from the cache | 1.35 to 1.47 s (1.26 to 1.40 s)   | 0.19 s, the same as `main`        |
| Update of one file        | 98 to 107 ms (96 to 115 ms)       | 21 to 22 ms (19 to 20 ms)         |
| Update of 1,000 files     | 0.23 to 0.29 s (0.23 to 0.24 s)   | 87 to 90 ms (77 to 80 ms)         |
| Peak resident memory      | 679 to 729 MB (632 to 715 MB)     | 260 to 276 MB (251 MB)            |
| All cases                 | 85 passed, 9 failed, as on `main` | 31 passed, 0 failed, as on `main` |
| Diagnostics               | 94, the same entries as on `main` | 0, as on `main`                   |

The 79 new accesses on the Drupal reference project are `self::` and `static::` accesses inside traits, such as `self::assertSame()` in test traits, which now reach the members of the classes using the trait. The project changed between runs, and every state gave the same 79 more than `main`. Times and memory stay within the run-to-run noise over five alternated runs.

## 2026-10-29, batch A performance

Same machine. The Drupal reference project is now on its `develop` branch, so both projects were measured with this branch and with `main` on the same checkouts, alternating, twice each. The batch fixes issues #1, #2, #3, #6, #24, #25, #26, #44 and #60.

| Metric                    | Drupal reference                      | Symfony reference                 |
| ------------------------- | ------------------------------------- | --------------------------------- |
| Member coverage           | 42,929, the same as `main`            | 13,097, the same as `main`        |
| Cold index                | 2.3 to 2.7 s (6.1 to 7.0 s on `main`) | 0.5 s (0.8 to 1.2 s on `main`)    |
| Warm start from the cache | 1.4 to 1.5 s (4.8 to 5.1 s)           | 0.19 s (0.43 to 0.46 s)           |
| Update of one file        | 100 ms (96 to 99 ms)                  | 18 ms (25 ms)                     |
| Update of 1,000 files     | 0.23 to 0.25 s (7.0 to 7.2 s)         | 76 ms (64 to 83 ms)               |
| Peak resident memory      | 653 to 702 MB (576 to 589 MB)         | 252 to 262 MB (245 to 251 MB)     |
| All cases                 | 85 passed, 9 failed, as on `main`     | 31 passed, 0 failed, as on `main` |
| Diagnostics               | 94, the same entries as on `main`     | 0, as on `main`                   |

The warm start and the update of 1,000 files meet their targets again, for the first time since step 2b. The Drupal module walked its project three more times for YAML files and canonicalized every file it met; it now reads the YAML files the discovery walk records. Its updates compared every changed path with every known file, which cost 7 s for 1,000 files. Discovery itself runs in parallel and canonicalizes only paths reached through a symlink.

Peak memory is about 70 MB higher on the Drupal reference project since the parallel walk, still under the 1 GB target; issue #74 tracks it. The coverage sweep of `web/modules/custom` now reports its time, 10.2 to 10.5 s. The diagnostics sweep of the same directory takes 4.8 s for the whole run, against 10.8 s on `main`.

## 2026-10-28, step 6g broken Symfony references

Same machine and checkouts as step 6f. Route, template and parameter names defined nowhere are flagged in PHP, Twig and YAML, and the diagnostics sweep now also checks the YAML, Twig and Doctrine XML files below its directory. The build before this step ran first.

| Metric               | Drupal reference                | Symfony reference          |
| -------------------- | ------------------------------- | -------------------------- |
| Member coverage      | 42,929, the same as before      | 13,097, the same as before |
| Cold index           | 6.0 s (6.1 s before)            | 0.8 s (1.0 s before)       |
| Update of one file   | 96 ms (95 ms before)            | 25 ms (24 ms before)       |
| Peak resident memory | 612 MB                          | 260 MB                     |
| All cases            | 87 passed, 7 failed, as before  | 31 passed, 0 failed        |
| Diagnostics          | 94 in custom modules, as before | 0                          |

The Symfony sweep covers `src`, `templates`, `config` and `tests` (1,047 files, slowest 5 ms) and finds no broken name. Its templates and parameters are certain: names injected into a template and into `services.yaml` were flagged in a probe. Its route names stay silent, because two routes are named by a class constant the module cannot read; a review copy with those two names written as literals found no route diagnostic either, and flagged an injected one. The Drupal sweep now also reads 1,256 YAML and Twig files of the custom modules, which get no diagnostic. A first run found no parameter diagnostic at all: the compiled container under `var/cache` counted as PHP configuration, and the module now leaves `var` out.

## 2026-10-28, step 6f Doctrine mapping

Same machine and checkouts as step 6e. Class and property names in `*.orm.xml` mappings open their declarations, and `getRepository(X::class)` is typed from the mappings. The build before this step ran first.

| Metric               | Drupal reference                | Symfony reference          |
| -------------------- | ------------------------------- | -------------------------- |
| Member coverage      | 42,929, the same as before      | 13,097, the same as before |
| Cold index           | 5.9 s (7.1 s before, run first) | 0.9 to 1.2 s               |
| Update of one file   | 93 ms (97 ms before)            | 24 ms (19 ms before)       |
| Peak resident memory | 625 MB                          | 256 MB                     |
| All cases            | 87 passed, 7 failed, as before  | 31 passed, 0 failed        |
| Diagnostics          | 94, the same as before          | 0, the same as before      |

All 344 class and property names in the 30 mapping files of the Symfony reference project resolve. Its mappings declare no repository class, so `getRepository()` keeps its declared type there. New cases cover an entity name, a field name, a `target-entity` and an `enum-type`. A first version walked the whole project for mapping files on every PHP update, which cost 58 ms; the module now walks again only when a directory or a mapping file changes.

## 2026-10-28, step 6e Symfony parameters and YAML

Same machine and checkouts as step 6d. Parameter names in PHP attributes, `getParameter()` and YAML lead to their definitions, and class names and `@id` references in Symfony YAML configuration open their class or service. This time the build before this step ran first.

| Metric               | Drupal reference                | Symfony reference          |
| -------------------- | ------------------------------- | -------------------------- |
| Member coverage      | 42,929, the same as before      | 13,097, the same as before |
| Cold index           | 6.4 s (7.2 s before, run first) | 0.9 s                      |
| Update of one file   | 99 ms (109 ms before)           | 19 ms, the same as before  |
| Peak resident memory | 616 MB                          | 259 MB                     |
| All cases            | 87 passed, 7 failed, as before  | 27 passed, 0 failed        |
| Diagnostics          | 94, the same as before          | 0, the same as before      |

All 41 parameter names in `#[Autowire]` attributes of the Symfony reference project resolve, apart from the kernel's and environment variables, which have no target. Every `@id` and class name in its `services.yaml` resolves; a probe that also matched `@sha256` image digests, `when@` keys and namespace prefixes found 75 of 83. New cases cover an `#[Autowire(param:)]` name, a `%x%` in YAML, a class key and an `@id` in `services.yaml`, and a kernel parameter expecting nothing.

## 2026-10-28, step 6d Symfony Messenger

Same machine and checkouts as step 6c. `dispatch()` lists the handlers of its message, and a handler lists the dispatches of its messages. Both projects were measured with this branch and with the build before it.

| Metric               | Drupal reference                   | Symfony reference          |
| -------------------- | ---------------------------------- | -------------------------- |
| Member coverage      | 42,929, the same as before         | 13,097, the same as before |
| Cold index           | 5.6 to 5.8 s (5.8 to 6.0 s before) | 0.9 s                      |
| Update of one file   | 108 ms (94 ms before)              | 17 ms (16 ms before)       |
| Peak resident memory | 578 MB                             | 248 MB                     |
| All cases            | 87 passed, 7 failed, as before     | 22 passed, 0 failed        |
| Diagnostics          | 94, the same as before             | 0, the same as before      |

Of the 28 `dispatch()` calls in the Symfony reference project, 27 list their handlers, and 28 of its 30 handlers list their dispatches. The other dispatch sends a message whose type comes from a callable through a method-level template, which also leaves its handler without dispatches; the last handler's message is sent by the Scheduler, not by `dispatch()`. New cases cover a `dispatch()` with its handler and that handler with its dispatch.

The Drupal cold index was measured alternating this branch and the build before it: the first run after a build is about 1 s slower, which explains the differences recorded in steps 6a to 6c.

## 2026-10-28, step 6c Symfony templates

Same machine and checkouts as step 6b. Template names in PHP and Twig open their Twig file. Both projects were measured with this branch and with the build before it.

| Metric               | Drupal reference               | Symfony reference          |
| -------------------- | ------------------------------ | -------------------------- |
| Member coverage      | 42,929, the same as before     | 13,097, the same as before |
| Cold index           | 6.8 s (5.8 s before)           | 1.1 s (0.9 s before)       |
| Update of one file   | 98 ms (92 ms before)           | 15 ms (16 ms before)       |
| Peak resident memory | 525 MB                         | 235 MB                     |
| All cases            | 87 passed, 7 failed, as before | 20 passed, 0 failed        |
| Diagnostics          | 94, the same as before         | 0, the same as before      |

Of the 307 literal template names the Symfony reference project writes in `render()`, `renderView()`, `#[Template]` and Twig tags and functions, 302 resolve. Of the other 5, one is built by concatenation, three are JavaScript imports inside a Markdown template, and one is TwigBridge's `form_div_layout.html.twig`. New cases cover a namespaced `render()` name, a plain `extends` name, an `include()` name and an `import` name. Cold index times vary from run to run, as in earlier steps.

## 2026-10-28, step 6b Symfony routes

Same machine and checkouts as step 6a. Route names in `redirectToRoute()`, `generateUrl()`, `UrlGeneratorInterface::generate()` and Twig `path()` and `url()` lead to the route's controller method or YAML definition. Both projects were measured with this branch and with the build before it.

| Metric               | Drupal reference               | Symfony reference          |
| -------------------- | ------------------------------ | -------------------------- |
| Member coverage      | 42,929, the same as before     | 13,097, the same as before |
| Cold index           | 7.5 s (6.3 s before)           | 1.0 s                      |
| Peak resident memory | 558 MB                         | 189 MB                     |
| Update of one file   | 105 ms (95 ms before)          | 20 ms (10 ms before)       |
| All cases            | 87 passed, 7 failed, as before | 16 passed, 0 failed        |
| Diagnostics          | 94, the same as before         | 0, the same as before      |

Of the 383 literal route names in the Symfony reference project's PHP and Twig files, 381 resolve. The other 2 name a route whose attribute gives its name through a class constant. New cases cover a `redirectToRoute()` name, a `generate()` name and a Twig `path()` name. On the Symfony reference project every PHP update reads the routes again, which the module keeps to about 10 ms by checking route attributes before paths. The Drupal timings vary from run to run as in earlier steps.

## 2026-10-28, step 6a Symfony module skeleton

Same machine and checkouts as step 5c. Each framework module now reads and answers only for its own project. A Symfony module turns on with `symfony/framework-bundle` and does nothing else yet. Both projects were measured with this branch and with the build before it.

Drupal reference project, where Drupal is the only module and reads the whole root:

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,929 of 45,837 accesses (93.7%) | at least 42,929 |
| Cold index                              | 6.5 s                             | under 10 s      |
| Warm start from the cache               | 4.7 s                             | under 2 s       |
| Update of one file                      | 96 ms                             | under 0.5 s     |
| Update of 1,000 files                   | 6.8 s                             | under 5 s       |
| Peak resident memory                    | 627 MB                            | under 1024 MB   |
| Slowest query case                      | 189 ms                            | under 1 s       |
| All cases                               | 87 passed, 7 failed               | all pass        |
| Diagnostics in `web/modules/custom`     | 94, the same as before            | no new ones     |

Coverage, cases and diagnostics match the build before this step, which measures 5.8 s cold, 91 ms for one update and 619 MB in the same run. A project that covers the whole root skips the scope check, which otherwise added about 30% to every update. The 7 failing cases point into files this branch of the project does not have, as in step 5b.

Symfony reference project, 16,931 PHP files:

| Metric                   | Value                             | Target          |
| ------------------------ | --------------------------------- | --------------- |
| Member coverage in `src` | 13,097 of 13,211 accesses (99.1%) | at least 13,097 |
| Cold index               | 1.2 s                             | under 10 s      |
| Peak resident memory     | 180 MB                            | under 1024 MB   |
| All cases                | 13 passed, 0 failed               | all pass        |
| Diagnostics in `src`     | 0, the same as before             | no new ones     |

Both modules are active: Drupal reads `codebase/drupal`, and Symfony reads the root without it. Before this step Drupal read the whole tree, Symfony code included; the build before measures 2.2 s cold and 206 MB. No access in `src` resolved through Drupal, so coverage is unchanged.

## 2026-10-28, step 5c element types

Same machine and checkouts as step 5b. Element types flow from declared arrays and generic collections to `foreach` values, array access, untyped callback parameters of built-in array functions, and members of generic receivers such as `$items->first()` and `->toArray()`. Both projects were measured with this branch and with the build before it.

Drupal reference project:

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,929 of 45,837 accesses (93.7%) | at least 42,514 |
| Cold index                              | 6.1 to 7.8 s                      | under 10 s      |
| Warm start from the cache               | 4.8 to 5.3 s                      | under 2 s       |
| Update of one file                      | 92 to 104 ms                      | under 0.5 s     |
| Update of 1,000 files                   | 7.0 to 7.4 s                      | under 5 s       |
| Peak resident memory                    | 584 to 619 MB                     | under 1024 MB   |
| Slowest query case                      | 158 to 191 ms                     | under 1 s       |
| All cases                               | 87 passed, 7 failed               | all pass        |
| Diagnostics in `web/modules/custom`     | 94, the same as before            | no new ones     |

The build before this step resolves 42,514 accesses, and no access it resolves is lost. It peaks at 622 MB in the same run, so memory varies from run to run rather than with this step. Most of the 415 new ones are entities from `loadMultiple()` reaching callbacks and array access. The 7 failing cases point into files this branch of the project does not have, as in step 5b. The warm start and the update of 1,000 files still miss their targets, as before this step. A new case covers an untyped `array_map` callback over `array_filter()` of loaded roles.

Symfony reference project, 16,931 PHP files:

| Metric                   | Value                             | Target        |
| ------------------------ | --------------------------------- | ------------- |
| Member coverage in `src` | 13,097 of 13,211 accesses (99.1%) | above 13,001  |
| Cold index               | 1.9 to 2.5 s                      | under 10 s    |
| Peak resident memory     | 192 to 196 MB                     | under 1024 MB |
| All cases                | 13 passed, 0 failed               | all pass      |
| Diagnostics in `src`     | 0, the same as before             | no new ones   |

New cases cover a `foreach` over a `Collection<int, X>` property, an untyped `array_map` callback over a `list<X>` result, array access on a typed list, a value from `->toArray()` of a collection, and a parameter with no element type of its own, which resolves to nothing. Doctrine's `ReadableCollection` mixes `@phpstan-template` and `@template`, so templates are read in docblock order whatever their prefix. The 114 accesses left come from array shapes and destructuring, invokable objects called as `($this->handler)(...)`, method-level templates such as `getRepository(X::class)`, parameter types documented only on the interface, and arrays built by assignment.

## 2026-10-28, step 5b enums

Same machine. Every enum implements `UnitEnum`, and a backed enum `BackedEnum` too, so `from()`, `tryFrom()`, `cases()`, `->value` and `->name` lead to their stubs. An enum case has its enum's type, so `Role::PlatformAdmin->label()` and `Status::from($x)->label()` resolve.

The Drupal reference project is on another branch than in step 5a, so both projects were measured with this branch and with the build before it, on the same checkouts.

Drupal reference project:

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,514 of 45,837 accesses (92.8%) | at least 42,481 |
| Cold index                              | 6.7 s                             | under 10 s      |
| Warm start from the cache               | 4.7 s                             | under 2 s       |
| Update of one file                      | 91 ms                             | under 0.5 s     |
| Update of 1,000 files                   | 6.8 s                             | under 5 s       |
| Peak resident memory                    | 547 MB                            | under 1024 MB   |
| Slowest query case                      | 155 ms                            | under 1 s       |
| All cases                               | 86 passed, 7 failed               | all pass        |
| Diagnostics in `web/modules/custom`     | 94, the same as before            | no new ones     |

The build before this step resolves 42,481 accesses on the same checkout. The 7 failing cases point into files this branch of the project does not have, and they fail the same way before this step. The warm start and the update of 1,000 files still miss their targets, as before this step. New cases cover `tryFrom()` and `->value` on an enum case.

Symfony reference project, 16,931 PHP files:

| Metric                   | Value                             | Target        |
| ------------------------ | --------------------------------- | ------------- |
| Member coverage in `src` | 13,001 of 13,211 accesses (98.4%) | above 12,801  |
| Cold index               | 2.3 s                             | under 10 s    |
| Peak resident memory     | 187 MB                            | under 1024 MB |
| All cases                | 8 passed, 0 failed                | all pass      |
| Diagnostics in `src`     | 0, the same as before             | no new ones   |

New cases cover `from()` on a backed enum, `->value` on an enum-typed property, a method called on an enum case, and a method called after `from()`. Of the 210 accesses left, the enum ones are on values read from arrays or loops, which step 5c covers.

## 2026-10-28, step 5a built-in class members

Same machine. Built-in classes, functions, constants and their members lead to their declaration in the phpstorm stubs, and their return types carry navigation on.

Drupal reference project:

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,824 of 46,204 accesses (92.7%) | at least 42,455 |
| Cold index                              | 5.7 to 6.8 s                      | under 10 s      |
| Warm start from the cache               | 4.7 s                             | under 2 s       |
| Update of one file                      | 90 to 97 ms                       | under 0.5 s     |
| Update of 1,000 files                   | 6.8 to 6.9 s                      | under 5 s       |
| Peak resident memory                    | 483 to 612 MB                     | under 1024 MB   |
| Slowest query case                      | 120 to 153 ms                     | under 1 s       |
| All cases                               | 91 passed, 0 failed               | all pass        |
| Diagnostics in `web/modules/custom`     | 94, the same as before            | no new ones     |

The warm start and the update of 1,000 files miss their targets, and the build before this step measures the same (4.7 s and 6.8 s), so the miss is older than this step. The stubs add about 40 MB of memory. The `count()` case now expects its stub, and new cases cover `getMessage()` on a caught exception and `format()` after `new \DateTimeImmutable()`.

Symfony reference project, 16,931 PHP files:

| Metric                   | Value                             | Target        |
| ------------------------ | --------------------------------- | ------------- |
| Member coverage in `src` | 12,801 of 13,211 accesses (96.9%) | above 12,568  |
| Cold index               | 1.7 s                             | under 10 s    |
| Peak resident memory     | 175 to 186 MB                     | under 1024 MB |
| All cases                | 4 passed, 0 failed                | all pass      |
| Diagnostics in `src`     | 0, the same as before             | no new ones   |

Coverage was 12,568 (95.1%) before this step. The cases cover `getMessage()` on a caught `RuntimeException`, `format()` on a `DateTimeImmutable` property, a qualified `\count()`, and a method reached through an injected interface, which keeps its target.

## 2026-10-27, names in docblocks

Same machine and project. Class names in PHPDoc types, members and functions in `@see`, `@uses` and `@covers`, and annotation names lead to their declarations, and references list them.

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%) | at least 42,455 |
| Cold index                              | 5.9 to 6.8 s, once 7.3 s          | under 10 s      |
| Peak resident memory                    | 424 to 451 MB                     | under 1024 MB   |
| Slowest definition case                 | 35 to 41 ms                       | under 1 s       |
| All cases                               | 89 passed, 0 failed               | all pass        |

New cases cover a core interface in a `@param`, a core method in a `@see`, an unimported webform handler annotation, a nested `@Translation`, and an unimported `@WebformElement` beside a plugin class of the same name, which leads to the annotation class as plugin discovery reads it. The references case for `EntityStorageInterface::load` gained the `@see` in `EntityDisplayRepositoryInterface`.

## 2026-10-27, step 4f broken Drupal references

Same machine and project. `phplumb bench --diagnostics <dir>` lists every diagnostic; the Drupal ones are service ids, route names and plugin ids defined nowhere on disk. Every Drupal diagnostic below was checked by hand.

| Area                  | Files  | Drupal diagnostics | What they are                                                                                                                     | Slowest file |
| --------------------- | ------ | ------------------ | --------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| `web/modules/custom`  | 1,228  | 6                  | 4 `'#type' => 'markup'`, 2 uses of a formatter defined nowhere                                                                    | 4 ms         |
| `web/modules/contrib` | 6,596  | 82                 | 34 `markup`, 16 wrong widget or formatter ids, 27 services and routes of modules not on disk or missing files, 5 other plugin ids | 32 ms        |
| `web/core`            | 11,473 | 11                 | 5 services called only by deprecated fallbacks, 5 `markup`, 1 `'#type' => '#markup'`                                              | 29 ms        |

The first sweep found 519 Drupal diagnostics. The false alarms came from routes declared by attributes, route aliases, routes and services defined by PHP code, hook classes as services, plugin managers that look ids up their own way, webform elements, test code and `*.api.php` files; each became a rule with a test. Nine real contrib findings stay silent because the engine cannot rule them out: core and contrib route providers build names such as `"$prefix.view"` and `"$entity_type_id.settings"`, and a service provider mentions `hal.link_manager`.

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%) | at least 42,455 |
| Cold index                              | 5.9 to 6.3 s                      | under 10 s      |
| Peak resident memory                    | 397 to 443 MB                     | under 1024 MB   |
| All cases                               | 84 passed, 0 failed               | all pass        |

## 2026-10-27, step 4e Drupal routes and YAML

Same machine and project. The Drupal module reads routes from `*.routing.yml`, and the server answers definitions inside services, routing and links YAML files.

| Metric                                  | Value                                                | Target          |
| --------------------------------------- | ---------------------------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%)                    | at least 42,455 |
| Cold index                              | 5.0 to 5.8 s, once 7.5 s with the machine under load | under 10 s      |
| Peak resident memory                    | 420 to 474 MB                                        | under 1024 MB   |
| Slowest definition case                 | 36 to 45 ms                                          | under 1 s       |
| All cases                               | 84 passed, 0 failed                                  | all pass        |

New cases cover a route name in `Url::fromRoute()`, a `_controller` and a `_form` in custom routing files, a `route_name` in a custom links file, and a class, an `@` argument and a factory method in custom services files.

## 2026-10-27, step 4d Drupal plugins

Same machine and project. The Drupal module reads plugin types from the constructors of plugin managers and plugins from each type's subdirectory of every module and core component namespace, by attribute or annotation, imported or not, and managers with the older constructor that reads annotations only.

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%) | at least 42,455 |
| Cold index                              | 3.9 to 5.7 s                      | under 10 s      |
| Peak resident memory                    | 421 to 453 MB                     | under 1024 MB   |
| Slowest definition case                 | 29 to 101 ms                      | under 1 s       |
| Slowest implementation case             | 79 to 98 ms                       |                 |
| All cases                               | 77 passed, 0 failed               | all pass        |

New cases cover `#type` values in custom forms, field types in `BaseFieldDefinition::create()`, a widget and a formatter in display options, a custom constraint added to a narrowed field config, `createInstance()` on a contrib plugin manager, the plugins of a core plugin manager, and the plugins of a contrib manager with the older constructor, whose annotations are not imported. An entity type constraint added through `$entity_types['node']->addConstraint()` stays unresolved, because the type of an array element is not inferred.

## 2026-10-27, step 4c Drupal hooks

Same machine and project. The Drupal module reads extensions from `*.info.yml` files and hook implementations the way Drupal 11 discovers them, procedural and `#[Hook]` attributes. Invocations are found when a definition is asked, among the calls of the module handler and theme manager.

| Metric                                  | Value                             | Target          |
| --------------------------------------- | --------------------------------- | --------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%) | at least 42,455 |
| Cold index                              | 4.1 to 6.2 s                      | under 10 s      |
| Peak resident memory                    | 404 to 461 MB                     | under 1024 MB   |
| Slowest definition case                 | 31 to 68 ms                       | under 1 s       |
| Slowest references case                 | 152 to 196 ms                     | under 1 s       |
| All cases                               | 67 passed, 0 failed               | all pass        |

New cases cover a custom `entity_presave`, `node_insert`, `form_node_form_alter` and `toolbar_alter` hook method, a theme's `library_info_alter` and `preprocess_html` functions, and the hook name in `invokeAll('toolbar')`, which lists 14 implementations and the documentation. Every site of the computed-name table exists in Drupal core 11.4.10. The bench now reports the slowest definition case.

## 2026-10-27, step 4b.2 property assignments

Same machine and project. A property's type now adds what its class and the class's parents assign to it, in the constructor, in `create()` and in any other method.

| Metric                                  | Value                             | Target        |
| --------------------------------------- | --------------------------------- | ------------- |
| Member coverage in `web/modules/custom` | 42,455 of 46,204 accesses (91.9%) | above 42,452  |
| Cold index                              | 3.9 to 5.3 s                      | under 10 s    |
| Peak resident memory                    | 401 to 473 MB                     | under 1024 MB |
| Slowest references case                 | 172 to 305 ms                     | under 1 s     |
| All cases                               | 60 passed, 0 failed               | all pass      |

New cases cover a method after `load()` on a storage kept by a constructor, one kept by `create()`, and one kept by a constructor in core views. Custom code on this project calls `getStorage()` inline and declares every property type, so its coverage gains only 3 accesses. References of `EntityStorageInterface::load` grew from 1,156 to 1,379, mostly in tests that reach storages through `$this->container`, which their base class assigns.

## 2026-10-27, step 4b Drupal entities

Same machine and project. The Drupal module reads entity types from attributes (65 outside tests) and annotations (62), types `getStorage('id')` as that entity type's storage, and `load()`, `create()` and `loadMultiple()` on it as the entity class.

| Metric                                  | Value                             | Target        |
| --------------------------------------- | --------------------------------- | ------------- |
| Member coverage in `web/modules/custom` | 42,452 of 46,204 accesses (91.9%) | above 91.6%   |
| Cold index                              | 3.7 to 5.0 s                      | under 10 s    |
| Peak resident memory                    | 405 to 445 MB                     | under 1024 MB |
| Slowest references case                 | 163 to 244 ms                     | under 1 s     |
| All cases                               | 57 passed, 0 failed               | all pass      |

New cases cover methods on files, menu links, a custom entity type and a contrib entity type declared by annotation, each loaded through its storage, and an entity type id leading to its declaration. With storages typed precisely, calls such as `getStorage('node')->load()` resolve to the storage's own `load()`; references of `EntityStorageInterface::load` now also list calls to its implementations, and grew from 1,104 to 1,156.

## 2026-10-27, step 4a Drupal services

Same machine and project. The Drupal module reads 252 services files (test modules and `vendor` excluded) and types `\Drupal::service()` and container `get()` calls.

| Metric                                  | Value                             | Target        |
| --------------------------------------- | --------------------------------- | ------------- |
| Member coverage in `web/modules/custom` | 42,303 of 46,204 accesses (91.6%) | above 90.7%   |
| Cold index                              | 3.4 to 4.5 s                      | under 10 s    |
| Peak resident memory                    | 382 to 402 MB                     | under 1024 MB |
| All cases                               | 52 passed, 0 failed               | all pass      |

New cases cover a method after `\Drupal::service()`, after `$container->get()` in `create()`, through an alias, a service id leading to its YAML key, and an id defined nowhere on disk. A service another module decorates resolves to both classes: `entity_type.manager` gives the core class and the webprofiler wrapper. Two earlier cases changed because more calls are now typed: a method on a service result that used to expect nothing, and the references of `EntityStorageInterface::load`, which grew from 1,033 to 1,104 with calls on storages taken from the container.

## 2026-10-27, step 3 broken reference diagnostics

Same machine and project. `phplumb bench --diagnostics <dir>` checks every file below a directory, whatever the editor's project-code scope, and lists each diagnostic.

| Area                  | Files  | Diagnostics | In `tests` | Slowest file |
| --------------------- | ------ | ----------- | ---------- | ------------ |
| `web/modules/custom`  | 1,228  | 88          | 0          | 2 ms         |
| `web/core`            | 11,473 | 65          | 64         | 32 ms        |
| `web/modules/contrib` | 6,596  | 253         | 89         | 28 ms        |

Cold index 4.3 s, peak memory 429 MB, 48 of 48 cases, member coverage 90.7%.

**Custom code: no false alarms.** All 88 diagnostics come from one module that uses three Composer packages (a SOAP client, its encoding library and a type library) that the project does not install; those classes and functions exist nowhere on disk.

**Core and contrib** were used to hunt false alarms, since that code works. The first run found four classes of false alarm, each now covered by a fixture case and fixed:

- A class declared twice (a static-analysis stub and the real class): member lookups now try every declaration of a name, which also fixes go to definition through such a parent.
- A receiver typed only by a wrong docblock, or holding a test mock: a method on any receiver other than `$this` is flagged only when no indexed or built-in class declares a method with that name.
- Methods added by trait `as` aliases.
- Example code in `*.api.php` hook documentation.

The final review found five more classes, now also fixed and covered: trait aliases declared inside another trait, `$this` and `self::` inside closures (whose scope can be rebound), classes created with `class_alias()`, `is_callable()` and named-argument guards, and names in namespaces that `phplumb.exclude` hides from the index.

What remains are references to modules and libraries the project does not install (optional integrations), Drupal 7 and Drush 8 functions in legacy files, and methods no class declares at all, such as `ModuleHandlerInterface::getImplementations()`, which Drupal 11 removed. One known false alarm class remains: classes that a vendor package loads with `include` instead of through Composer's autoloader (geoPHP's `Geometry`), because the vendor filter does not index those files. It does not occur in this project's custom code.

## 2026-10-27, step 2e Linux binary

Static musl builds in Docker Desktop on the same Mac, with the project copied into the container's own filesystem. Bind-mounting the project from macOS instead raises the cold index to 18 to 19 seconds on both architectures, so that number measures Docker Desktop's file sharing, not the engine.

| Metric                  | `x86_64` (emulated) | `aarch64` (native)  | Target        |
| ----------------------- | ------------------- | ------------------- | ------------- |
| Cold index              | 2.5 s               | 1.7 s               | under 10 s    |
| Peak resident memory    | 475 MB              | 447 MB              | under 1024 MB |
| Slowest references case | 273 ms              | 134 ms              | under 1 s     |
| Slowest symbol query    | 32 ms               | 33 ms               | under 100 ms  |
| All cases               | 48 passed, 0 failed | 48 passed, 0 failed | all pass      |

The `x86_64` binary also runs on Debian 11 (glibc 2.31), and the Rust test suite passes on `linux/amd64`. No native Linux x64 machine was used.

## 2026-10-26, step 2e symbols and outline

Same machine and project. Every run now times five workspace symbol queries (`node`, `EntityStorage`, `gEnt`, `load` and `zzqqxx`) over all declarations and members in the index.

| Metric                  | Value               | Target        |
| ----------------------- | ------------------- | ------------- |
| Cold index              | 2.6 to 4.3 s        | under 10 s    |
| Peak resident memory    | 345 to 366 MB       | under 1024 MB |
| Slowest symbol query    | 29 to 31 ms         | under 100 ms  |
| Slowest references case | 103 to 135 ms       | under 1 s     |
| All cases               | 48 passed, 0 failed | all pass      |

## 2026-10-26, step 2d references and call hierarchy

Same machine and project. Each indexed file now records hashes of the names it mentions, so a references query parses only the files that can contain a match.

| Metric                                  | Value                                                       | Target        |
| --------------------------------------- | ----------------------------------------------------------- | ------------- |
| Cold index                              | 2.8 s                                                       | under 10 s    |
| Peak resident memory                    | 381 MB                                                      | under 1024 MB |
| Slowest references case                 | 108 ms (1,033 references of `EntityStorageInterface::load`) | under 1 s     |
| Slowest implementation case             | 11 ms                                                       | under 1 s     |
| All cases                               | 48 passed, 0 failed                                         | all pass      |
| Member coverage in `web/modules/custom` | 41,894 of 46,204 accesses (90.7%)                           | 90.7% before  |

The custom references cases match an independent text search exactly. They cover two classes named `ImportStatisticsTracker` in different modules, where each class and its `trackCreated` method must list only its own uses, and a global function whose docblock mentions are not references. The expected list of the `load` case comes from the engine, with 20 sampled entries checked by hand.

## 2026-10-26, step 2c implementations and type hierarchy

Same machine and project. Members documented with `{@inheritdoc}` and no type of their own now take the nearest typed ancestor's type.

| Metric                                  | Value                                             | Target        |
| --------------------------------------- | ------------------------------------------------- | ------------- |
| Cold index                              | 2.4 to 3.9 s                                      | under 10 s    |
| Peak resident memory                    | 354 MB                                            | under 1024 MB |
| Slowest implementation case             | 7 ms (195 implementations of `ProcessPluginBase`) | under 1 s     |
| Definition and implementation cases     | 42 passed, 0 failed                               | all pass      |
| Member coverage in `web/modules/custom` | 41,894 of 46,204 accesses (90.7%)                 | 88.6% before  |

The implementation cases were checked against an independent text scan of `extends` and `implements` across core, contrib and custom code; every difference was a limit of the scan (classes sharing a short name), not of the engine.

## 2026-10-26, step 2b members and hover

Same machine and project. The index now records class shapes (parents, interfaces, traits and typed members); signatures and docblocks are read from the file when hover needs them.

| Metric                                  | Value                             | Target        |
| --------------------------------------- | --------------------------------- | ------------- |
| Cold index                              | 2.6 to 3.8 s                      | under 10 s    |
| Warm start from the cache               | 1.6 to 1.7 s                      | under 2 s     |
| Update of 1,000 files                   | 39 ms                             | under 5 s     |
| Peak resident memory                    | 426 to 471 MB                     | under 1024 MB |
| Cache size                              | 21 MB                             |               |
| Definition cases                        | 36 passed, 0 failed               | all pass      |
| Member coverage in `web/modules/custom` | 40,951 of 46,204 accesses (88.6%) | baseline      |

The coverage counts every `->`, `?->` and `::` access whose member is named in the code and reaches at least one definition. A sample of 27 resolved accesses, read by hand, had no wrong target. Most unresolved accesses go through the Drupal service container or magic field properties, which the Drupal module addresses.

## 2026-10-26, step 2a index freshness

Same machine and project. `bench --cache` writes the cache after the cold build, builds again from it, then updates files without changing them.

| Metric                    | Value               | Target        |
| ------------------------- | ------------------- | ------------- |
| PHP files indexed         | 39,417              |               |
| Cold index                | 2.2 to 3.7 s        | under 10 s    |
| Warm start from the cache | 1.6 to 1.7 s        | under 2 s     |
| Update of one file        | under 1 ms          | under 0.5 s   |
| Update of 1,000 files     | 37 to 39 ms         | under 5 s     |
| Peak resident memory      | 220 to 339 MB       | under 1024 MB |
| Cache size                | 8 MB                |               |
| Definition cases          | 20 passed, 0 failed | all pass      |

Warm start time is mostly the directory walk and file stats; parsing is skipped for unchanged files.

## 2026-10-26, step 1 skeleton

Machine: Apple M3 Pro, 11 cores, 36 GB RAM. Release build. Reference project: Drupal `11.4.10`.

| Metric                              | Value               | Target        |
| ----------------------------------- | ------------------- | ------------- |
| PHP files indexed                   | 39,417              |               |
| Declarations                        | 39,830              |               |
| Files with parse errors             | 0                   |               |
| Cold index, warm file cache         | 2.9 to 3.8 s        | under 10 s    |
| Cold index, first run after a while | 6.7 s               | under 10 s    |
| Peak resident memory                | 185 to 283 MB       | under 1024 MB |
| Definition cases                    | 20 passed, 0 failed | all pass      |

Without the Composer vendor filter the same project yields 52,948 files and 242 files with parse errors. The difference is package tests, fixtures and a stale copy of Drupal core under `vendor/composer` that no autoloader reaches.

The case set covers custom code reaching core, contrib and vendor classes, functions declared in `.module` and `.inc` files, an aliased namespace import, a class declared in two places, and four positions that must return nothing.
