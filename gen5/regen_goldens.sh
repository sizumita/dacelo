#!/bin/zsh
# Regenerate gen5/oracle golden files from a trusted checker.
# Usage: ./gen5/regen_goldens.sh [checker-binary, default ./gen5check]
# Requires the Gen0 reference: every golden is cross-checked (checker output
# must equal Gen0 --types output byte-for-byte, including exit code),
# so goldens permanently encode Gen0 agreement and later builds can run
# Rust-free (see GEN5_NO_RUST in gen5/build.sh).
set -e
cd "$(dirname "$0")/.."

CHECK="${1:-./gen5check}"
GEN0=./gen0-interp-rs/target/release/dacelo
[ -x $CHECK ] || { echo "missing checker $CHECK"; exit 1; }
[ -x $GEN0 ] || { echo "missing $GEN0: goldens can only be regenerated with the Gen0 reference present"; exit 1; }

n=0
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  g5=0; $CHECK check "$f" > /tmp/o_g5.log 2>&1 || g5=$?
  g0=0; $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || g0=$?
  if [ $g5 -ne $g0 ]; then echo "EXIT-DIFFER($f): checker=$g5 gen0=$g0, NOT writing goldens"; exit 1; fi
  if ! diff -q /tmp/o_g5.log /tmp/o_g0.log > /dev/null; then echo "MSG-DIFFER($f), NOT writing goldens"; exit 1; fi
  out="gen5/oracle/${f%.dc}.expected"
  mkdir -p "$(dirname "$out")"
  printf 'exit=%d\n' $g5 > "$out"
  cat /tmp/o_g5.log >> "$out"
  n=$((n+1))
done
echo "wrote $n goldens (checker+Gen0 agree on all)"
