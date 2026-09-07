#!/bin/zsh
# One-shot Gen5 bootstrap (Gen5-RC lineage): a seed Dacelo compiler builds
# gen5 once (stage1, runs on the seed's runtime), stage1 builds gen5 again
# (stage2: the first binaries that run on the RC runtime gen5/rt5.c), stage2
# self-checks both sources and rebuilds the compiler (stage3) for the
# assembly fixpoint. Delivered: ./gen5check + ./dcc_6 (= stage2, RC-compiled,
# fixpoint-verified).
#
# Rust-free: never invokes cargo/rustc/gen0-interp-rs/gen2-dcc-rs. Set
# GEN5_NO_RUST=1 to force golden-oracle mode even when Gen0 exists.
#   seed compiler : first present of ./dcc_7, ./dcc_6, ./dcc_1 (all Dacelo)
#   gate check    : existing ./gen5check, else Gen0 --types when available
#   oracle        : live Gen0 comparison when available, else gen5/oracle goldens
# Remaining non-Dacelo deps: zsh, system cc, gen5/rt5.c (plain C).
#
# Memory: the RC runtime frees garbage eagerly, so the self-check no longer
# needs the ~25GB that the old (never-collecting) mark-sweep runtime did; the
# stage1 binaries still run on the seed's runtime, which is why the
# self-check is done with the stage2 checker.
# Seed choice matters: an RC-compiled seed (./dcc_7 from test.sh C, or a
# promoted ./dcc_6) builds stage1 in ~5 s at ~1.7GB; the Gen3 seed ./dcc_1
# (no GC, quadratic line joining) peaks at 23-27GB on the 8,500-line
# compiler source -- run it alone, never two builds at once.
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
if [ -n "$GEN5_NO_RUST" ]; then USE_RUST=0
elif [ -x $GEN0 ]; then USE_RUST=1
else USE_RUST=0; fi

SEED=""
for c in ./dcc_7 ./dcc_6 ./dcc_1; do
  if [ -x $c ]; then SEED=$c; break; fi
done
[ -n "$SEED" ] || { echo "no Dacelo seed compiler (need one of ./dcc_7 ./dcc_6 ./dcc_1); bootstrap the Gen2/Gen3 chain first"; exit 1; }
echo "seed: $SEED (USE_RUST=$USE_RUST)"

S1C=/tmp/g5s1_check; S1D=/tmp/g5s1_dcc
S2C=/tmp/g5s2_check; S2D=/tmp/g5s2_dcc
S3D=/tmp/g5s3_dcc
G5CHECK_SRC=gen5/g5check_full.dc
G5CC_SRC=gen5/g5cc_full.dc

build_seed() {
  if [ $SEED = ./dcc_1 ]; then $SEED "$1" "$2" > /dev/null 2>&1
  else $SEED "$1" "$2" --backend-only > /dev/null 2>&1; fi
}

echo "== 1. stage1: seed builds gen5 once =="
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/g5_driver.dc gen5/g5_main.dc > $G5CHECK_SRC
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/g5_driver.dc gen5/g5_cg.dc gen5/g5_cgdriver.dc gen5/g5cc_driver.dc > $G5CC_SRC
# an RC-compiled checker prints an `rc:` stats line; a mark-sweep-built one
# (never collects) would need ~25GB for the 8,000-line gate and is skipped
is_rc_bin() { DACELO_RC_STATS=1 "$1" check examples/hello.dc 2>&1 | grep -q '^rc:'; }
if [ -x ./gen5check ] && is_rc_bin ./gen5check; then
  ./gen5check check $G5CHECK_SRC > /dev/null 2>&1 && echo "gate: existing gen5check (RC) accepts checker source" || { echo "gate FAIL (checker source)"; exit 1; }
  ./gen5check check $G5CC_SRC > /dev/null 2>&1 && echo "gate: existing gen5check (RC) accepts compiler source" || { echo "gate FAIL (compiler source)"; exit 1; }
elif [ -x ./gen5check ]; then
  echo "gate: existing gen5check is a mark-sweep build (would need ~25GB); skipped -- stage2 self-check is mandatory"
elif [ $USE_RUST -eq 1 ]; then
  $GEN0 $G5CHECK_SRC --types > /dev/null && echo "gate: Gen0 accepts checker source"
  $GEN0 $G5CC_SRC --types > /dev/null && echo "gate: Gen0 accepts compiler source"
else
  echo "gate: skipped (no checker, no reference) -- stage2 self-check still gates"
fi
build_seed $G5CHECK_SRC $S1C || { echo "stage1 checker build FAIL"; exit 1; }
build_seed $G5CC_SRC $S1D || { echo "stage1 dcc build FAIL"; exit 1; }
echo "stage1 built (runs on the seed's runtime)"

echo "== 2. stage2: stage1 builds gen5 (first RC-runtime binaries) =="
$S1D $G5CC_SRC $S2D --backend-only > /dev/null 2>&1 || { echo "  stage2 dcc FAIL"; exit 1; }
$S1D $G5CHECK_SRC $S2C --backend-only > /dev/null 2>&1 || { echo "  stage2 checker FAIL"; exit 1; }
echo "  stage2 dcc + checker built"

echo "== 3. self-check with the stage2 (RC) checker =="
$S2C check $G5CC_SRC > /dev/null 2>&1 || { echo "  self-check FAIL (compiler source)"; exit 1; }
echo "  stage2 accepts its own compiler source"
$S2C check $G5CHECK_SRC > /dev/null 2>&1 || { echo "  self-check FAIL (checker source)"; exit 1; }
echo "  stage2 accepts its own checker source"

echo "== 4. stage3 + fixpoint =="
$S2D $G5CC_SRC $S3D --backend-only > /dev/null 2>&1 || { echo "  stage3 dcc FAIL"; exit 1; }
cmp -s $S2D.s $S3D.s || { echo "  FIXPOINT FAIL: $S2D.s != $S3D.s"; exit 1; }
echo "  FIXPOINT: compiler .s identical"

echo "== 5. verify stage2 checker against the oracle =="
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
echo "  self-built checker oracle: $pass agree, $fail differ"
[ $fail -eq 0 ] || exit 1
$S2D examples/hello.dc /tmp/g5final_hello > /dev/null 2>&1 || { echo "  stage2 compile FAIL"; exit 1; }
/tmp/g5final_hello | grep -q "Hello, dacelo!" || { echo "  stage2 run FAIL"; exit 1; }
DACELO_RC_CHECK=1 /tmp/g5final_hello > /dev/null 2>&1 || { echo "  stage2 hello LEAK"; exit 1; }
$S2D gen5-examples/record.dc /tmp/g5final_rec > /dev/null 2>&1 || { echo "  stage2 record FAIL"; exit 1; }
[ "$(/tmp/g5final_rec)" = "Alice,Bob" ] || { echo "  stage2 record OUTPUT MISMATCH"; exit 1; }
DACELO_RC_CHECK=1 /tmp/g5final_rec > /dev/null 2>&1 || { echo "  stage2 record LEAK"; exit 1; }
echo "  stage2 smokes OK (hello + record, leak-free)"

echo "== 6. promote =="
cp $S2C ./gen5check
cp $S2D ./dcc_6
./gen5check check examples/hello.dc > /dev/null 2>&1 || { echo "  promoted checker FAIL"; exit 1; }
./dcc_6 examples/hello.dc /tmp/g5prom_hello > /dev/null 2>&1 || { echo "  promoted dcc FAIL"; exit 1; }
/tmp/g5prom_hello | grep -q "Hello, dacelo!" || { echo "  promoted run FAIL"; exit 1; }
echo " promoted ./gen5check + ./dcc_6 (RC-compiled) smoke OK"

echo "GEN5 BUILD COMPLETE (self-hosted RC checker + compiler, Rust-free)"
