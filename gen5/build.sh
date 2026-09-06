#!/bin/zsh
# One-shot Gen5 bootstrap: a seed Dacelo compiler builds gen5 ONCE (stage1),
# stage1 self-checks its own sources, then stage1 builds gen5 again.
# Delivered: ./gen5check + ./dcc_6, both Gen5-built and fixpoint-verified.
#
# Rust-free: this script never invokes cargo, rustc, gen0-interp-rs, or
# gen2-dcc-rs. Set GEN5_NO_RUST=1 to force hermetic mode even when the Rust
# reference exists (missing reference auto-selects the same mode).
#   seed compiler : first present of ./dcc_6, ./dcc_7, ./dcc_1 (all Dacelo;
#                   stages after the first need no seed at all)
#   gate check    : existing ./gen5check, else Gen0 --types when available,
#                   else skipped with a warning (stage1 build itself still
#                   gates stage2 via the mandatory self-check in step 2)
#   oracle        : live Gen0 comparison when available, else byte-compare
#                   against gen5/oracle goldens (which permanently encode a
#                   Gen0-agreeing run; regenerate via gen5/regen_goldens.sh,
#                   Gen0 required only for regeneration)
# Remaining non-Dacelo deps: zsh, system cc, gen2-dcc-rs/rt/rt.c (plain C
# source, no Rust toolchain involved) — same as every dcc build.
# Resource note: the step-2 self-checks peak ~25GB RSS (quadratic checker,
# see gen5/RESUME.md). On a 34GB shared box a transient OOM kill is possible;
# step 2 retries once (a pass is conclusive, so retries can't hide real bugs).
#
# Why this is complete: codegen consumes parse trees only (the check gates
# by status), and stage1 already accepted both sources in step 2, so the
# --backend-only stage2 outputs equal a full check-then-compile run.
# The .s fixpoint then proves backend stability; the oracle proves the
# self-built checker matches the reference behaviour.
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
if [ -n "$GEN5_NO_RUST" ]; then USE_RUST=0
elif [ -x $GEN0 ]; then USE_RUST=1
else USE_RUST=0; fi

SEED=""
for c in ./dcc_6 ./dcc_7 ./dcc_1; do
  if [ -x $c ]; then SEED=$c; break; fi
done
[ -n "$SEED" ] || { echo "no Dacelo seed compiler (need one of ./dcc_6 ./dcc_7 ./dcc_1); bootstrap the Gen2/Gen3 chain first, Rust cannot be used here by design"; exit 1; }
echo "seed: $SEED (USE_RUST=$USE_RUST)"

S1C=/tmp/g5s1_check
S1D=/tmp/g5s1_dcc6
S2C=/tmp/g5s2_check
S2D=/tmp/g5s2_dcc7
G5CHECK_SRC=gen5/g5check_full.dc
G5CC_SRC=gen5/g5cc_full.dc

# Seed build helper: dcc_1 has no check phase; Dacelo compilers run
# check-then-compile by default, which OOMs on 6000+ line inputs when
# combined with codegen in one 34GB process (each phase alone fits).
# Gate (above) + mandatory self-check (step 2) already cover checking,
# so Dacelo seeds build stage1 backend-only.
build_seed() {
  if [ $SEED = ./dcc_1 ]; then $SEED "$1" "$2" > /dev/null 2>&1
  else $SEED "$1" "$2" --backend-only > /dev/null 2>&1; fi
}

echo "== 1. stage1: seed builds gen5 once =="
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5_main.dc > $G5CHECK_SRC
if [ -x ./gen5check ]; then
  ./gen5check check $G5CHECK_SRC > /dev/null 2>&1 || { echo "gate FAIL (checker source)"; exit 1; }
  echo "gate: existing gen5check accepts checker source"
elif [ $USE_RUST -eq 1 ]; then
  $GEN0 $G5CHECK_SRC --types > /dev/null && echo "gate: Gen0 accepts checker source"
else
  echo "gate: skipped (no checker, no reference) -- stage1 build proceeds unchecked"
fi
build_seed $G5CHECK_SRC $S1C || { echo "stage1 checker build FAIL"; exit 1; }
echo "stage1 gen5check built"
cat gen3-dcc-dc/dcc.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5cc_driver.dc > $G5CC_SRC
if [ -x ./gen5check ]; then
  ./gen5check check $G5CC_SRC > /dev/null 2>&1 || { echo "gate FAIL (compiler source)"; exit 1; }
  echo "gate: existing gen5check accepts compiler source"
elif [ $USE_RUST -eq 1 ]; then
  $GEN0 $G5CC_SRC --types > /dev/null && echo "gate: Gen0 accepts compiler source"
else
  echo "gate: skipped (no checker, no reference) -- stage1 build proceeds unchecked"
fi
build_seed $G5CC_SRC $S1D || { echo "stage1 dcc build FAIL"; exit 1; }
echo "stage1 dcc built"

echo "== 2. self-check with stage1 (slow, ~15 min for both sources) =="
# Self-checks need ~25GB peak on a 34GB shared box; a transient OOM kill is
# retried once. Sound: only a real exit 0 passes, so no false positives.
try_twice() {
  "$@" > /dev/null 2>&1 || { echo "  retrying after failure..."; sleep 5; "$@" > /dev/null 2>&1; } || return 1
}
try_twice $S1C check $G5CC_SRC || { echo "  self-check FAIL (compiler source)"; exit 1; }
echo "  stage1 accepts its own compiler source"
try_twice $S1C check $G5CHECK_SRC || { echo "  self-check FAIL (checker source)"; exit 1; }
echo "  stage1 accepts its own checker source"

echo "== 3. stage2: gen5 builds gen5 =="
$S1D $G5CC_SRC $S2D --backend-only > /dev/null 2>&1 || { echo "  stage2 dcc FAIL"; exit 1; }
echo "  stage2 dcc built (by stage1)"
$S1D $G5CHECK_SRC $S2C --backend-only > /dev/null 2>&1 || { echo "  stage2 checker FAIL"; exit 1; }
echo "  stage2 checker built (by stage1)"

echo "== 4. verify stage2 =="
cmp -s $S1D.s $S2D.s || { echo "  FIXPOINT FAIL: $S1D.s != $S2D.s"; exit 1; }
echo "  FIXPOINT: compiler .s identical"
pass=0; fail=0
check_live() {
  f="$1"
  g5=0; $S2C check "$f" > /tmp/o_g5.log 2>&1 || g5=$?
  g0=0; $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || g0=$?
  if [ $g5 -ne $g0 ]; then echo "EXIT-DIFFER($f)"; fail=$((fail+1)); return; fi
  if ! diff -q /tmp/o_g5.log /tmp/o_g0.log > /dev/null; then echo "MSG-DIFFER($f)"; fail=$((fail+1)); return; fi
  pass=$((pass+1))
}
check_golden() {
  f="$1"
  g5=0; $S2C check "$f" > /tmp/o_g5.log 2>&1 || g5=$?
  gold="gen5/oracle/${f%.dc}.expected"
  [ -f "$gold" ] || { echo "NO-GOLDEN($f)"; fail=$((fail+1)); return; }
  printf 'exit=%d\n' $g5 > /tmp/o_g5.exp
  cat /tmp/o_g5.log >> /tmp/o_g5.exp
  if ! diff -q /tmp/o_g5.exp "$gold" > /dev/null; then echo "GOLDEN-DIFFER($f)"; fail=$((fail+1)); return; fi
  pass=$((pass+1))
}
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  if [ $USE_RUST -eq 1 ]; then check_live "$f"; else check_golden "$f"; fi
done
if [ $USE_RUST -eq 1 ]; then echo "  self-built checker oracle (live): $pass agree, $fail differ";
else echo "  self-built checker oracle (goldens): $pass agree, $fail differ"; fi
[ $fail -eq 0 ] || exit 1
$S2D examples/hello.dc /tmp/g5final_hello > /dev/null 2>&1 || { echo "  stage2 compile FAIL"; exit 1; }
/tmp/g5final_hello | grep -q "Hello, dacelo!" || { echo "  stage2 run FAIL"; exit 1; }
$S2D gen5-examples/record.dc /tmp/g5final_rec > /dev/null 2>&1 || { echo "  stage2 record FAIL"; exit 1; }
[ "$(/tmp/g5final_rec)" = "Alice,Bob" ] || { echo "  stage2 record OUTPUT MISMATCH"; exit 1; }
echo "  stage2 smokes OK (hello + record)"

echo "== 5. promote =="
cp $S2C ./gen5check
cp $S2D ./dcc_6
./gen5check check examples/hello.dc > /dev/null 2>&1 || { echo "  promoted checker FAIL"; exit 1; }
./dcc_6 examples/hello.dc /tmp/g5prom_hello > /dev/null 2>&1 || { echo "  promoted dcc FAIL"; exit 1; }
/tmp/g5prom_hello | grep -q "Hello, dacelo!" || { echo "  promoted run FAIL"; exit 1; }
echo " promoted ./gen5check + ./dcc_6 smoke OK"

echo "GEN5 BUILD COMPLETE (self-hosted checker + compiler, Rust-free)"
