# Checker semantics

This document states what the checker enforces today.

It is a rules document, not a history. Every rule below is grounded in current code in `crates/deed-typeck`.

## Typing rules

### 1. Checking is local and bidirectional

The checker is bidirectional and local. There is no global inference and no unification variables. `let` initializers are inferred, and signatures drive checking elsewhere. See `crates/deed-typeck/src/check.rs`, module docs and `Checker::check_against` and `Checker::infer`.

### 2. `compatible` is structural except for explicit escape hatches

`Checker::compatible` compares types structurally:

- `Result`, `List`, `Named`, `External`, and `Fn` are compared componentwise.
- For `Fn`, parameters and return are compared componentwise.
- The row check uses `FnRow::within`.

See `crates/deed-typeck/src/check.rs`, `Checker::compatible`.

### 3. `Unknown` and `Never` absorb compatibility checks

`compatible` returns true when either side "absorbs". This is why unresolved or already-reported shapes do not cascade into extra type errors, and why non-returning expressions fit any expected type. See `crates/deed-typeck/src/check.rs`, `Checker::compatible`, and `crates/deed-typeck/src/ty.rs`, `Ty::Unknown` and `Ty::Never` docs.

### 4. Refinement fit is directional and happens outside `compatible`

`compatible` itself does not treat refinement names as subtypes of their base type. Refinement-to-base fit is handled by widening in assignment (`Checker::widen` and `Checker::assign_carrying`). The reverse direction is an obligation, not implicit fit. See `crates/deed-typeck/src/check.rs`, `Checker::widen`, `Checker::assign_carrying`, and `Checker::prove_refinement`.

### 5. `settled` computes the post-join type for branching and loops

`settled(carried, produced)` keeps the carried shape unless the carried side is unknown at a position. It recurses into `List`, `Result`, `Named`, and `External` arguments.

Rule:

- Unknown-in-carried is filled from produced, unless produced is also absorbing.
- Known carried parts are preserved.

This is used for both `if` joins and `for` accumulator results. See `crates/deed-typeck/src/check.rs`, `settled`, `settled_all`, the `if` join in `Checker::check_if`, and the loop result in `Checker::check_for`.

For a loop that may run zero times, the initializer still has to be accepted. This is why `settled` preserves known carried types and only fills unknown slots. See `Checker::check_for` and `settled` in `crates/deed-typeck/src/check.rs`.

## Effect and row rules

### 6. Rows are part of function types

Rows are in `Ty::Fn { row: FnRow, ... }`. A row difference is a type difference. See `crates/deed-typeck/src/ty.rs`, `FnRow` and `Ty::Fn`.

### 7. Declared row normalization is canonical

Rows from syntax are lowered through `RowLowering::normalised`, then stored as `FnRow::Declared`. Ordering and duplicates are removed so equivalent spellings compare equal. See `crates/deed-typeck/src/check.rs`, `Checker::lower_type` and signature collection in `Checker::collect`.

### 8. `FnRow::within` is the only subtyping relation inside `compatible`

`FnRow::within` is containment of actual-performed operations inside expected-allowed operations.

- Each actual row entry must be named directly in expected, or covered by an expected whole-effect entry (`operation: None` with matching module/effect).
- If expected contains a row variable entry, `within` returns true.

This is the only non-equality relation in `compatible`. See `crates/deed-typeck/src/ty.rs`, `FnRow::within`, and `crates/deed-typeck/src/check.rs`, `Checker::compatible`.

### 9. Row variables are declaration-scoped and not globally meaningful

Before lowering rows in a function, the checker sets which row variables are in scope by calling `rows.declaring(&function.sig.rows)`. This is done both in signature collection and when checking the body. See `crates/deed-typeck/src/check.rs`, `Checker::collect` and `Checker::check_fn_against`.

A row variable is treated as a permissive placeholder in expected rows, and row obligations are not recorded for function values when expected contains a variable entry. See `Checker::assign_carrying` and `FnRow::within`.

### 10. What a row variable may and may not appear in, as enforced here

In this checker pass, a row variable only has meaning when it appears in rows lowered under a declaration that introduced it (`rows.declaring`). Outside that context it is just a normal effect-name-shaped entry.

Operationally, row-variable behavior is only consumed in two places:

- `FnRow::within` special-cases expected rows that contain variable entries.
- `Checker::assign_carrying` skips `Types::require_row` when expected rows contain variable entries.

See `crates/deed-typeck/src/check.rs`, `Checker::check_fn_against` and `Checker::assign_carrying`, and `crates/deed-typeck/src/ty.rs`, `FnRow::within`.

## Resolution and module rules

### 11. Cross-module checking uses a surface world

Cross-module typing does not carry `DefId`s across files. The checker reads `SurfaceItem`s in a `World`, where identities are portable (`Ty::External` by module path and name). See `crates/deed-typeck/src/surface.rs`, module docs, `surface`, and `World::get`.

### 12. Alias expansion versus nominal preservation

Transparent aliases expand to their target type. Refinements stay nominal.

Local behavior:

- `Checker::alias_ty` expands aliases without predicates.
- An alias with a predicate becomes `Nominal::Refinement` and does not expand.

Imported behavior:

- `Checker::imported_ty` expands `SurfaceItem::Alias` and substitutes arguments.
- Imported `SurfaceItem::Refinement` stays an external nominal type.

World behavior:

- `World::of` calls `World::expand_aliases` so aliases in exported signatures are written out across modules.

See `crates/deed-typeck/src/check.rs`, `Checker::alias_ty` and `Checker::imported_ty`, and `crates/deed-typeck/src/surface.rs`, `World::of` and `World::expand_aliases`.

### 13. Imported preconditions keep source-module name roles

`where` clauses are exported with resolved identifier roles (`SurfaceRequires::names`). Imported preconditions are checked by reading those roles, not by resolving spans in the importing module. See `crates/deed-typeck/src/surface.rs`, `requires_of`, and `crates/deed-typeck/src/check.rs`, `Checker::imported_function_signature` and `Checker::clause_holds`.

## Contract and obligation rules

### 14. Contract clauses are typed in the callee and checked at call sites

In a function body:

- Each `where` clause must typecheck as `Bool`.
- Each `where` clause is added as a fact for body checking.
- Each `ensures` clause is typed as `Bool` with `result` bound per outcome.

See `crates/deed-typeck/src/check.rs`, `Checker::check_fn_against`.

At call sites:

- Preconditions are checked against caller facts (`Checker::check_preconditions`).
- Outcomes are recorded as tiers (`Proven` or `Guarded`) in `Types::push_precondition`.

See `crates/deed-typeck/src/check.rs`, `Checker::check_call_against`, `Checker::check_preconditions`, and `Checker::facts_for_call`.

### 15. Guarantees from `ensures` are `ok`-only for return promises

`promised_by` reads only `ok` clauses when building a function return guarantee. `err` clauses do not promise the value a successful call returns. See `crates/deed-typeck/src/check.rs`, `promised_by` and guarantee assembly in `Checker::collect`.

### 16. Row obligations are recorded, then discharged by the row-aware pass

When assigning to a concrete function type, the checker records the row required at that expression span (`Types::require_row`). It intersects multiple expectations at one span. This pass does not settle performed-row correctness itself. See `crates/deed-typeck/src/check.rs`, `Checker::assign_carrying`, and `crates/deed-typeck/src/ty.rs`, `Types::require_row`.

## Deliberately unspecified

The checker deliberately leaves these points to other passes or future design work:

1. The row of an inline closure (`FnRow::Inferred`) is not inferred by this pass. It is accepted here and settled by the row/effects pass. See `crates/deed-typeck/src/ty.rs`, `FnRow::Inferred`, and row checks in `Checker::compatible`.
2. Cross-module precondition calls are not resolved as callable declarations in the importing module. They are treated conservatively when evaluating imported clauses. See `crates/deed-typeck/src/check.rs`, `Checker::clause_holds` with `Origin::Elsewhere`.
3. Refinement predicate expressions are not exported as portable proof obligations. Only resulting guarantees and nominal identities cross the module boundary. See `crates/deed-typeck/src/surface.rs`, module docs and `requires_of`.
4. Imported `where`-clause reasoning is intentionally limited to parameter identities and `length`. Other names are not re-resolved on the importing side, so the outcome is conservatively thinned. See `crates/deed-typeck/src/surface.rs`, `SurfaceRequires` and `requires_of`, and `crates/deed-typeck/src/check.rs`, `Checker::clause_holds`.
5. Generic refinements are rejected instead of being given partial semantics. An alias with a predicate cannot take type parameters. See `crates/deed-typeck/src/check.rs`, alias collection in `Checker::collect`.
