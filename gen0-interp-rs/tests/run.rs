// dacelo Gen 0 integration tests

use dacelo::run_source_captured;

fn ok(src: &str) -> String {
    run_source_captured(src).unwrap_or_else(|e| panic!("expected success, got: {}\n---\n{}", e, src))
}

fn err(src: &str) -> String {
    match run_source_captured(src) {
        Ok(out) => panic!("expected error, got success with output:\n{}\n---\n{}", out, src),
        Err(e) => e,
    }
}

#[test]
fn hello() {
    assert_eq!(ok("let main () = print_string \"Hello\\n\""), "Hello\n");
}

#[test]
fn arithmetic_and_precedence() {
    let out = ok(
        r#"
let main () =
  print_int (1 + 2 * 3) ;
  print_string "," ;
  print_int ((1 + 2) * 3) ;
  print_string "," ;
  print_int (10 % 3) ;
  print_string "," ;
  print_int (7 / 2)
"#,
    );
    assert_eq!(out, "7,9,1,3");
}

#[test]
fn curried_application() {
    let out = ok(
        r#"
let add x y = x + y
let inc = add 1
let main () = print_int (inc 41)
"#,
    );
    assert_eq!(out, "42");
}

#[test]
fn recursion_fib() {
    let out = ok(include_str!("../../examples/fib.dc"));
    assert_eq!(out, "fib 20 = 6765\nfib 30 = 832040\n");
}

#[test]
fn adt_pattern_match_tree() {
    let out = ok(include_str!("../../examples/tree.dc"));
    assert_eq!(out, "sum   = 21\ndepth = 3\nsorted: 1 3 4 5 8\n");
}

#[test]
fn higher_order_lists() {
    let out = ok(include_str!("../../examples/list_ops.dc"));
    assert_eq!(
        out,
        "sum of squares 1..10 = 385\nevens 1..10          = 2 4 6 8 10\nproduct of 1..5      = 120\n"
    );
}

#[test]
fn closures_capture_env() {
    let out = ok(
        r#"
let compose f g = fun x -> f (g x)
let double = fun x -> x * 2
let add6 = fun x -> x + 6
let main () = print_int ((compose add6 double) 5)
"#,
    );
    assert_eq!(out, "16");
}

#[test]
fn local_lets_and_shadowing() {
    let out = ok(
        r#"
let x = 1
let main () =
  (let x = 10 in print_int x) ;
  print_string "," ;
  print_int x
"#,
    );
    assert_eq!(out, "10,1");
}

#[test]
fn let_rec_local() {
    let out = ok(
        r#"
let main () =
  let rec fact n = if n < 2 then 1 else n * fact (n - 1) in
  print_int (fact 5)
"#,
    );
    assert_eq!(out, "120");
}

#[test]
fn tuples_and_annotations() {
    let out = ok(
        r#"
let swap p : (Int, Bool) -> (Bool, Int) = case p of
  | (x, b) -> (b, x)
let main () =
  let q = swap (3, true) in
  print_string (show q)
"#,
    );
    assert_eq!(out, "(true, 3)");
}

#[test]
fn string_ops() {
    let out = ok(
        r#"
let main () =
  let s = "dacelo" in
  print_int (string_length s) ; print_string "\n" ;
  print_string ("hello" ++ " " ++ "world") ; print_string "\n" ;
  print_int (ord "A") ; print_string "," ;
  print_string (chr 66) ; print_string "\n" ;
  print_string (substring s 2 3) ; print_string "\n" ;
  print_int (string_get s 0) ; print_string "\n" ;
  print_int (string_to_int "123")
"#,
    );
    assert_eq!(out, "6\nhello world\n65,B\ncel\n100\n123");
}

#[test]
fn parametric_adt() {
    let out = ok(
        r#"
type Pair a b =
  | MkPair a b

type Maybe a =
  | Nothing
  | Just a

let rec find_first p xs = case xs of
  | [] -> Nothing
  | h :: rest -> if p h then Just h else find_first p rest

let get_or d m = case m of
  | Nothing -> d
  | Just v -> v

let main () =
  let r = find_first (fun x -> x > 3) [1, 5, 2] in
  print_int (get_or (-1) r) ; print_string "\n" ;
  let n = find_first (fun x -> x > 100) [1, 2] in
  print_int (get_or 0 n) ; print_string "\n" ;
  let p = MkPair true 7 in
  print_string (show p) ; print_string "," ;
  print_string (case p of | MkPair b v -> show v)
"#,
    );
    assert_eq!(out, "5\n0\n(MkPair true 7),7");
}

#[test]
fn polymorphic_identity() {
    let out = ok(
        r#"
let id x = x
let main () =
  print_int (id 42) ; print_string (id ",") ; print_string (id "poly")
"#,
    );
    assert_eq!(out, "42,poly");
}

#[test]
fn structural_equality() {
    let out = ok(
        r#"
let rec append xs ys = case xs of
    [] -> ys
  | h :: rest -> h :: append rest ys
let main () =
  let a = [1, 2, 3] in
  let b = append [1] [2, 3] in
  print_string (if a == b then "eq" else "ne") ;
  print_string "," ;
  print_string (if (1, "x") == (1, "x") then "eq" else "ne")
"#,
    );
    assert_eq!(out, "eq,eq");
}

// ---------- errors ----------

#[test]
fn type_error_mismatch() {
    let e = err("let main () = print_int true");
    assert!(e.contains("type mismatch"), "{}", e);
}

#[test]
fn unbound_variable() {
    let e = err("let main () = print_int nope");
    assert!(e.contains("unbound variable `nope`"), "{}", e);
}

#[test]
fn unknown_constructor_in_type_annotation() {
    let e = err("let f x : NoSuchType = x");
    assert!(e.contains("unknown type"), "{}", e);
}

#[test]
fn parse_error_missing_else() {
    let e = err("let main () = if true then 1");
    assert!(!e.is_empty());
}

#[test]
fn runtime_div_by_zero() {
    let e = err("let main () = print_int (1 / 0)");
    assert!(e.contains("division by zero"), "{}", e);
}

#[test]
fn non_exhaustive_match() {
    let e = err(
        r#"
let main () = case [] of
  | h :: _ -> print_int h
"#,
    );
    assert!(e.contains("non-exhaustive") || e.contains("pattern match failed"), "{}", e);
}

#[test]
fn occurs_check_rejects_infinite_type() {
    let e = err(
        r#"
let f x = x x
let main () = ()
"#,
    );
    assert!(e.contains("infinite type"), "{}", e);
}

#[test]
fn mutual_recursion_even_odd() {
    let out = ok(
        r#"
let rec even n = if n == 0 then true else odd (n - 1)
and odd n = if n == 0 then false else even (n - 1)

let main () =
  print_string (if even 10 then "even" else "?") ;
  print_string "," ;
  print_string (if odd 7 then "odd" else "?") ;
  print_string "," ;
  print_string (if even 3 then "?" else "no")
"#,
    );
    assert_eq!(out, "even,odd,no");
}

#[test]
fn mutual_recursion_three_way() {
    let out = ok(
        r#"
let rec ping n = if n == 0 then 0 else pong (n - 1) + 1
and pong n = if n == 0 then 0 else pang (n - 1) + 10
and pang n = if n == 0 then 0 else ping (n - 1) + 100

let main () =
  print_int (ping 5) ; print_string "," ; print_int (pang 2)
"#,
    );
    // ping5 -> pong4 -> pang3 -> ping2 -> pong1 -> pang0 -> 0+10+100+1... trace:
    // ping(5)=pong(4)+1; pong(4)=pang(3)+10; pang(3)=ping(2)+100;
    // ping(2)=pong(1)+1; pong(1)=pang(0)+10=10; => ping(2)=11, pang(3)=111,
    // pong(4)=121, ping(5)=122; pang(2)=ping(1)+100; ping(1)=pong(0)+1=1;
    // pang(2)=101
    assert_eq!(out, "122,101");
}

#[test]
fn mutual_recursion_with_lists() {
    let out = ok(
        r#"
-- split a list into elements at even / odd positions
let rec evens xs = case xs of
    [] -> []
  | h :: rest -> h :: odds rest
and odds xs = case xs of
    [] -> []
  | _ :: rest -> evens rest

let rec print_ints xs = case xs of
    [] -> print_string "\n"
  | h :: rest -> print_int h ; print_string "," ; print_ints rest

let main () =
  print_ints (evens [10, 20, 30, 40, 50]) ;
  print_ints (odds [10, 20, 30, 40, 50])
"#,
    );
    assert_eq!(out, "10,30,50,\n20,40,\n");
}

#[test]
fn pattern_let_binding() {
    let out = ok(
        r#"
let main () =
  let (a, b) = (3, 4) in
  print_int (a + b) ; print_string "," ;
  let (h :: t) = [10, 20] in
  print_int h
"#,
    );
    assert_eq!(out, "7,10");
}

#[test]
fn argv_sees_script_as_zero() {
    // argv 0 is the running script path; here we can only check that it is a
    // non-empty string that ends with .dc when invoked via the CLI. In tests
    // the process args are the test binary's; just exercise the builtin.
    let _ = run_source_captured("let main () = print_int (string_length (argv 0))");
}
