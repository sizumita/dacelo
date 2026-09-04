#!/bin/zsh
# Gen 1 self-hosting verification
# usage: zsh gen1-interp-dc/test.sh [path-to-dacelo-binary]

set -e

BIN=${1:-gen0-interp-rs/target/release/dacelo}
INTERP=gen1-interp-dc/interp.dc

fail() { echo "FAIL: $1"; exit 1; }

echo "== Gen 0 sanity =="
$BIN examples/hello.dc | grep -q "Hello, dacelo!" || fail "gen0 hello"

echo "== Gen 1: single stage (dacelo interp.dc prog.dc) =="
for f in hello tree list_ops closures; do
  $BIN $INTERP examples/$f.dc > /tmp/out_$f || fail "single stage $f"
  echo "  $f.dc ok"
done

echo "== Gen 1: fib(20) only =="
echo 'let rec fib n = if n < 2 then n else fib (n - 1) + fib (n - 2)
let main () =
  print_string "fib 20 = " ; print_int (fib 20) ; print_string "\n"' > /tmp/fib20.dc
$BIN $INTERP /tmp/fib20.dc | grep -q "fib 20 = 6765" || fail "interp fib"

echo "== Gen 1: double stage (dacelo interp.dc interp.dc prog.dc) =="
$BIN $INTERP $INTERP examples/hello.dc > /tmp/dd_hello || fail "double hello"
grep -q "Hello, dacelo!" /tmp/dd_hello || fail "double hello output"
echo "  hello.dc ok"

for f in closures; do
  $BIN $INTERP $INTERP examples/$f.dc > /tmp/dd_$f || fail "double stage $f"
  diff <($BIN examples/$f.dc) /tmp/dd_$f > /dev/null || fail "double vs direct mismatch: $f"
  echo "  $f.dc ok"
done

echo ""
echo "ALL GEN 1 TESTS PASSED -- language-level self-hosting confirmed."
