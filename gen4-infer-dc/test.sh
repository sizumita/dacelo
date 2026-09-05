#!/bin/zsh
# Gen 4 verification (all steps build on each other):
#   A. checker: dcc_1 builds gen4check; 38-case oracle vs Gen0 --types.
#   B. compiler: dcc_1 builds dcc_4 (check+codegen); .s identical to dcc_1,
#      examples run OK, ill-typed inputs rejected with Gen0 messages.
#   C. self-build (Gen4 builds Gen4): dcc_4 builds dcc_5; .s fixpoint,
#      examples run OK, rejection still works.
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
DCC=./gen2-dcc-rs/target/release/dcc
DCC_SRC=gen3-dcc-dc/dcc_full_latest.dc
G4_SRC=gen4-infer-dc/g4_full.dc
G4CC_SRC=gen4-infer-dc/g4cc_full.dc

echo "== A. checker =="
cat gen3-dcc-dc/dcc.dc gen4-infer-dc/infer.dc gen4-infer-dc/g4_check.dc gen4-infer-dc/g4_main.dc > $G4_SRC
$GEN0 $G4_SRC --types > /dev/null && echo "gen4 typecheck OK"

if [ ! -x ./dcc_1 ]; then
  $DCC $DCC_SRC -o dcc_1 > /dev/null
  echo "dcc_1 built"
fi
./dcc_1 $G4_SRC gen4check > /dev/null
echo "gen4check built (by dcc_1)"

pass=0; fail=0
check_one() {
  f="$1"
  g4=0; ./gen4check "$f" > /tmp/o_g4.log 2>&1 || g4=$?
  g0=0; $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || g0=$?
  if [ $g4 -ne $g0 ]; then
    echo "EXIT-DIFFER($f): g4=$g4 g0=$g0"; echo "  g4: $(cat /tmp/o_g4.log)"; echo "  g0: $(cat /tmp/o_g0.log)"; fail=$((fail+1)); return
  fi
  if ! diff -q /tmp/o_g4.log /tmp/o_g0.log > /dev/null; then
    echo "MSG-DIFFER($f):"; echo "  g4: $(cat /tmp/o_g4.log)"; echo "  g0: $(cat /tmp/o_g0.log)"; fail=$((fail+1)); return
  fi
  pass=$((pass+1))
}
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  check_one "$f"
done
echo "checker oracle: $pass agree, $fail differ"
[ $fail -eq 0 ]

echo "== B. compiler (dcc_4) =="
cat gen3-dcc-dc/dcc.dc gen4-infer-dc/infer.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen4-infer-dc/g4_check.dc gen4-infer-dc/g4cc_driver.dc > $G4CC_SRC
$GEN0 $G4CC_SRC --types > /dev/null && echo "g4cc typecheck OK"
./dcc_1 $G4CC_SRC dcc_4 > /dev/null
echo "dcc_4 built (by dcc_1)"
for f in hello fib list_ops closures tree; do
  ./dcc_4 examples/$f.dc /tmp/g4_$f > /dev/null 2>&1 || { echo "  $f: COMPILE FAIL"; exit 1; }
  ./dcc_1 examples/$f.dc /tmp/g1_$f > /dev/null 2>&1
  diff -q /tmp/g4_$f.s /tmp/g1_$f.s > /dev/null || { echo "  $f: .s DIFFERS from dcc_1"; exit 1; }
  /tmp/g4_$f > /tmp/g4_$f.out 2>&1 || { echo "  $f: RUNTIME FAIL"; exit 1; }
  $GEN0 examples/$f.dc > /tmp/ref_$f.out 2>&1
  diff -q /tmp/ref_$f.out /tmp/g4_$f.out > /dev/null || { echo "  $f: OUTPUT MISMATCH"; exit 1; }
  echo "  $f: .s identical, runs OK"
done
for f in gen4-infer-dc/tests/t_bad.dc gen4-infer-dc/tests/unbound.dc gen4-infer-dc/tests/occurs.dc; do
  ./dcc_4 "$f" /tmp/g4rej > /tmp/o_g4.log 2>&1 && { echo "  REJECT-FAIL($f): compiled ill-typed code"; exit 1; }
  $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || true
  diff -q /tmp/o_g4.log /tmp/o_g0.log > /dev/null || { echo "  REJECT-MSG-DIFFER($f)"; exit 1; }
  echo "  reject OK($f)"
done

echo "== C. self-build (dcc_4 builds dcc_5) =="
./dcc_4 $G4CC_SRC dcc_5 > /dev/null
echo "dcc_5 built (by dcc_4)"
diff -q dcc_4.s dcc_5.s > /dev/null || { echo "  FIXPOINT FAIL: dcc_4.s != dcc_5.s"; exit 1; }
echo "fixpoint: dcc_4.s == dcc_5.s"
./dcc_5 examples/fib.dc /tmp/g5_fib > /dev/null 2>&1
/tmp/g5_fib > /tmp/g5_fib.out 2>&1
diff -q /tmp/ref_fib.out /tmp/g5_fib.out > /dev/null || { echo "  dcc_5 fib MISMATCH"; exit 1; }
./dcc_5 gen4-infer-dc/tests/t_bad.dc /tmp/g5rej > /dev/null 2>&1 && { echo "  dcc_5 REJECT-FAIL"; exit 1; }
echo "dcc_5 runs fib OK, rejects ill-typed OK"

echo "== Gen 4 complete: checker 38/38, compiler identical, self-build fixpoint =="
