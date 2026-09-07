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

sec_A() {
echo "== A. checker =="
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/g5_driver.dc gen5/g5_main.dc > $G5CHECK_SRC
$GEN0 $G5CHECK_SRC --types > /dev/null && echo "gen5 typecheck OK"

if [ ! -x ./dcc_1 ]; then
  echo "dcc_1 missing: build it first (see gen3-dcc-dc/test.sh)"; exit 1
fi
if [ -n "${GEN5_KEEP_BIN:-}" ] && [ -x ./gen5check ]; then echo "gen5check kept (GEN5_KEEP_BIN)";
else ./dcc_1 $G5CHECK_SRC gen5check > /dev/null; [ -x ./gen5check ] || { echo "gen5check build produced no binary (dcc_1 ignores cc failures; check memory)"; exit 1; }; echo "gen5check built (by dcc_1)"; fi

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

}

sec_B() {
echo "== B. compiler (dcc_6: Gen5-RC backend = Perceus RC + borrow inference + reuse) =="
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/g5_driver.dc gen5/g5_cg.dc gen5/g5_cgdriver.dc gen5/g5cc_driver.dc > $G5CC_SRC
$GEN0 $G5CC_SRC --types > /dev/null && echo "g5cc typecheck OK"
if [ -n "${GEN5_KEEP_BIN:-}" ] && [ -x ./dcc_6 ]; then echo "dcc_6 kept (GEN5_KEEP_BIN)";
else ./dcc_1 $G5CC_SRC dcc_6 > /dev/null; [ -x ./dcc_6 ] || { echo "dcc_6 build produced no binary (dcc_1 ignores cc failures; check memory)"; exit 1; }; echo "dcc_6 built (by dcc_1; emits RC code, links gen5/rt5.c)"; fi
# every example: native output == Gen0, and DACELO_RC_CHECK proves every block is freed
for f in hello fib list_ops closures tree gc_stress; do
  ./dcc_6 examples/$f.dc /tmp/g6_$f > /dev/null 2>&1 || { echo "  $f: COMPILE FAIL"; exit 1; }
  /tmp/g6_$f > /tmp/g6_$f.out 2>&1 || { echo "  $f: RUNTIME FAIL"; exit 1; }
  $GEN0 examples/$f.dc > /tmp/ref_$f.out 2>&1
  diff -q /tmp/ref_$f.out /tmp/g6_$f.out > /dev/null || { echo "  $f: OUTPUT MISMATCH"; exit 1; }
  DACELO_RC_CHECK=1 /tmp/g6_$f > /dev/null 2> /tmp/g6_$f.leak || { echo "  $f: LEAK/UAF CHECK FAIL"; cat /tmp/g6_$f.leak; exit 1; }
  echo "  $f: output == Gen0, leak-free"
done
./dcc_6 gen5-examples/record.dc /tmp/g6_record > /dev/null 2>&1 || { echo "  record: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g6_record)" = "Alice,Bob" ] || { echo "  record: OUTPUT MISMATCH"; exit 1; }
DACELO_RC_CHECK=1 /tmp/g6_record > /dev/null 2>&1 || { echo "  record: LEAK"; exit 1; }
echo "  record: runs OK (Alice,Bob), leak-free"
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
DACELO_RC_CHECK=1 /tmp/g6_recw > /dev/null 2>&1 || { echo "  record_with: LEAK"; exit 1; }
echo "  record_with: runs OK (2), leak-free"
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
# drop-guided reuse: a unique list mapped in place must recycle every cons cell
cat > /tmp/g6_reuse.dc <<'EOF'
let rec range n acc = if n == 0 then acc else range (n - 1) (n :: acc)
let rec inc xs = case xs of
  | [] -> []
  | h :: t -> (h + 1) :: inc t
let rec sum xs acc = case xs of
  | [] -> acc
  | h :: t -> sum t (acc + h)
let main () = print_int (sum (inc (range 1000 [])) 0)
EOF
./dcc_6 /tmp/g6_reuse.dc /tmp/g6_reuse > /dev/null 2>&1 || { echo "  reuse: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g6_reuse)" = "501500" ] || { echo "  reuse: OUTPUT MISMATCH"; exit 1; }
DACELO_RC_STATS=1 DACELO_RC_CHECK=1 /tmp/g6_reuse > /dev/null 2> /tmp/g6_reuse.stats || { echo "  reuse: LEAK"; cat /tmp/g6_reuse.stats; exit 1; }
grep -qE 'reuses=(1000|[1-9][0-9]{3,})' /tmp/g6_reuse.stats || { echo "  reuse: expected >= 1000 in-place reuses"; cat /tmp/g6_reuse.stats; exit 1; }
echo "  reuse: unique list mapped in place ($(grep -oE 'reuses=[0-9]+' /tmp/g6_reuse.stats)), leak-free"
# use-after-free / double-free guard never fires on a closure-heavy program
DACELO_RC_CHECK=1 /tmp/g6_closures > /dev/null 2>&1 || { echo "  closures: RC CHECK FAIL"; exit 1; }
echo "  closures: partial application + local let rec leak-free"
}

sec_C() {
echo "== C. self-host: Gen5-RC builds Gen5-RC (dcc_7 -> dcc_8 fixpoint) =="
# stage1: dcc_6 (runs on the old runtime) builds the RC-compiled compiler and checker
./dcc_6 $G5CC_SRC dcc_7 --backend-only > /dev/null 2>&1 || { echo "  dcc_7 BUILD FAIL"; exit 1; }
./dcc_6 $G5CHECK_SRC gen5check_rc --backend-only > /dev/null 2>&1 || { echo "  gen5check_rc BUILD FAIL"; exit 1; }
echo "  stage1: dcc_7 + gen5check_rc built by dcc_6 (both run under RC)"
# stage2: the RC-compiled compiler rebuilds itself; assembly fixpoint
./dcc_7 $G5CC_SRC dcc_8 --backend-only > /dev/null 2>&1 || { echo "  dcc_8 BUILD FAIL"; exit 1; }
cmp -s dcc_7.s dcc_8.s || { echo "  FIXPOINT FAIL: dcc_7.s != dcc_8.s"; exit 1; }
echo "  FIXPOINT: dcc_7.s == dcc_8.s ($(wc -c < dcc_7.s | tr -d ' ') bytes)"
./dcc_8 examples/hello.dc /tmp/g8_hello > /dev/null 2>&1 || { echo "  dcc_8 SMOKE FAIL"; exit 1; }
/tmp/g8_hello > /tmp/g8_hello.out 2>&1
$GEN0 examples/hello.dc > /tmp/ref_hello.out 2>&1
diff -q /tmp/ref_hello.out /tmp/g8_hello.out > /dev/null || { echo "  dcc_8 OUTPUT MISMATCH"; exit 1; }
DACELO_RC_CHECK=1 ./dcc_8 examples/tree.dc /tmp/g8_tree > /dev/null 2>&1 || { echo "  dcc_8 (RC-compiled compiler) leaked or crashed while compiling"; exit 1; }
echo "  dcc_8 smokes OK; the RC-compiled compiler itself is leak-free on tree.dc"
# the RC-compiled checker must agree with Gen0 on the 38-case oracle
pass=0; fail=0
for f in examples/hello.dc examples/fib.dc examples/list_ops.dc examples/closures.dc examples/tree.dc gen4-infer-dc/tests/*.dc; do
  g5=0; ./gen5check_rc check "$f" > /tmp/o_g5.log 2>&1 || g5=$?
  g0=0; $GEN0 "$f" --types > /tmp/o_g0.log 2>&1 || g0=$?
  if [ $g5 -ne $g0 ]; then echo "EXIT-DIFFER($f): g5=$g5 g0=$g0"; fail=$((fail+1)); continue; fi
  diff -q /tmp/o_g5.log /tmp/o_g0.log > /dev/null || { echo "MSG-DIFFER($f)"; fail=$((fail+1)); continue; }
  pass=$((pass+1))
done
echo "  gen5check_rc oracle: $pass agree, $fail differ"
[ $fail -eq 0 ]
# headline: self-check under RC (the checker checks the compiler source), with time + peak RSS
set +e
/usr/bin/time -l ./gen5check_rc check $G5CC_SRC > /tmp/g5_selfcheck_rc.log 2>&1; sc2=$?
set -e
echo "  self-check (RC build): exit $sc2; $(grep -E 'maximum resident|real' /tmp/g5_selfcheck_rc.log | tr -s ' ' | tr '\n' ' ')"
[ $sc2 -eq 0 ] || { echo "  self-check FAIL under RC (log: /tmp/g5_selfcheck_rc.log)"; exit 1; }
DACELO_RC_CHECK=1 ./gen5check_rc check examples/tree.dc > /dev/null 2>&1 || { echo "  gen5check_rc leaked while checking tree.dc"; exit 1; }
echo "  gen5check_rc accepts its own compiler source and is leak-free on tree.dc"
# comparison point: the same self-check with the mark-sweep-built checker (never collects;
# known to be OOM-killed on 34GB boxes). Informational unless GEN5_SELFCHECK_STRICT=1.
if [ -n "${GEN5_SELFCHECK_STRICT:-}" ] || [ -n "${GEN5_SELFCHECK_COMPARE:-}" ]; then
  set +e
  /usr/bin/time -l ./gen5check check $G5CC_SRC > /tmp/g5_selfcheck.log 2>&1; sc=$?
  set -e
  echo "  self-check (mark-sweep build): exit $sc; $(grep -E 'maximum resident|real' /tmp/g5_selfcheck.log | tr -s ' ' | tr '\n' ' ')"
  if [ -n "${GEN5_SELFCHECK_STRICT:-}" ]; then [ $sc -eq 0 ] || { echo "  self-check FAIL (mark-sweep build)"; exit 1; }; fi
fi
}

sec_D() {
echo "== D. formatter round-trip =="
for f in gen3-dcc-dc/dcc.dc gen3-dcc-dc/g3_pm_v2.dc gen3-dcc-dc/g3_ce_v2.dc gen3-dcc-dc/g3_driver_v2.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/g5_query.dc gen5/g5_lower.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/g5_cg.dc gen5/g5_cgdriver.dc gen5/g5_driver.dc gen5/g5cc_driver.dc gen5/g5_main.dc gen5/probe_row_occurs.dc gen5/probe_row_unify.dc gen5/probe_own.dc gen5/probe_cg.dc gen4-infer-dc/infer.dc gen4-infer-dc/g4_check.dc gen4-infer-dc/g4_main.dc gen4-infer-dc/g4cc_driver.dc; do
  ./gen5check format "$f" > /dev/null 2>&1 || { echo "  FORMAT FAIL: $f"; exit 1; }
done
for f in gen5-examples/*.dc; do
  ./gen5check format "$f" > /dev/null 2>&1 || { echo "  FORMAT FAIL: $f"; exit 1; }
done
echo "  all sources format-clean"

}

sec_E() {
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
}

sec_F() {
echo "== F. issue #1 follow-up regressions (P1 soundness + P2 contracts) =="
cat > /tmp/g5f_pick.dc <<'EOF'
let pick a b =
  let ax = a.x in
  let by = b.y in
  if false then a else b
let bad = pick { x = 1, y = 2 } { y = 3 }
let main () = print_int (bad.x)
EOF
set +e
./gen5check check /tmp/g5f_pick.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  row-unify: pick must reject"; exit 1; }
set -e
echo "  row unify: residual fields enforced OK"
cat > /tmp/g5f_pick2.dc <<'EOF'
let pick2 a b = let c = if true then a else b in c.x + c.y
let main () = print_int (pick2 { x = 1, y = 2 } { y = 20, x = 10 })
EOF
cat > /tmp/g5f_pick2s.dc <<'EOF'
let pick2s a b = let c = if false then a else b in c.x + c.y
let main () = print_int (pick2s { x = 1, y = 2 } { y = 20, x = 10 })
EOF
./dcc_6 /tmp/g5f_pick2.dc /tmp/g5f_pick2 > /dev/null 2>&1 || { echo "  pick2: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_pick2)" = "3" ] || { echo "  pick2: expected 3"; exit 1; }
./dcc_6 /tmp/g5f_pick2s.dc /tmp/g5f_pick2s > /dev/null 2>&1 || { echo "  pick2s: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_pick2s)" = "30" ] || { echo "  pick2s: swapped branches must still check and run (30)"; exit 1; }
echo "  row unify: both sides normalize + branch order irrelevant OK"
cat > /tmp/g5f_occ.dc <<'EOF'
let f r = let s = { x = r } in if true then r else s
let main () = print_int 0
EOF
set +e
./gen5check check /tmp/g5f_occ.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  row occurs: nested cycle must reject"; exit 1; }
set -e
echo "  row occurs-check: nested cycle rejected OK"
cat > /tmp/g5f_proj.dc <<'EOF'
let mk x = { value = x }
let main () = print_int ((mk 42).value)
EOF
./dcc_6 /tmp/g5f_proj.dc /tmp/g5f_proj > /dev/null 2>&1 || { echo "  proj: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_proj)" = "42" ] || { echo "  proj: expected 42"; exit 1; }
cat > /tmp/g5f_lamapp.dc <<'EOF'
sig main : Unit -> Unit
let main () = print_int ((fun x -> x + 1) 41)
EOF
./dcc_6 /tmp/g5f_lamapp.dc /tmp/g5f_lamapp > /dev/null 2>&1 || { echo "  lamapp: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_lamapp)" = "42" ] || { echo "  lamapp: expected 42"; exit 1; }
echo "  lowering parens: projection receiver + fn lambda OK"
python3 -c "open('/tmp/g5f_str.dc','w').write('sig main : Unit -> Unit\nlet main () = print_int (string_length \"\\\\\\\\n\")\n')"
./dcc_6 /tmp/g5f_str.dc /tmp/g5f_str > /dev/null 2>&1 || { echo "  str: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_str)" = "2" ] || { echo "  str: backslash-n must stay 2 bytes"; exit 1; }
python3 -c "open('/tmp/g5f_strp.dc','w').write('sig main : Unit -> Unit\nlet f s = case s of | \"\\\\\\\\n\" -> 7 | _ -> 0\nlet main () = print_int (f \"\\\\\\\\n\")\n')"
./dcc_6 /tmp/g5f_strp.dc /tmp/g5f_strp > /dev/null 2>&1 || { echo "  strpat: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_strp)" = "7" ] || { echo "  strpat: expected 7"; exit 1; }
echo "  lowering strings: re-escape in expr + pattern OK"
cat > /tmp/g5f_cap.dc <<'EOF'
let read g5_rec_get = ({ x = 1 }).x
let fake r k = 99
let main () = print_int (read fake)
EOF
set +e
./gen5check check /tmp/g5f_cap.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  capture: reserved param must reject"; exit 1; }
set -e
cat > /tmp/g5f_legit.dc <<'EOF'
let ident g5_rec_get = g5_rec_get
let main () = print_int (ident 7)
EOF
./gen5check check /tmp/g5f_legit.dc > /dev/null 2>&1 || { echo "  capture: legacy-only binding must stay valid"; exit 1; }
echo "  helper capture: reserved rejected, legacy compat kept OK"
mkdir -p /tmp/g5fmod && printf 'module Dep exposing (answer)\nlet answer = 42\n' > /tmp/g5fmod/Dep.dc
printf 'import Dep\nlet copied = answer\nlet main () = print_int copied\n' > /tmp/g5fmod/Main.dc
./dcc_6 /tmp/g5fmod/Main.dc /tmp/g5fmod_main > /dev/null 2>&1 || { echo "  modorder: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5fmod_main)" = "42" ] || { echo "  modorder: top-level dep value must init first"; exit 1; }
printf 'module Base exposing (two)\nlet two = 2\n' > /tmp/g5fmod/Base.dc
printf 'module Mid exposing (twice)\nimport Base\nlet twice x = x + two\n' > /tmp/g5fmod/Mid.dc
printf 'import Base\nimport Mid\nlet v = twice two\nlet main () = print_int v\n' > /tmp/g5fmod/Dia.dc
./dcc_6 /tmp/g5fmod/Dia.dc /tmp/g5fmod_dia > /dev/null 2>&1 || { echo "  diamond: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5fmod_dia)" = "4" ] || { echo "  diamond: expected 4"; exit 1; }
printf 'module Dd exposing (double)\nlet double x = x * 2\n' > /tmp/g5fmod/Dd.dc
printf 'import Dd\nlet x = double 21\nlet main () = print_int x\n' > /tmp/g5fmod/Topcall.dc
./dcc_6 /tmp/g5fmod/Topcall.dc /tmp/g5fmod_top > /dev/null 2>&1 || { echo "  topcall: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5fmod_top)" = "42" ] || { echo "  topcall: expected 42"; exit 1; }
echo "  module order: dep-first init (value/diamond/top-call) OK"
mkdir -p /tmp/g5fiso && printf 'module A exposing (a)\nlet a = 1\n' > /tmp/g5fiso/A.dc
printf 'module B exposing (b)\nlet b = a\n' > /tmp/g5fiso/B.dc
printf 'import A\nimport B\nlet main () = print_int b\n' > /tmp/g5fiso/Main.dc
printf 'import B\nimport A\nlet main () = print_int b\n' > /tmp/g5fiso/Main2.dc
set +e
./gen5check check /tmp/g5fiso/Main.dc > /dev/null 2>&1; r1=$?
./gen5check check /tmp/g5fiso/Main2.dc > /dev/null 2>&1; r2=$?
./gen5check check /tmp/g5fiso/B.dc > /dev/null 2>&1; r3=$?
set -e
[ $r1 -eq 1 ] && [ $r2 -eq 1 ] && [ $r3 -eq 1 ] || { echo "  isolation: unimported export must reject (order-independent)"; exit 1; }
[ $r1 -eq $r2 ] || { echo "  isolation: import order changed result"; exit 1; }
echo "  module isolation: no leakage, import-order independent OK"
mkdir -p /tmp/g5fx && printf 'module Dep exposing (dep)\nlet dep = 0\n' > /tmp/g5fx/Dep.dc
printf 'import Dep\nlet value =              true\n' > /tmp/g5fx/Main.dc
./gen5check focus /tmp/g5fx/Main.dc --at=2:26 --format=json > /tmp/g5fx.json 2>&1 || { echo "  xfile focus: FAIL"; exit 1; }
grep -q '"occurrence":"Bool"' /tmp/g5fx.json || { echo "  xfile focus: picked wrong file node"; exit 1; }
cat > /tmp/g5fsc.dc <<'EOF'
let f x = x + 1
let g y = y
EOF
./gen5check focus /tmp/g5fsc.dc --at=2:11 --format=json > /tmp/g5fsc.json 2>&1 || { echo "  scope focus: FAIL"; exit 1; }
if grep -q '"name":"x"' /tmp/g5fsc.json; then echo "  scope: exited param x leaked"; exit 1; fi
grep -q '"name":"y"' /tmp/g5fsc.json || { echo "  scope: current param y missing"; exit 1; }
echo "  focus: cross-file pick + lexical local_env OK"
cat > /tmp/g5f_hole9.dc <<'EOF'
let good = 1
let wrong = true
let answer = (?value : Int)
let main () = print_int answer
EOF
./gen5check holes /tmp/g5f_hole9.dc > /tmp/g5f_hole9.txt 2>&1 || { echo "  holes9: FAIL"; exit 1; }
grep -q 'good : Int' /tmp/g5f_hole9.txt || { echo "  holes9: fitting candidate missing"; exit 1; }
if grep -q 'wrong' /tmp/g5f_hole9.txt; then echo "  holes9: ill-fitting candidate kept"; exit 1; fi
echo "  holes: post-annotation re-trial OK"
cat > /tmp/g5f_d10a.dc <<'EOF'
module M exposing (make)
type T = A
let make () = A
EOF
cat > /tmp/g5f_d10b.dc <<'EOF'
module M exposing (make)
type T = A | B
let make () = A
EOF
./gen5check interface /tmp/g5f_d10a.dc --write=/tmp/g5f_d10.dci > /dev/null 2>&1 || { echo "  dci write: FAIL"; exit 1; }
set +e
./gen5check interface /tmp/g5f_d10b.dc --check=/tmp/g5f_d10.dci > /dev/null 2>&1
[ $? -eq 3 ] || { echo "  dci: added ctor must diff"; exit 1; }
set -e
echo "  interface: ADT change detected OK"
mkdir -p /tmp/g5ffakebin && printf '#!/bin/sh\nexit 1\n' > /tmp/g5ffakebin/cc && chmod +x /tmp/g5ffakebin/cc
rm -f /tmp/g5ffake_out
set +e
PATH=/tmp/g5ffakebin:$PATH ./dcc_6 examples/hello.dc /tmp/g5ffake_out > /dev/null 2>&1
[ $? -ne 0 ] || { echo "  toolchain: failing cc must fail build"; exit 1; }
set -e
[ ! -e /tmp/g5ffake_out ] || { echo "  toolchain: stale output must not exist"; exit 1; }
echo "  toolchain: assembler/linker failure propagates OK"
cat > /tmp/g5f_plain.dc <<'EOF'
let main () = print_int (1 + 2)
EOF
cat > /tmp/g5f_wrapped.dc <<'EOF'
module W exposing (main)
sig main : Unit -> Unit
let main () = print_int (1 + 2)
EOF
./dcc_6 /tmp/g5f_plain.dc /tmp/g5f_plain > /dev/null 2>&1 || { echo "  meaning: PLAIN FAIL"; exit 1; }
./dcc_6 /tmp/g5f_wrapped.dc /tmp/g5f_wrapped > /dev/null 2>&1 || { echo "  meaning: WRAPPED FAIL"; exit 1; }
[ "$(/tmp/g5f_plain)" = "$(/tmp/g5f_wrapped)" ] || { echo "  meaning: sig/module wrapper changed result"; exit 1; }
[ "$(/tmp/g5f_plain)" = "3" ] || { echo "  meaning: expected 3"; exit 1; }
cat > /tmp/g5f_combo.dc <<'EOF'
module C exposing (main)
sig main : Unit -> Unit
let apply f x = f x
let r = { name = "a\nb", n = 1 }
let r2 = { r with n = apply (fun v -> v + 1) r.n }
let main () = print_string (r2.name ++ show r2.n)
EOF
./dcc_6 /tmp/g5f_combo.dc /tmp/g5f_combo > /dev/null 2>&1 || { echo "  combo: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5f_combo)" = $'a\nb2' ] || { echo "  combo: OUTPUT MISMATCH"; exit 1; }
echo "  meaning preserved across lowering paths OK"
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/probe_row_occurs.dc > /tmp/g5occ_full.dc
$GEN0 /tmp/g5occ_full.dc > /tmp/g5occ.out 2>&1 || { echo "  occurs probe: RUN FAIL"; exit 1; }
printf 'true\ntrue\ntrue\ntrue\nfalse\nfalse\ntrue\n' > /tmp/g5occ.exp
diff -q /tmp/g5occ.exp /tmp/g5occ.out > /dev/null || { echo "  occurs probe: OUTPUT MISMATCH"; exit 1; }
echo "  row occurs: unit predicate OK (direct/nested/fun/tuple/negatives/subst)"
cat gen3-dcc-dc/dcc.dc gen5/g5_front.dc gen5/g5_infer.dc gen5/probe_row_unify.dc > /tmp/g5runify_full.dc
$GEN0 /tmp/g5runify_full.dc > /tmp/g5runify.out 2>&1 || { echo "  row unify probe: RUN FAIL"; exit 1; }
printf 'true\n{x : Int, y : Bool | r}\n{y : Bool | r}\n{x : Int | r}\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n' > /tmp/g5runify.exp
diff -q /tmp/g5runify.exp /tmp/g5runify.out > /dev/null || { echo "  row unify probe: OUTPUT MISMATCH"; diff /tmp/g5runify.exp /tmp/g5runify.out; exit 1; }
echo "  row unify: unit invariants OK (both sides normalize equal, swap-invariant, residual to both tails, rigid/closed/occurs)"
mkdir -p /tmp/g5r1 && printf 'module A exposing (make)\ntype Box = ABox Int\nlet make x = ABox x\n' > /tmp/g5r1/A.dc
printf 'module B exposing (read)\ntype Box = BBox Bool\nlet read x = case x of | BBox b -> if b then 1 else 0\n' > /tmp/g5r1/B.dc
printf 'import A\nimport B\nlet main () = print_int (read (make 42))\n' > /tmp/g5r1/Main.dc
set +e
./gen5check check /tmp/g5r1/Main.dc > /dev/null 2>&1
[ $? -eq 1 ] || { echo "  nominal dup: same-name ADT across modules must reject"; exit 1; }
set -e
mkdir -p /tmp/g5r1d && printf 'module Sh exposing (mk, get)\ntype T = | C Int\nlet mk n = C n\nlet get x = case x of | C n -> n\n' > /tmp/g5r1d/Sh.dc
printf 'import Sh\nlet main () = print_int (get (mk 2))\n' > /tmp/g5r1d/U1.dc
printf 'import Sh\nlet main () = print_int 0\n' > /tmp/g5r1d/E1.dc
printf 'import Sh\nlet main () = print_int 0\n' > /tmp/g5r1d/E2.dc
printf 'import E1\nimport E2\nimport Sh\nlet main () = print_int 0\n' > /tmp/g5r1d/Dia.dc
./gen5check check /tmp/g5r1d/Dia.dc > /dev/null 2>&1 || { echo "  nominal diamond: shared ADT via two paths must accept"; exit 1; }
echo "  nominal ADT: dup rejected, diamond accepted OK"
cat > /tmp/g5r2_scope.dc <<'EOF'
let f u =
  let a = (let hidden = 1 in hidden) in
  u
let later = true
EOF
./gen5check focus /tmp/g5r2_scope.dc --at=3:3 --format=json > /tmp/g5r2_scope.json 2>&1 || { echo "  scope: FAIL"; exit 1; }
if grep -q '"name":"hidden"' /tmp/g5r2_scope.json; then echo "  scope: exited let leaked"; exit 1; fi
if grep -q '"name":"later"' /tmp/g5r2_scope.json; then echo "  scope: later def leaked"; exit 1; fi
grep -q '"name":"u"' /tmp/g5r2_scope.json || { echo "  scope: current param missing"; exit 1; }
grep -q '"name":"a"' /tmp/g5r2_scope.json || { echo "  scope: visible local missing"; exit 1; }
cat > /tmp/g5r2_sib.dc <<'EOF'
type T = | A Int | B Int
let f v = case v of | A n -> n + 1 | B m -> m + 2
let main () = print_int 0
EOF
./gen5check focus /tmp/g5r2_sib.dc --at=2:38 --format=json > /tmp/g5r2_sib.json 2>&1 || { echo "  sibscope: FAIL"; exit 1; }
if grep -q '"name":"n"' /tmp/g5r2_sib.json; then echo "  sibscope: sibling branch leaked"; exit 1; fi
grep -q '"name":"m"' /tmp/g5r2_sib.json || { echo "  sibscope: own branch missing"; exit 1; }
cat > /tmp/g5r2_lam.dc <<'EOF'
let g a = a
let f x = g (fun y -> y)
let main () = print_int 0
EOF
./gen5check focus /tmp/g5r2_lam.dc --at=2:23 --format=json > /tmp/g5r2_lam.json 2>&1 || { echo "  lamscope: FAIL"; exit 1; }
grep -q '"name":"y"' /tmp/g5r2_lam.json || { echo "  lamscope: inner lambda param missing"; exit 1; }
if grep -q '"name":"a"' /tmp/g5r2_lam.json; then echo "  lamscope: outer param leaked into file scope"; exit 1; fi
cat > /tmp/g5r2_sh.dc <<'EOF'
let f x = 1
let g x = x
let main () = print_int 0
EOF
./gen5check focus /tmp/g5r2_sh.dc --at=2:11 --format=json > /tmp/g5r2_sh.json 2>&1 || { echo "  shadowscope: FAIL"; exit 1; }
grep -q '"start":18,"end":19' /tmp/g5r2_sh.json || { echo "  shadowscope: nearest binding not chosen"; exit 1; }
cat > /tmp/g5r2_rec.dc <<'EOF'
let rec f n = if n == 0 then 0 else f (n - 1)
let main () = print_int (f 3)
EOF
./gen5check focus /tmp/g5r2_rec.dc --at=1:33 --format=json > /tmp/g5r2_rec.json 2>&1 || { echo "  recscope: FAIL"; exit 1; }
grep -q '"name":"f"' /tmp/g5r2_rec.json || { echo "  recscope: rec name missing in own body"; exit 1; }
grep -q '"name":"n"' /tmp/g5r2_rec.json || { echo "  recscope: param missing"; exit 1; }
echo "  lexical scope: exited/later/sibling/lambda/shadow/rec OK"
mkdir -p /tmp/g5r3 && printf 'module A exposing (value)\nlet value = 42\n' > /tmp/g5r3/A.dc
printf 'module B exposing (other)\nlet other value = value\nlet value = true\n' > /tmp/g5r3/B.dc
printf 'module C exposing (third)\nlet value = "s"\nlet third = value\n' > /tmp/g5r3/C.dc
printf 'import A\nimport B\nimport C\nlet answer = value\n' > /tmp/g5r3/Main.dc
./gen5check focus /tmp/g5r3/Main.dc --at=4:14 --format=json > /tmp/g5r3.json 2>&1 || { echo "  impbinding: FAIL"; exit 1; }
grep -q '"occurrence":"Int"' /tmp/g5r3.json || { echo "  impbinding: occurrence not Int (private same-name value leaked?)"; exit 1; }
grep -q '"binding":{"id":[0-9]*,"name":"value","decl":{"start":30,"end":35,"file":"/tmp/g5r3/A.dc"}' /tmp/g5r3.json || { echo "  impbinding: decl is not A.dc top-level value [30,35)"; exit 1; }
echo "  import binding: decl = exporter's def despite same-name param/private values in B/C OK"
cat > /tmp/g5r4.dc <<'EOF'
let good = 1
let x = 2
let f x = if x then (?h : Int) else 0
EOF
./gen5check holes /tmp/g5r4.dc > /tmp/g5r4.txt 2>&1 || { echo "  shadowhole: FAIL"; exit 1; }
grep -q 'good : Int' /tmp/g5r4.txt || { echo "  shadowhole: good missing"; exit 1; }
if grep -q '^  x :' /tmp/g5r4.txt; then echo "  shadowhole: shadowed x kept"; exit 1; fi
cat > /tmp/g5r4f.dc <<'EOF'
let good = 1
let x = 2
let f x = if x then good else 0
let main () = print_int (f true)
EOF
./gen5check check /tmp/g5r4f.dc > /dev/null 2>&1 || { echo "  shadowhole: filled recheck FAIL"; exit 1; }
./dcc_6 /tmp/g5r4f.dc /tmp/g5r4f > /dev/null 2>&1 || { echo "  shadowhole: filled COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5r4f)" = "1" ] || { echo "  shadowhole: filled run MISMATCH"; exit 1; }
echo "  hole shadowing: hidden binding dropped, fill rechecks+runs OK"

}

sec_G() {
echo "== G. ownership: borrow inference visible through the query API =="
cat > /tmp/g5g_own.dc <<'EOF'
let id x = x
let rec len xs = case xs of
  | [] -> 0
  | _ :: t -> 1 + len t
let rec map f xs = case xs of
  | [] -> []
  | h :: t -> f h :: map f t
let pair a b = (a, b)
let main () = print_int (len (map id [1, 2, 3]))
EOF
./gen5check types /tmp/g5g_own.dc --format=json > /tmp/g5g_own.json 2>&1 || { echo "  ownership types: FAIL"; exit 1; }
python3 - <<'PYEOF' || { echo "  ownership: JSON signatures differ from the inferred contract"; exit 1; }
import json
d = json.load(open('/tmp/g5g_own.json'))
got = {b['name']: b.get('ownership') for it in d['result']['items'] for b in it['binds']}
want = {'id': ['owned'],                 # returned -> consumed
        'len': ['borrowed'],             # only inspected
        'map': ['borrowed', 'owned'],    # f only applied; xs matched + rebuilt (reuse)
        'pair': ['owned', 'owned'],      # stored in a tuple
        'main': ['borrowed']}            # unit parameter, only matched
bad = {k: (got.get(k), v) for k, v in want.items() if got.get(k) != v}
if bad:
    print("  ownership mismatch:", bad); raise SystemExit(1)
PYEOF
echo "  ownership signatures: id/len/map/pair as inferred OK"
./dcc_6 /tmp/g5g_own.dc /tmp/g5g_own > /dev/null 2>&1 || { echo "  ownership program: COMPILE FAIL"; exit 1; }
[ "$(/tmp/g5g_own)" = "3" ] || { echo "  ownership program: expected 3"; exit 1; }
DACELO_RC_CHECK=1 /tmp/g5g_own > /dev/null 2>&1 || { echo "  ownership program: LEAK"; exit 1; }
echo "  ownership program: runs and is leak-free OK"
cat gen3-dcc-dc/dcc.dc gen5/g5_oir.dc gen5/g5_own.dc gen5/probe_own.dc > /tmp/g5own_full.dc
$GEN0 /tmp/g5own_full.dc > /tmp/g5own.out 2>&1 || { echo "  ownership probe: RUN FAIL"; exit 1; }
diff -q gen5/oracle/probe_own.expected /tmp/g5own.out > /dev/null || { echo "  ownership probe: IR dump changed (diff vs gen5/oracle/probe_own.expected)"; diff gen5/oracle/probe_own.expected /tmp/g5own.out | head -20; exit 1; }
echo "  ownership probe: ANF/borrow inference/dup-drop/reuse dump pinned OK"
}

# Section selection (sections A-F above are functions; default runs all):
#   GEN5_SKIP="C"     skip listed sections (space-separated letters)
#   GEN5_ONLY="E F"   run only the listed sections
# Later sections use ./gen5check (built by A) and ./dcc_6 (built by B); when
# skipping A/B the existing binaries must already match the current sources
# (test.sh regenerates gen5/g5*_full.dc, so a plain `cmp` against them tells).
skip_sec() {
  case " ${GEN5_SKIP:-} " in *" $1 "*) return 0;; esac
  if [ -n "${GEN5_ONLY:-}" ]; then
    case " $GEN5_ONLY " in *" $1 "*) return 1;; *) return 0;; esac
  fi
  return 1
}
for sec in A B C D E F G; do
  if skip_sec $sec; then echo "== $sec. skipped (GEN5_SKIP/GEN5_ONLY) =="; else sec_$sec; fi
done

echo "ALL GEN5 CHECKS PASSED"
