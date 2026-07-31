# Editions

## Interop rule

A module in one edition must interoperate seamlessly with a module in another edition.

This rule is the point of editions. If editions split module interop, they split the ecosystem and are worse than never evolving the language. So edition changes are skin deep: they can change parsing and surface spelling, but must not change what a module means once lowered into the shared internal representation.

## Where a module declares its edition

A module may declare an edition on its `module` line:

```deed
module payments/ledger edition 2025
```

If omitted, the module is in edition `2024`.

The declaration is per module, not per build and not per repository. That keeps migration incremental and allows old and new modules to compile together.

## What an edition may and may not change

Allowed edition changes:

- Surface syntax and parser rules that erase before resolution and type checking.
- Keyword policy, including promoting existing soft keywords to hard keywords in a new edition.
- Parser recovery and diagnostics quality, when semantics are unchanged.

Not allowed edition changes:

- Name resolution semantics once syntax is lowered.
- Type system meaning or runtime behavior of existing constructs.
- Module interface representation seen by other modules.

First concrete candidate: promote a soft keyword such as `refuses` (or `ok`, `err`, `state`, `at`, `while`) to a real keyword in a new edition.

## Mechanical migration and formatter requirements

Migration should be mechanical.

For an edition upgrade to be mechanical, deed-fmt must be able to print one canonical spelling for each edition. In practice that means:

- deed-fmt must preserve and print the module's edition declaration.
- deed-fmt must normalize edition-scoped surface spellings into that edition's canonical form.
- if a migration introduces fallback spellings, deed-fmt must rewrite them deterministically.

With those guarantees, `deed-fmt --edition <target>` can become the upgrade tool: parse with old rules, print with new canonical rules, then run normal checks.

## Minimal mechanism in this change

This change adds the scaffold and one edition-gated parse difference to prove the mechanism:

- optional `edition <year>` on a module declaration (`2024` and `2025` recognized)
- a parser toggle: `use ...;` is accepted in `edition 2025` and rejected in `edition 2024`
- a cross-module test showing edition `2024` and `2025` modules interoperating

The toggle is intentionally small. Its job is to prove per-module edition dispatch and mixed-edition compilation without introducing a second full grammar.
