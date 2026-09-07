# Gen5 examples (language version `dacelo-gen5/1`)

Small syntax collection, one per construct (RFC §9: LLM mixing other languages
can retrieve the matching example from diagnostics).

| File | Construct | Try |
|---|---|---|
| `let_case_adt.dc` | `let` / `case-of` / ADT + patterns (core HM, unchanged) | `./gen5check check gen5-examples/let_case_adt.dc` |
| `record.dc` | immutable row-polymorphic records `{…}`, `r.f` | `./dcc_6 gen5-examples/record.dc /tmp/r && /tmp/r` → `Alice,Bob` |
| `record_with.dc` | same-type `{s with f = …}` update, executed natively | `./dcc_6 gen5-examples/record_with.dc /tmp/rw && /tmp/rw` → `2` |
| `module.dc` | `module M exposing (…)` (file-unit, no cycles) | `./gen5check interface gen5-examples/module.dc` |
| `Mymod.dc` + `use_mymod.dc` | `import Mymod` (explicit, sibling `Mymod.dc`) | `./dcc_6 gen5-examples/use_mymod.dc /tmp/um && /tmp/um` → `42` |
| `sig.dc` | `sig f : forall a. …` rigid contracts (optional, pass) | `./gen5check check gen5-examples/sig.dc` |
| `sig_wrong.dc` | false `forall` contract (must reject, exit 1) | `./gen5check check gen5-examples/sig_wrong.dc` |
| `annotation.dc` | flexible `e : T` (legacy meaning, distinct from `sig`) | `./gen5check check gen5-examples/annotation.dc` |
| `hole.dc` | static holes `?name` (`partial`, exit 2) | `./gen5check holes gen5-examples/hole.dc` |
| `hole_filled.dc` | `hole.dc` with `?body` filled (`checked`, exit 0) | `./gen5check check gen5-examples/hole_filled.dc` |
| `unicode_span.dc` | CJK comments/strings: UTF-8 byte spans vs scalar columns | `./gen5check focus gen5-examples/unicode_span.dc --at=11:5` |
| `inplace_map.dc` | Gen5-RC: inferred ownership + drop-guided reuse (functional but in-place) | `./dcc_6 gen5-examples/inplace_map.dc /tmp/im && DACELO_RC_STATS=1 /tmp/im` → `5050`, `reuses=100` |

Query entry points (all share one Typed IR; byte budgets via `--max-bytes`;
implemented by `gen5check`, execution by `dcc_6` check-then-compile):

```sh
./gen5check check gen5-examples/record.dc --format=json
./gen5check types gen5-examples/record.dc --format=json
./gen5check focus gen5-examples/record.dc --at=15:20 --max-bytes=16000
./gen5check why gen5-examples/record.dc --at=15:20 --format=json
./gen5check holes gen5-examples/hole.dc --format=json
./gen5check interface gen5-examples/module.dc --format=json
./gen5check format gen5-examples/record.dc
./dcc_6 gen5-examples/record.dc /tmp/prog && /tmp/prog
```

Notes:

- Records: unique labels, order-independent identity, open rows only via
  inference (`{ name : a | r } -> a`). Initial `with` is same-type update of an
  existing field (no add/remove/type-change). Nominal ADTs stay for
  semantically distinct wrappers (`UserId` vs `OrderId`).
- Modules: single file-unit, explicit `import`/`exposing`, acyclic. `import`
  is unqualified in v1 (exposed names enter scope); `interface` publishes only
  exposed, hole-free schemes with source vs semantic hashes separated.
- `sig` uses rigid (skolem) checking; `e : T` stays flexible. `sig` never
  enables polymorphic recursion by itself (rec placeholders stay monomorphic).
  `types` returns both `scheme` (principal) and `contract` (null or text);
  `.dci` publishes the contract when present. Module importers in v1 resolve
  against the principal closed scheme (sound; contract narrowing for callers
  is future work).
- Holes make the program `partial`: `check` reports `partial` (exit 2 text),
  `holes` gives expected types + in-scope candidates, and neither executables
  nor trusted interfaces are produced until filled. Filling re-checks with
  plain W (accepted set unchanged).
- Match exhaustiveness/redundancy are advisory warnings (not rejections) in v1.
- No persistent cache in v1 (always clean); snapshots carry source/dependency
  hashes so future incremental results must equal clean results to be `checked`.
- Memory management (Gen5-RC): precise reference counting with inferred
  borrowed parameters and drop-guided reuse (see `gen5/GC_DESIGN.md`).
  `gen5check types --format=json` shows the inferred `ownership` of every
  direct-callable function; `DACELO_RC_CHECK=1` verifies leak-freedom at
  exit, `DACELO_RC_STATS=1` prints alloc/free/reuse counters.
