# Gen4: dacelo-written Hindley-Milner type inference — work log

## Goal
Gen3 (`dcc`) generates code but performs NO type inference. Gen4 adds a
complete HM type checker written in dacelo itself, built via Gen3
(`dcc_1`), that agrees with the Gen0 (Rust) reference on exit code AND
stderr.

## Layout
- `infer.dc` (~670 lines): pure explicit-substitution Algorithm W.
  `Ty = TVI Int | TC String | TAr Ty Ty | TTup (List Ty) | TApp String (List Ty)`,
  `Sch = Sch (List Int) Ty`, state `TIS subst ctr appn ectx tydefs` threaded
  functionally (mirrors `gen0-interp-rs/src/infer.rs`, including `and`-group
  mutual recursion, annotations, occurs check, let-polymorphism).
- `g4_driver.dc`: parse file, check items in source order, report first error
  as `dacelo: [item N: kind] msg` (exit 1) or silent success (exit 0) —
  mirrors Gen0 `print_types`.
- `tests/` (33 files): ADT/polymorphism/annotations/occurs-check/shadowing/
  higher-order + negative cases (exact stderr compared).
- `test.sh`: typechecks sources via Gen0, builds `dcc_1` (Gen2) if missing,
  builds `gen4check` via `dcc_1`, runs the 38-case oracle.
- `g4_full.dc`: generated concatenation
  (`dcc.dc + infer.dc + g4_driver.dc`, gitignored, rebuilt by test.sh).

## Gen3 frontend extensions (parser keeps types for inference)
`gen3-dcc-dc/dcc.dc` now retains what the backend used to drop:
`TyAst` (`TTVar/TTCon/TTArr/TTTup`), `EAnn`, `TyDecl4`/`CtorDecl4`,
`ProgItem` + 4-tuple `Prog` (ctor pairs, groups, tydecls, source ORDER —
order matters: use-before-def is an error, like Gen0).
Backend compatibility: `strip_lams`/`ce`/`fv_expr` erase `EAnn`;
driver patterns take the 4-tuple. Gen3 fixpoint re-verified
(`dcc_2.s == dcc_3.s`, examples 5/5).

## Message parity
Error strings mirror Gen0 exactly (unify mismatch, occurs, binop sites via
REPLACE wrappers, `in application #N`, `case branch N`, `in def`, `in`,
`annotation on`, if/seq/case wrappers). Custom `em` param on `ti_unify`
+ `ectx` prefix stack (`ti_push_ctx`/`ti_unify_wrap` with save/restore).

## Bugs found while building (all fixed)
1. **Nested-`case` absorption (parser)**: an inner multi-branch `case` as a
   branch body absorbs the outer's following `|` branches
   (`t_case.dc`: `(B 1 2)` fell through). The 25-function parser group in
   `dcc.dc` itself relies on this shape. Fix: parenthesize nested
   multi-branch cases in `infer.dc`/`g4_driver.dc` (3 sites:
   `ti_unify_ap`/`ti_unify_struct`/`ti_unify_lists`, plus `ti_pat_each`,
   `ti_show` arrow case, `TTCon "List"` case). Lesson: never write an
   unparenthesized multi-branch `case` as a branch body.
2. **Stale-subst aliasing (`ti_walk`)**: nested chase reused the outer `s`
   instead of the residual list → `walk8=V9` instead of `V5`, causing bogus
   occurs failures (`mem_str`: `cannot unify a with a`). Fix: `ti_walk_full`
   threads BOTH the original (for values) and residual (for search).
3. **Missing generalization normalization**: storing raw types without
   applying subst let one use-site bind a shared var (`rev_acc` double-use:
   `expected String, found Int`). Fix: generalize stores
   chase-normalized types; instantiate chases during rewrite.
4. **`set -e` vs expected-failure tests**: guard checker calls
   (`g4=0; run || g4=$?`).

## Performance (the hard part)
Naive explicit-subst W was super-quadratic (pfx80: 2.8s/1.8GB, full file
OOM at 15GB+): every op chased through ever-growing history, all retained.
Fix — **normalize + prune at item boundaries** (`ti_norm_env` +
real `tis_clear_subst` in `g4_run_ty`/`g4_run_grp`): sound because
top-level schemes are closed (every var in a stored ty is quantified;
later binds only touch fresh ids, and member-placeholder links are
resolved by normalization before pruning). Result: pfx80 0.27s/218MB,
pfx160 0.60s/486MB, full `dcc.dc` (278 items) 7.6s, self (`g4_full.dc`,
340 items) 12s. Naive per-item pruning WITHOUT normalization is unsound
(false `expected Expr, found Pat`) — normalization is the load-bearing step.

## Verification
- `zsh gen4-infer-dc/test.sh`: **38 agree, 0 differ** (exit code + stderr).
- Self-checks agree with Gen0: `dcc.dc` (exit 0, 7.6s), `g4_full.dc`
  (exit 0, 12s). Peak RSS ~5–9GB on thousand-line files (known limitation:
  explicit-subst + runtime block overhead; typical programs run in ms/MBs).
- Gen3 chain re-verified after frontend extensions: typecheck OK,
  examples 5/5, `dcc_2.s == dcc_3.s` (0 diff), `t_overwrite` 200 400.

## Gen4 builds Gen4 (check+codegen integration)
`gen4check` checks but cannot emit binaries. Integration (`g4cc_driver.dc`):
check-then-compile with the exact `dcc_1` CLI.
- Reuse without touching Gen3: `main` moved out of `g3_driver_v2.dc`
  into `g3_main.dc` (pure move; 5-file concat is byte-identical to the old
  4-file concat — Gen3 chain re-verified green). Same split for the Gen4
  driver (`g4_check.dc` + `g4_main.dc`).
- `g4cc_full.dc` = frontend + `infer.dc` + backend + `g4_check.dc` +
  `g4cc_driver.dc` (3179 lines, no name clashes, Gen0 typecheck clean).
- `dcc_1` builds `dcc_4`: `.s` byte-identical to `dcc_1` on all 5 examples
  (same `emit_code`), all run OK; `t_bad`/`unbound`/`occurs` rejected with
  Gen0-identical messages, no output files written.
- `dcc_4` builds `dcc_5` from the same source: `dcc_4.s == dcc_5.s`
  (fixpoint), `dcc_5` runs fib OK and still rejects ill-typed.
- `zsh gen4-infer-dc/test.sh` runs A (checker oracle) + B (compiler
  identity/reject) + C (self-build fixpoint) end to end, exit 0.
