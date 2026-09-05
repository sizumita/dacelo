#!/bin/zsh
# Gen 4 self-hosting verification:
#   dcc_1 (Gen3, dacelo-written compiler) builds gen4check (dacelo-written
#   Hindley-Milner type inference) -> gen4check agrees with Gen0 --types
#   on exit code AND stderr across examples + corpus.
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
DCC=./gen2-dcc-rs/target/release/dcc
DCC_SRC=gen3-dcc-dc/dcc_full_latest.dc
G4_SRC=gen4-infer-dc/g4_full.dc

cat gen3-dcc-dc/dcc.dc gen4-infer-dc/infer.dc gen4-infer-dc/g4_driver.dc > $G4_SRC
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
  echo "  AGREE($f, exit=$g0)"
  pass=$((pass+1))
}
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  check_one "$f"
done
echo "== Gen 4 self-hosting: $pass agree, $fail differ =="
[ $fail -eq 0 ]
