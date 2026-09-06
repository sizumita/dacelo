#!/bin/zsh
# Gen5 verification (all steps build on each other):
#   A. checker: dcc_1 builds gen5check; 38-case oracle vs Gen0 --types.
#   B. compiler: dcc_1 builds dcc_6 (check+codegen); legacy .s identical
#      to dcc_1, Gen5 features (record/with/module/sig/hole) behave,
#      ill-typed inputs rejected with Gen0-comparable messages.
#   C. self-build (Gen5 builds Gen5): gen5check checks g5cc_full clean,
#      then dcc_6 --backend-only builds dcc_7; .s fixpoint
#      (dcc_6.s == dcc_7.s); dcc_7 smokes OK.
#      NOTE on --backend-only: codegen consumes parse trees only (the
#      check gates by status), so backend-only output is identical to a
#      full run whenever the check passes. The split exists because a
#      combined check+codegen process exceeds this machine's 34GB RAM
#      on the 7000-line self-host input (each phase alone fits).
#   D. formatter: every Gen3/Gen4/Gen5 source round-trips (exit 0).
#   E. RFC acceptance: sig/record/hole/module negatives, focus containing +
#      contract fields, interface diff, match warnings, shadowing, Japanese
#      bytes, byte budgets (all small files, fast).
set -e
cd "$(dirname "$0")/.."

GEN0=./gen0-interp-rs/target/release/dacelo
G5CHECK_SRC=gen5/g5check_full.dc
G5CC_SRC=gen5/g5cc_full.dc

echo "== A. checker =="
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5_main.dc > $G5CHECK_SRC
$GEN0 $G5CHECK_SRC --types > /dev/null && echo "gen5 typecheck OK"

if [ ! -x ./dcc_1 ]; then
  echo "dcc_1 missing: build it first (see gen3-dcc-dc/test.sh)"; exit 1
fi
./dcc_1 $G5CHECK_SRC gen5check > /dev/null
echo "gen5check built (by dcc_1)"

pass=0; fail=0
check_one() {
  f="$1"
  g5=0; ./gen5check check "$f" > /tmp/o_g5.log 2>&1 || g5=$?
  g0=0; $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || g0=$?
  if [ $g5 -ne $g0 ]; then
    echo "EXIT-DIFFER($f): g5=$g5 g0=$g0"; echo "  g5: $(cat /tmp/o_g5.log)"; echo "  g0: $(cat /tmp/o_g0.log)"; fail=$((fail+1)); return
  fi
  if ! diff -q /tmp/o_g5.log /tmp/o_g0.log > /dev/null; then
    echo "MSG-DIFFER($f):"; echo "  g5: $(cat /tmp/o_g5.log)"; echo "  g0: $(cat /tmp/o_g0.log)"; fail=$((fail+1)); return
  fi
  pass=$((pass+1))
}
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  check_one "$f"
done
echo "checker oracle: $pass agree, $fail differ"
[ $fail -eq 0 ]

echo "== B. compiler (dcc_6) =="
cat gen3-dcc-dc/dcc.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5cc_driver.dc > $G5CC_SRC
$GEN0 $G5CC_SRC --types > /dev/null && echo "g5cc typecheck OK"
./dcc_1 $G5CC_SRC dcc_6 > /dev/null
echo "dcc_6 built (by dcc_1)"
for f in hello fib list_ops closures tree gc_stress; do
  ./dcc_6 examples/$f.dc /tmp/g6_$f > /dev/null 2>&1 || { echo "  $f: COMPILE FAIL"; exit 1; }
  ./dcc_1 examples/$f.dc /tmp/g1_$f > /dev/null 2>&1
  diff -q /tmp/g6_$f.s /tmp/g1_$f.s > /dev/null || { echo "  $f: .s DIFFERS from dcc_1"; exit 1; }
  /tmp/g6_$f > /tmp/g6_$f.out 2>&1 || { echo "  $f: RUNTIME FAIL"; exit 1; }
  $GEN0 examples/$f.dc > /tmp/ref_$f.out 2>&1
  diff -q /tmp/ref_$f.out /tmp/g6_$f.out > /dev/null || { echo "  $f: OUTPUT MISMATCH"; exit 1; }
  echo "  $f: .s identical, runs OK"
done
./dcc_6 gen5-examples/record.dc /tmp/g6_record > /dev/null 2>&1 || { echo "  record: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g6_record)" = "Alice,Bob" ] || { echo "  record: OUTPUT MISMATCH"; exit 1; }
echo "  record: runs OK (Alice,Bob)"
./dcc_6 gen5-examples/module.dc /tmp/g6_module > /dev/null 2>&1 || { echo "  module: COMPILE FAIL"; exit 1; }
echo "  module: links OK"
./gen5check check gen5-examples/sig.dc > /dev/null 2>&1 || { echo "  sig: CHECK FAIL"; exit 1; }
echo "  sig: checks OK"
set +e
./gen5check check gen5-examples/sig_wrong.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  sig_wrong: expected reject (exit 1)"; exit 1; }
set -e
echo "  sig_wrong: rejected OK"
./dcc_6 gen5-examples/record_with.dc /tmp/g6_recw > /dev/null 2>&1 || { echo "  record_with: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g6_recw)" = "2" ] || { echo "  record_with: OUTPUT MISMATCH"; exit 1; }
echo "  record_with: runs OK (2)"
./dcc_6 gen5-examples/use_mymod.dc /tmp/g6_usemod > /dev/null 2>&1 || { echo "  use_mymod: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g6_usemod)" = "42" ] || { echo "  use_mymod: OUTPUT MISMATCH"; exit 1; }
echo "  use_mymod: runs OK (42)"
./gen5check check gen5-examples/hole_filled.dc > /dev/null 2>&1 || { echo "  hole_filled: CHECK FAIL"; exit 1; }
echo "  hole_filled: checks OK (fill re-checks with plain W)"
set +e
./gen5check check gen5-examples/hole.dc > /dev/null 2>&1
[ $? -eq 2 ] || { echo "  hole: expected partial (exit 2)"; exit 1; }
set -e
echo "  hole: partial OK"
./dcc_6 gen4-infer-dc/tests/t_bad.dc /tmp/g6rej > /tmp/o_g6.log 2>&1 && { echo "  REJECT-FAIL(t_bad): compiled ill-typed code"; exit 1; }
$GEN0 gen4-infer-dc/tests/t_bad.dc --types > /tmp/o_g0.log 2>&1 || true
diff -q /tmp/o_g6.log /tmp/o_g0.log > /dev/null || { echo "  REJECT-MSG-DIFFER(t_bad)"; exit 1; }
echo "  reject OK (t_bad, Gen0-comparable message)"

echo "== C. self-build fixpoint (dcc_7) =="
./gen5check check $G5CC_SRC > /dev/null 2>&1 || { echo "  self-check FAIL"; exit 1; }
echo "  gen5check accepts its own compiler source (exit 0)"
./dcc_6 $G5CC_SRC dcc_7 --backend-only > /dev/null 2>&1 || { echo "  dcc_7 BUILD FAIL"; exit 1; }
diff -q dcc_6.s dcc_7.s > /dev/null || { echo "  FIXPOINT FAIL: dcc_6.s != dcc_7.s"; exit 1; }
echo "  FIXPOINT: dcc_6.s == dcc_7.s"
./dcc_7 examples/hello.dc /tmp/g7_hello > /dev/null 2>&1 || { echo "  dcc_7 SMOKE FAIL"; exit 1; }
/tmp/g7_hello > /tmp/g7_hello.out 2>&1
$GEN0 examples/hello.dc > /tmp/ref_hello.out 2>&1
diff -q /tmp/ref_hello.out /tmp/g7_hello.out > /dev/null || { echo "  dcc_7 OUTPUT MISMATCH"; exit 1; }
echo "  dcc_7 smokes OK"

echo "== D. formatter round-trip =="
for f in gen3-dcc-dc/dcc.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_driver.dc gen5/g5cc_driver.dc gen5/g5_main.dc gen4-infer-dc/infer.dc gen4-infer-dc/g4_check.dc gen4-infer-dc/g4_main.dc gen4-infer-dc/g4cc_driver.dc; do
  ./gen5check format "$f" > /dev/null 2>&1 || { echo "  FORMAT FAIL: $f"; exit 1; }
done
for f in gen5-examples/*.dc; do
  ./gen5check format "$f" > /dev/null 2>&1 || { echo "  FORMAT FAIL: $f"; exit 1; }
done
echo "  all sources format-clean"

echo "== E. RFC acceptance (queries, negatives, i18n, budgets) =="
cat > /tmp/g5e_id.dc <<'EOF'
let id x = x
let v = id 1
EOF
./gen5check focus /tmp/g5e_id.dc --at=2:9 --format=json > /tmp/g5e_focus.json 2>&1 || { echo "  focus: FAIL"; exit 1; }
grep -q '"occurrence":"Int -> Int"' /tmp/g5e_focus.json || { echo "  focus: occurrence not Int->Int"; exit 1; }
grep -q '"scheme":"forall a. a -> a"' /tmp/g5e_focus.json || { echo "  focus: binding scheme not forall"; exit 1; }
grep -q '"type":"Int"' /tmp/g5e_focus.json || { echo "  focus: instantiation a:=Int missing"; exit 1; }
grep -q '"containing":{"node"' /tmp/g5e_focus.json || { echo "  focus: containing app missing"; exit 1; }
grep -q '"reason":"application result"' /tmp/g5e_focus.json || { echo "  focus: containing reason missing"; exit 1; }
echo "  focus: definition vs occurrence + containing OK"
cat > /tmp/g5e_sig.dc <<'EOF'
sig restricted : Int -> Int
let restricted x = x
EOF
./gen5check types /tmp/g5e_sig.dc --format=json > /tmp/g5e_sig.json 2>&1 || { echo "  sig types: FAIL"; exit 1; }
grep -q '"scheme":"forall a. a -> a"' /tmp/g5e_sig.json || { echo "  sig: principal missing"; exit 1; }
grep -q '"contract":"Int -> Int"' /tmp/g5e_sig.json || { echo "  sig: contract field missing"; exit 1; }
echo "  sig: principal vs contract OK"
cat > /tmp/g5e_mod.dc <<'EOF'
module M exposing (restricted)
sig restricted : Int -> Int
let restricted x = x
EOF
./gen5check interface /tmp/g5e_mod.dc > /tmp/g5e.dci 2>&1 || { echo "  interface: FAIL"; exit 1; }
grep -q 'val restricted : Int -> Int' /tmp/g5e.dci || { echo "  interface: contract not published"; exit 1; }
echo "  interface: contract published OK"
cat > /tmp/g5e_narrow.dc <<'EOF'
module M exposing (f)
let f x = x
EOF
cat > /tmp/g5e_special.dc <<'EOF'
module M exposing (f)
let f x = x + 1
EOF
./gen5check interface /tmp/g5e_narrow.dc --write=/tmp/g5e_old.dci > /dev/null 2>&1 || { echo "  dci write: FAIL"; exit 1; }
set +e
./gen5check interface /tmp/g5e_special.dc --check=/tmp/g5e_old.dci > /dev/null 2>&1
[ $? -eq 3 ] || { echo "  dci diff: expected exit 3"; exit 1; }
set -e
echo "  interface diff: specialization detected OK"
cat > /tmp/g5e_dup.dc <<'EOF'
let a = { x = 1, x = 2 }
EOF
set +e
./gen5check check /tmp/g5e_dup.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  record dup: expected reject"; exit 1; }
cat > /tmp/g5e_miss.dc <<'EOF'
let f r = r.z
let a = { x = 1 }
let v = f a
EOF
./gen5check check /tmp/g5e_miss.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  record missing: expected reject"; exit 1; }
cat > /tmp/g5e_wbad.dc <<'EOF'
let a = { x = 1 }
let b = { a with x = true }
EOF
./gen5check check /tmp/g5e_wbad.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  record with-type-change: expected reject"; exit 1; }
set -e
echo "  record negatives: dup/missing/type-change rejected OK"
cat > /tmp/g5e_match.dc <<'EOF'
type T =
  | A
  | B
  | C
let f x = case x of | A -> 1 | B -> 2
EOF
./gen5check check /tmp/g5e_match.dc --format=json > /tmp/g5e_match.json 2>&1 || { echo "  match: CHECK FAIL"; exit 1; }
grep -q 'non-exhaustive' /tmp/g5e_match.json || { echo "  match: warning missing"; exit 1; }
grep -q '"missing":\["C"\]' /tmp/g5e_match.json || { echo "  match: missing example missing"; exit 1; }
echo "  match: exhaustiveness warning OK"
cat > /tmp/g5e_shadow.dc <<'EOF'
let x = 1
let x = true
let v = x
EOF
./gen5check focus /tmp/g5e_shadow.dc --at=3:9 --format=json > /tmp/g5e_shadow.json 2>&1 || { echo "  shadow: FAIL"; exit 1; }
grep -q '"occurrence":"Bool"' /tmp/g5e_shadow.json || { echo "  shadow: occurrence not Bool"; exit 1; }
grep -q '"start":14,"end":15' /tmp/g5e_shadow.json || { echo "  shadow: binding decl not second x"; exit 1; }
echo "  shadowing: resolved to nearest binding OK"
printf -- '-- \xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\nlet id x = x\nlet msg = "test"\nlet v = id 1\n' > /tmp/g5e_jp.dc
./gen5check check /tmp/g5e_jp.dc > /dev/null 2>&1 || { echo "  japanese: CHECK FAIL"; exit 1; }
./gen5check focus /tmp/g5e_jp.dc --at=4:9 --format=json > /tmp/g5e_jp.json 2>&1 || { echo "  japanese focus: FAIL"; exit 1; }
grep -q '"found":true' /tmp/g5e_jp.json || { echo "  japanese: byte span mismatch"; exit 1; }
echo "  japanese bytes: spans consistent OK"
./gen5check focus /tmp/g5e_id.dc --at=2:9 --max-bytes=100 --format=json > /tmp/g5e_trunc.json 2>&1 || { echo "  trunc: FAIL"; exit 1; }
grep -q '"truncated":true' /tmp/g5e_trunc.json || { echo "  trunc: expected truncated:true"; exit 1; }
grep -q 'continuation' /tmp/g5e_trunc.json || { echo "  trunc: continuation missing"; exit 1; }
echo "  byte budget: truncation + continuation OK"
cat > /tmp/g5e_ord1.dc <<'EOF'
module M exposing (f)
let f r = (r.a, r.b)
EOF
cat > /tmp/g5e_ord2.dc <<'EOF'
module M exposing (f)
let f r = let t = r.b in (r.a, t)
EOF
./gen5check interface /tmp/g5e_ord1.dc --write=/tmp/g5e_ord.dci > /dev/null 2>&1 || { echo "  order: WRITE FAIL"; exit 1; }
./gen5check interface /tmp/g5e_ord2.dc --check=/tmp/g5e_ord.dci > /dev/null 2>&1 || { echo "  order: field-order-only change must not diff"; exit 1; }
echo "  interface diff: field-order insensitive OK"
cat > /tmp/g5e_ordrt.dc <<'EOF'
let a = { x = 1, y = 2 }
let b = { y = 2, x = 1 }
let main () = print_int (a.x + b.x + a.y + b.y)
EOF
./dcc_6 /tmp/g5e_ordrt.dc /tmp/g5e_ordrt > /dev/null 2>&1 || { echo "  order runtime: COMPILE FAIL"; exit 1; }
/tmp/g5e_ordrt > /tmp/g5e_ordrt.out 2>&1 || { echo "  order runtime: RUN FAIL"; exit 1; }
[ "$(cat /tmp/g5e_ordrt.out)" = "6" ] || { echo "  order runtime: expected 6"; exit 1; }
echo "  record order: runtime access order-insensitive OK"
cat > /tmp/g5e_coll.dc <<'EOF'
let bad f = (f 1, f true)
EOF
./gen5check why /tmp/g5e_coll.dc --at=1:14 --format=json > /tmp/g5e_coll.json 2>&1 || true
grep -q '"kind":"collision-sites"' /tmp/g5e_coll.json || { echo "  collision: note missing"; exit 1; }
grep -q '"sites":\[' /tmp/g5e_coll.json || { echo "  collision: machine spans missing"; exit 1; }
grep -q 'monomorphic here' /tmp/g5e_coll.json || { echo "  collision: binding kind missing"; exit 1; }
echo "  why: collision sites OK"
./gen5check holes gen5-examples/hole.dc --format=json > /tmp/g5e_cap.json 2>&1 || { echo "  holes: FAIL"; exit 1; }
grep -q '"capped":true' /tmp/g5e_cap.json || { echo "  holes: capped flag missing"; exit 1; }
echo "  holes: cap transparency OK"
cat > /tmp/g5e_doc.dc <<'EOF'
--- Adds one to its argument.
--- Second line.
let inc x = x + 1
let main () = print_int (inc 41)
EOF
./gen5check types /tmp/g5e_doc.dc --format=json > /tmp/g5e_doc.json 2>&1 || { echo "  doc: FAIL"; exit 1; }
grep -q '"doc":"Adds one to its argument. Second line."' /tmp/g5e_doc.json || { echo "  doc: not extracted"; exit 1; }
echo "  doc comments: extracted OK"
cat > /tmp/g5e_cont.dc <<'EOF'
let bad f = (f 1, f true)
let ok = 42
let uses_bad = bad
let main () = print_int ok
EOF
set +e
./gen5check check /tmp/g5e_cont.dc > /tmp/g5e_cont.txt 2>&1
[ $? -eq 1 ] || { echo "  cont: expected exit 1"; exit 1; }
set -e
grep -q 'item 0: let bad' /tmp/g5e_cont.txt || { echo "  cont: first error changed"; exit 1; }
./gen5check check /tmp/g5e_cont.dc --format=json > /tmp/g5e_cont.json 2>&1 || true
grep -q '"kind":"lacking"' /tmp/g5e_cont.json || { echo "  cont: lacking mark missing"; exit 1; }
grep -q '"name":"ok"' /tmp/g5e_cont.json || { echo "  cont: independent item not analyzed"; exit 1; }
grep -q '"name":"main"' /tmp/g5e_cont.json || { echo "  cont: main not analyzed"; exit 1; }
echo "  error recovery: independent parts analyzed, dependents marked OK"

echo "ALL GEN5 CHECKS PASSED"
