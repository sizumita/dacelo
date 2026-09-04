#!/bin/zsh
# Gen 3 self-hosting verification:
#   dcc (Gen2) compiles the dacelo-written compiler -> dcc_1
#   dcc_1 compiles every example -> run -> diff against reference interpreter
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
DCC=./gen2-dcc-rs/target/release/dcc
DCC_SRC=gen3-dcc-dc/dcc_full_latest.dc

$GEN0 $DCC_SRC --types > /dev/null && echo "typecheck OK"

$DCC $DCC_SRC -o dcc_1 > /dev/null
echo "dcc_1 built"

pass=0; fail=0
for f in hello fib list_ops closures tree; do
  ./dcc_1 examples/$f.dc ${f}_g3 > /dev/null 2>&1 || { echo "  $f: COMPILE FAIL"; fail=$((fail+1)); continue; }
  ./${f}_g3 > /tmp/g3_$f.out 2>&1 || { echo "  $f: RUNTIME FAIL"; fail=$((fail+1)); continue; }
  $GEN0 examples/$f.dc > /tmp/ref_$f.out 2>&1
  if diff -q /tmp/ref_$f.out /tmp/g3_$f.out > /dev/null; then
    echo "  $f: PASS"; pass=$((pass+1))
  else
    echo "  $f: OUTPUT MISMATCH"; fail=$((fail+1))
  fi
done
echo "== Gen 3 self-hosting: $pass passed, $fail failed =="
[ $fail -eq 0 ]
