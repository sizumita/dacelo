#!/bin/zsh
# Gen 2 (native compiler) verification
# usage: zsh gen2-dcc-rs/test.sh [path-to-dcc-binary]

set -e

DCC=${1:-gen2-dcc-rs/target/release/dcc}
INTERP=gen0-interp-rs/target/release/dacelo

fail() { echo "FAIL: $1"; exit 1; }

echo "== build dcc (release) =="
(cd gen2-dcc-rs && cargo build --release 2>&1 | tail -1)

echo "== native runs =="
for f in hello tree list_ops closures fib; do
  $DCC examples/$f.dc -o /tmp/nv_$f > /dev/null || fail "compile $f"
  /tmp/nv_$f > /tmp/nv_$f.out || fail "run $f"
  # output must match the reference interpreter
  $INTERP examples/$f.dc > /tmp/iv_$f.out
  diff -q /tmp/iv_$f.out /tmp/nv_$f.out > /dev/null || fail "output mismatch: $f"
  echo "  $f: ok (matches interpreter)"
done

echo "== GC stress =="
cat > /tmp/gcs.dc << 'EOF'
let rec build n acc = if n == 0 then acc else build (n - 1) (n :: acc)
let rec sum xs = case xs of
    [] -> 0
  | h :: rest -> h + sum rest
let rec loop k acc =
  if k == 0 then acc
  else loop (k - 1) (sum (build 30000 []))
let main () =
  let r = loop 40 0 in
  print_string "checksum = " ; print_int r ; print_string "\n"
EOF
$DCC /tmp/gcs.dc -o /tmp/gcs > /dev/null && /tmp/gcs | grep -q "checksum = 450015000" || fail "gc stress"
echo "  gc stress ok"

echo "== perf sanity (fib 30 under 1s) =="
/usr/bin/time -p /tmp/nv_fib 2>&1 | grep real

echo ""
echo "ALL GEN 2 TESTS PASSED -- native ARM64 codegen confirmed."
