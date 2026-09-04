// codegen.rs -- dacelo AST -> ARM64 machine code

use std::collections::HashMap;

use crate::encoder::{CodeBuf, Cond};
use crate::macho::{ObjectBuilder, Reloc, Section};
use dacelo::ast::*;

// value encoding constants -- must match rt.c
const TAG_STRING: u64 = 1;
const TAG_TUPLE: u64 = 2;
const TAG_ADT: u64 = 3;
const TAG_CLOSURE: u64 = 4;

fn hdr(tag: u64, size_words: u64) -> u64 {
    (size_words << 8) | tag
}

/// builtin indices -- must match rt.c enum order
pub const BUILTINS: &[&str] = &[
    "print_int",
    "print_string",
    "int_to_string",
    "bool_to_string",
    "string_length",
    "str_concat",
    "read_file",
    "write_file",
    "exit",
    "chr",
    "ord",
    "string_get",
    "substring",
    "string_to_int",
    "error",
    "show",
    "argv",
    "system",
];

const FRAME_BYTES: u32 = 4032; // fixed frame: 504 spill slots (imm12 limit)

const X0: u32 = 0; // result / first arg
const X1: u32 = 1; // argument / second operand

struct FnState {
    nslots: u32,
    scope: Vec<(String, u32)>,
}

impl FnState {
    fn fresh() -> FnState {
        FnState { nslots: 0, scope: Vec::new() }
    }
}

fn wrap_lams(params: &[Pattern], mut body: Expr) -> Expr {
    for p in params.iter().rev() {
        body = Expr::Lam(p.clone(), Box::new(body));
    }
    body
}

fn pat_name(p: &Pattern) -> Option<String> {
    match p {
        Pattern::Var(n) => Some(n.clone()),
        _ => None,
    }
}

/// a function whose machine code is emitted after top-level definitions
struct PendingFn {
    labels: Vec<u32>,
    syms: Vec<String>,
    captured_names: Vec<String>,
    params: Vec<Pattern>,
    body: Expr,
}

pub struct Codegen {
    asm: CodeBuf,
    ctors: HashMap<String, (u32, usize)>, // name -> (id, arity)
    ctor_names: Vec<String>,
    globals: HashMap<String, u32>, // name -> global table index
    n_globals: u32,
    data: Vec<u8>,
    data_syms: Vec<(String, u64)>,
    data_relocs: Vec<Reloc>,
    /// label id -> (symbol name, exported)
    label_syms: HashMap<u32, (String, bool)>,
    pending: Vec<PendingFn>,
    frame_patches: Vec<usize>,
    externs: Vec<String>,
    str_consts: HashMap<String, String>,
    lam_counter: u32,
}

impl Codegen {
    pub fn new(ctor_names_in_order: &[String], ctor_arities: &HashMap<String, usize>) -> Codegen {
        let mut ctors = HashMap::new();
        for (i, name) in ctor_names_in_order.iter().enumerate() {
            ctors.insert(name.clone(), (i as u32, ctor_arities[name]));
        }
        let mut g = Codegen {
            asm: CodeBuf::new(),
            ctors,
            ctor_names: ctor_names_in_order.to_vec(),
            globals: HashMap::new(),
            n_globals: 0,
            data: Vec::new(),
            data_syms: Vec::new(),
            data_relocs: Vec::new(),
            label_syms: HashMap::new(),
            pending: Vec::new(),
            frame_patches: Vec::new(),
            externs: Vec::new(),
            str_consts: HashMap::new(),
            lam_counter: 0,
        };
        for b in BUILTINS {
            g.alloc_global(b);
        }
        for name in ctor_names_in_order {
            if g.ctors[name].1 > 0 {
                g.alloc_global(name);
            }
        }
        g
    }

    fn alloc_global(&mut self, name: &str) -> u32 {
        let idx = self.n_globals;
        self.globals.insert(name.to_string(), idx);
        self.n_globals += 1;
        idx
    }

    fn use_extern(&mut self, name: &str) {
        if !self.externs.iter().any(|e| e == name) {
            self.externs.push(name.to_string());
        }
    }

    // ---------------- runtime calls ----------------

    fn call_rt(&mut self, unmangled: &str) {
        let sym = format!("_{unmangled}");
        self.use_extern(&sym);
        self.asm.ldr_lit_extern(crate::encoder::R9, &sym);
        self.asm.blr(crate::encoder::R9);
    }

    /// load value of global slot `g` into x0
    fn emit_gget(&mut self, g: u32) {
        self.asm.load_const_u64(X0, g as u64);
        self.call_rt("dc_gget");
    }

    /// store x0 into global slot `g`
    fn fill_global(&mut self, g: u32) {
        self.asm.mov(X1, X0);
        self.asm.load_const_u64(X0, g as u64);
        self.call_rt("dc_gset");
    }

    fn load_unit(&mut self, reg: u32) {
        self.use_extern("_dc_unit_block");
        self.asm.ldr_lit_extern(reg, "_dc_unit_block");
    }

    // ---------------- static data ----------------

    fn intern_str(&mut self, s: &str) -> String {
        if let Some(sym) = self.str_consts.get(s) {
            return sym.clone();
        }
        let sym = format!("_str{}", self.str_consts.len());
        self.str_consts.insert(s.to_string(), sym.clone());
        let bytes = s.as_bytes();
        let words = 2 + (bytes.len() + 7) / 8;
        let off = self.data.len() as u64;
        self.data.extend_from_slice(&hdr(TAG_STRING, words as u64).to_le_bytes());
        self.data.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.data.extend_from_slice(bytes);
        while self.data.len() % 8 != 0 {
            self.data.push(0);
        }
        self.data_syms.push((sym.clone(), off));
        sym
    }

    fn intern_nullary_ctor(&mut self, name: &str) -> String {
        let probe = format!("_ctorinst_{name}");
        if self.data_syms.iter().any(|(n, _)| *n == probe) {
            return probe;
        }
        let id = self.ctors[name].0 as u64;
        let off = self.data.len() as u64;
        self.data.extend_from_slice(&hdr(TAG_ADT, 2).to_le_bytes());
        self.data.extend_from_slice(&id.to_le_bytes());
        self.data_syms.push((probe.clone(), off));
        probe
    }

    fn emit_ctor_names_table(&mut self) {
        // pointer array first, name bytes after; relocations are collected
        // out-of-band so emission order doesn't matter
        let off = self.data.len() as u64;
        for _ in &self.ctor_names {
            self.data.extend_from_slice(&[0u8; 8]); // placeholder quad
        }
        for (i, name) in self.ctor_names.iter().enumerate() {
            let nm_sym = format!("_nm{i}");
            let noff = self.data.len() as u64;
            self.data.extend_from_slice(name.as_bytes());
            self.data.push(0);
            while self.data.len() % 8 != 0 {
                self.data.push(0);
            }
            self.data_syms.push((nm_sym.clone(), noff));
            self.data_relocs.push(Reloc {
                offset: off + 8 * i as u64,
                sym_name: nm_sym,
                kind: crate::macho::RelocKind::Unsigned,
            });
        }
        // exported: rt.o's show() indexes this table
        self.data_syms.push(("_dc_ctor_names".to_string(), off));
    }

    // ---------------- frames ----------------

    // Dynamic frames: prologue reserves bytes via `movz x9,#N; sub sp,sp,x9`
    // with N patched after the body is emitted (max slot count known then).
    fn fn_prologue(&mut self, captured: usize) {
        self.asm.stp_fp_lr();
        self.asm.mov_sp_fp();
        let patch_pos = self.asm.code.len();
        self.asm.load_const_u64(crate::encoder::R9, 0);
        self.asm.sub_sp_reg();
        self.frame_patches.push(patch_pos);
        // Zero-initialize the whole reserved frame so that any slot read
        // before its first write yields 0 instead of stale garbage left by
        // previous frames (root cause class of layout-dependent crashes).
        // x9 still holds the (patched) byte count here; x10/x11 are scratch.
        //   x10 = sp (cursor); x11 = sp + bytes (end)
        //   loop: if x10 >= x11 goto done; *x10 = 0; x10 += 8; repeat
        let zloop = self.new_label();
        let zdone = self.new_label();
        self.asm.add_imm(crate::encoder::R10, crate::encoder::RSP, 0);
        self.asm.add_reg(crate::encoder::R11, crate::encoder::R10, crate::encoder::R9, 0);
        self.asm.bind_label(zloop);
        self.asm.cmp_reg(crate::encoder::R10, crate::encoder::R11);
        self.asm.b_cond(crate::encoder::Cond::Hs, zdone);
        self.asm.str_xzr_post8(crate::encoder::R10);
        self.asm.b(zloop);
        self.asm.bind_label(zdone);
        // closure layout: [hdr@0][code@8][env_size@16][env words @24..]
        for i in 0..captured {
            self.asm.ldr_off(crate::encoder::R9, X0, 24 + 8 * i as i64);
            self.asm.str_slot(crate::encoder::R9, i as u32);
        }
    }

    fn fn_epilogue(&mut self, max_slots: u32) {
        let bytes = ((max_slots as u64 + 8) * 8 + 15) & !15;
        assert!(bytes <= 0xFFF0, "function frame exceeds 64 KiB");
        let pos = self
            .frame_patches
            .pop()
            .expect("fn_epilogue without fn_prologue");
        self.asm.patch_movz_imm16(pos, bytes as u16);
        self.asm.load_const_u64(crate::encoder::R9, bytes);
        self.asm.add_sp_reg();
        self.asm.ldp_fp_lr_ret();
    }

    fn spill_slot(f: &mut FnState) -> u32 {
        let s = f.nslots;
        f.nslots += 1;
        s
    }

    fn scope_find(f: &FnState, name: &str) -> Option<u32> {
        f.scope.iter().rev().find(|(n, _)| n == name).map(|(_, s)| *s)
    }

    /// allocate closure {code=label, env=captured}; result in x0
    fn make_closure(&mut self, label: u32, captured: &[(String, u32)]) {
        let n = captured.len() as u64;
        let words = 3 + n;
        self.asm.load_const_u64(X0, words * 8);
        self.call_rt("dacelo_alloc");
        self.asm.load_const_u64(crate::encoder::R9, hdr(TAG_CLOSURE, words));
        self.asm.str_off(crate::encoder::R9, X0, 0);
        self.asm.ldr_lit_label(crate::encoder::R9, label);
        self.asm.str_off(crate::encoder::R9, X0, 8);
        self.asm.load_const_u64(crate::encoder::R9, n);
        self.asm.str_off(crate::encoder::R9, X0, 16);
        for (j, (_nm, src)) in captured.iter().enumerate() {
            self.asm.ldr_slot(crate::encoder::R9, *src);
            self.asm.str_off(crate::encoder::R9, X0, 24 + 8 * j as i64);
        }
    }

    // ---------------- variables / constructors ----------------

    /// load variable value into x0; false if unbound
    fn load_var(&mut self, f: &mut FnState, name: &str) -> bool {
        if let Some(s) = Self::scope_find(f, name) {
            self.asm.ldr_slot(X0, s);
            return true;
        }
        if let Some(g) = self.globals.get(name).cloned() {
            self.emit_gget(g);
            return true;
        }
        false
    }

    fn load_ctor_value(&mut self, name: &str) {
        let (_, arity) = self.ctors[name];
        if arity == 0 {
            let sym = self.intern_nullary_ctor(name);
            self.asm.ldr_lit_extern(X0, &sym);
        } else {
            let g = self.globals[name];
            self.emit_gget(g);
        }
    }

    fn emit_apply(&mut self) {
        // fval in x0, arg in x1
        self.asm.ldr_off(crate::encoder::R9, X0, 8);
        self.asm.blr(crate::encoder::R9);
    }

    // ---------------- patterns ----------------

    /// test pattern against value in reg; jump fail on mismatch; bind on success
    fn match_pat(&mut self, f: &mut FnState, pat: &Pattern, reg: u32, fail: u32) {
        let R = crate::encoder::R9;
        let T = crate::encoder::R10;
        // INVARIANT: caller-saved registers may be clobbered at any nested
        // step (the string-equality helper is a C call), so every compound
        // pattern works from its own spill slot instead of trusting `reg`.
        match pat {
            Pattern::Wildcard => {}
            Pattern::Var(n) => {
                let s = Self::spill_slot(f);
                self.asm.str_slot(reg, s);
                f.scope.push((n.clone(), s));
            }
            Pattern::PLit(Lit::Int(k)) => {
                // literal goes in T so a nested call (reg == R) still works
                self.asm.load_const_i64(T, (*k << 2) | 1);
                self.asm.cmp_reg(reg, T);
                self.asm.b_cond(Cond::Ne, fail);
            }
            Pattern::PLit(Lit::Bool(b)) => {
                self.asm.load_const_i64(T, if *b { 7 } else { 3 });
                self.asm.cmp_reg(reg, T);
                self.asm.b_cond(Cond::Ne, fail);
            }
            Pattern::PLit(Lit::Str(s)) => {
                // compare by CONTENT: substrings are fresh objects
                let sym = self.intern_str(s);
                let sv = Self::spill_slot(f);
                self.asm.str_slot(reg, sv);
                self.asm.ldr_lit_extern(X0, &sym);
                self.asm.ldr_slot(X1, sv);
                self.call_rt("dc_val_eq");
                self.asm.cmp_imm(X0, 7);
                self.asm.b_cond(Cond::Ne, fail);
                // (scrutinee preserved via slot; no reg aliasing possible)
            }
            Pattern::PLit(Lit::Unit) => {
                self.use_extern("_dc_unit_block");
                self.asm.ldr_lit_extern(T, "_dc_unit_block");
                self.asm.cmp_reg(reg, T);
                self.asm.b_cond(Cond::Ne, fail);
            }
            Pattern::PTuple(ps) => {
                let sv = Self::spill_slot(f);
                self.asm.str_slot(reg, sv);
                self.asm.ldr_slot(R, sv);
                self.asm.ldr_off(R, R, 0);
                self.asm.load_const_u64(T, hdr(TAG_TUPLE, 1 + ps.len() as u64));
                self.asm.cmp_reg(R, T);
                self.asm.b_cond(Cond::Ne, fail);
                for (i, p) in ps.iter().enumerate() {
                    self.asm.ldr_slot(R, sv);
                    self.asm.ldr_off(R, R, 8 * (i as i64 + 1));
                    self.match_pat(f, p, R, fail);
                }
            }
            Pattern::PCtor(name, ps) => {
                let (id, arity) = self.ctors[name];
                if arity == 0 {
                    assert!(ps.is_empty(), "nullary ctor pattern with args");
                    let sym = self.intern_nullary_ctor(name);
                    // NEVER clobber `reg`: when called from a parent match
                    // reg == R and loading into R would compare x9 with x9
                    self.asm.ldr_lit_extern(T, &sym);
                    self.asm.cmp_reg(reg, T);
                    self.asm.b_cond(Cond::Ne, fail);
                } else {
                    assert_eq!(ps.len(), arity, "ctor arity mismatch in pattern");
                    let sv = Self::spill_slot(f);
                    self.asm.str_slot(reg, sv);
                    self.asm.ldr_slot(R, sv);
                    self.asm.ldr_off(R, R, 0);
                    self.asm.load_const_u64(T, hdr(TAG_ADT, 2 + arity as u64));
                    self.asm.cmp_reg(R, T);
                    self.asm.b_cond(Cond::Ne, fail);
                    self.asm.ldr_slot(R, sv);
                    self.asm.ldr_off(R, R, 8);
                    self.asm.load_const_u64(T, id as u64);
                    self.asm.cmp_reg(R, T);
                    self.asm.b_cond(Cond::Ne, fail);
                    for (i, p) in ps.iter().enumerate() {
                        self.asm.ldr_slot(R, sv);
                        self.asm.ldr_off(R, R, 8 * (i as i64 + 2));
                        self.match_pat(f, p, R, fail);
                    }
                }
            }
        }
    }

    fn case_expr(&mut self, f: &mut FnState, scrut: &Expr, branches: &[(Pattern, Expr)]) {
        self.expr(f, scrut);
        let sv = Self::spill_slot(f);
        self.asm.str_slot(X0, sv);
        let end = self.new_label();
        let nexts: Vec<u32> = branches.iter().map(|_| self.new_label()).collect();
        for (i, (pat, body)) in branches.iter().enumerate() {
            let saved_len = f.scope.len();
            self.asm.ldr_slot(X1, sv);
            self.match_pat(f, pat, X1, nexts[i]);
            self.expr(f, body);
            f.scope.truncate(saved_len);
            self.asm.b(end);
            // every alternative label is bound; the final one falls through
            // to the match-failure call below
            self.asm.bind_label(nexts[i]);
        }
        self.asm.ldr_slot(X0, sv);
        self.call_rt("dc_match_fail");
        self.asm.bind_label(end);
    }

    fn new_label(&mut self) -> u32 {
        self.asm.new_label()
    }

    fn new_fn_label(&mut self, sym: String, exported: bool) -> u32 {
        let l = self.asm.new_label();
        self.label_syms.insert(l, (sym, exported));
        l
    }

    // ---------------- expressions ----------------

    fn expr(&mut self, f: &mut FnState, e: &Expr) {
        match e {
            Expr::Lit(Lit::Int(n)) => {
                self.asm.load_const_i64(X0, (*n << 2) | 1);
            }
            Expr::Lit(Lit::Bool(b)) => {
                self.asm.load_const_i64(X0, if *b { 7 } else { 3 });
            }
            Expr::Lit(Lit::Str(s)) => {
                let sym = self.intern_str(s);
                self.asm.ldr_lit_extern(X0, &sym);
            }
            Expr::Lit(Lit::Unit) => self.load_unit(X0),
            Expr::Var(n) => {
                if !self.load_var(f, n) {
                    panic!("codegen: unbound variable `{}`", n);
                }
            }
            Expr::Ctor(name) => self.load_ctor_value(name),
            Expr::App(fe, ae) => {
                self.expr(f, fe);
                let sf = Self::spill_slot(f);
                self.asm.str_slot(X0, sf);
                self.expr(f, ae);
                self.asm.mov(X1, X0);
                self.asm.ldr_slot(X0, sf);
                self.emit_apply();
            }
            Expr::Bin(op, l, r) => self.binop(f, *op, l, r),
            Expr::If(c, t, el) => {
                self.expr(f, c);
                let else_l = self.new_label();
                let end_l = self.new_label();
                self.asm.cmp_imm(X0, 7); // true?
                self.asm.b_cond(Cond::Ne, else_l);
                self.expr(f, t);
                self.asm.b(end_l);
                self.asm.bind_label(else_l);
                self.expr(f, el);
                self.asm.bind_label(end_l);
            }
            Expr::Case(scrut, branches) => self.case_expr(f, scrut, branches),
            Expr::Let { is_rec, name, rhs, body } => {
                if *is_rec {
                    self.local_rec_let(f, name, rhs, body);
                } else {
                    self.expr(f, rhs);
                    let s = Self::spill_slot(f);
                    self.asm.str_slot(X0, s);
                    f.scope.push((name.clone(), s));
                    self.expr(f, body);
                }
            }
            Expr::Seq(l, r) => {
                self.expr(f, l);
                self.expr(f, r);
            }
            Expr::Tuple(es) => {
                let mut slots = Vec::new();
                for el in es {
                    self.expr(f, el);
                    let s = Self::spill_slot(f);
                    self.asm.str_slot(X0, s);
                    slots.push(s);
                }
                let words = 1 + es.len() as u64;
                self.asm.load_const_u64(X0, words * 8);
                self.call_rt("dacelo_alloc");
                self.asm.load_const_u64(crate::encoder::R9, hdr(TAG_TUPLE, words));
                self.asm.str_off(crate::encoder::R9, X0, 0);
                for (i, s) in slots.iter().enumerate() {
                    self.asm.ldr_slot(crate::encoder::R9, *s);
                    self.asm.str_off(crate::encoder::R9, X0, 8 * (i as i64 + 1));
                }
            }
            Expr::Ann(inner, _) => self.expr(f, inner),
            Expr::Lam(..) => self.lambda(f, e),
        }
    }

    fn binop(&mut self, f: &mut FnState, op: BinOp, l: &Expr, r: &Expr) {
        use BinOp::*;
        match op {
            And | Or => {
                self.expr(f, l);
                let short = self.new_label();
                let end = self.new_label();
                self.asm.cmp_imm(X0, 7); // == true?
                if op == And {
                    self.asm.b_cond(Cond::Ne, short);
                } else {
                    self.asm.b_cond(Cond::Eq, short);
                }
                self.expr(f, r);
                self.asm.b(end);
                self.asm.bind_label(short);
                self.asm.load_const_i64(X0, if op == And { 3 } else { 7 });
                self.asm.bind_label(end);
            }
            Eq | Neq => {
                self.expr(f, l);
                let sa = Self::spill_slot(f);
                self.asm.str_slot(X0, sa);
                self.expr(f, r);
                self.asm.mov(X1, X0);
                self.asm.ldr_slot(X0, sa);
                self.call_rt("dc_val_eq"); // bool immediate in x0
                if op == Neq {
                    let t = self.new_label();
                    let e = self.new_label();
                    self.asm.cmp_imm(X0, 7);
                    self.asm.b_cond(Cond::Eq, t);
                    self.asm.load_const_i64(X0, 7);
                    self.asm.b(e);
                    self.asm.bind_label(t);
                    self.asm.load_const_i64(X0, 3);
                    self.asm.bind_label(e);
                }
            }
            Lt | Gt | Le | Ge | Add | Sub | Mul | Div | Mod => {
                self.expr(f, l);
                let la = Self::spill_slot(f);
                self.asm.str_slot(X0, la);
                self.expr(f, r);
                let rb = Self::spill_slot(f);
                self.asm.str_slot(X0, rb);
                self.int_arith(op, la, rb);
            }
            Concat => {
                self.expr(f, l);
                let la = Self::spill_slot(f);
                self.asm.str_slot(X0, la);
                self.expr(f, r);
                self.asm.mov(X1, X0);
                self.asm.ldr_slot(X0, la);
                self.call_rt("dc_bi_str_concat");
            }
            Cons => {
                // build ADT cell Cons(head, tail): [hdr][ctor_id][head][tail]
                let cons_id = self.ctors["Cons"].0;
                let words = 4u64;
                self.expr(f, l);
                let la = Self::spill_slot(f);
                self.asm.str_slot(X0, la);
                self.expr(f, r);
                let rb = Self::spill_slot(f);
                self.asm.str_slot(X0, rb);
                self.asm.load_const_u64(X0, words * 8);
                self.call_rt("dacelo_alloc");
                self.asm.load_const_u64(crate::encoder::R9, hdr(TAG_ADT, words));
                self.asm.str_off(crate::encoder::R9, X0, 0);
                self.asm.load_const_u64(crate::encoder::R9, cons_id as u64);
                self.asm.str_off(crate::encoder::R9, X0, 8);
                self.asm.ldr_slot(crate::encoder::R9, la);
                self.asm.str_off(crate::encoder::R9, X0, 16);
                self.asm.ldr_slot(crate::encoder::R9, rb);
                self.asm.str_off(crate::encoder::R9, X0, 24);
            }
        }
    }

    fn int_arith(&mut self, op: BinOp, la: u32, rb: u32) {
        use BinOp::*;
        let R9 = crate::encoder::R9;
        let R10 = crate::encoder::R10;
        let XZ = crate::encoder::XZR;
        match op {
            Lt | Gt | Le | Ge => {
                self.asm.ldr_slot(X0, la);
                self.asm.asr_imm(X0, X0, 2);
                self.asm.ldr_slot(R9, rb);
                self.asm.asr_imm(R9, R9, 2);
                self.asm.cmp_reg(X0, R9);
                let cond = match op {
                    Lt => Cond::Lt,
                    Gt => Cond::Gt,
                    Le => Cond::Le,
                    Ge => Cond::Ge,
                    _ => unreachable!(),
                };
                self.asm.cset(X0, cond);
                self.asm.add_reg(X0, XZ, X0, 2); // << 2
                self.asm.add_imm(X0, X0, 3);     // bool tag
            }
            Add => {
                self.asm.ldr_slot(X0, la);
                self.asm.ldr_slot(X1, rb);
                self.asm.add_reg(X0, X0, X1, 0);
                self.asm.sub_imm(X0, X0, 1);
            }
            Sub => {
                self.asm.ldr_slot(X0, la);
                self.asm.ldr_slot(X1, rb);
                self.asm.sub_reg(X0, X0, X1);
                self.asm.add_imm(X0, X0, 1);
            }
            Mul => {
                self.asm.ldr_slot(X0, la);
                self.asm.asr_imm(X0, X0, 2);
                self.asm.ldr_slot(X1, rb);
                self.asm.asr_imm(X1, X1, 2);
                self.asm.mul(X0, X0, X1);
                self.asm.add_reg(X0, XZ, X0, 2);
                self.asm.add_imm(X0, X0, 1);
            }
            Div => {
                self.asm.ldr_slot(X0, rb);
                self.call_rt("dc_div_check"); // unboxed nonzero b in x0
                self.asm.mov(R10, X0);
                self.asm.ldr_slot(X0, la);
                self.asm.asr_imm(X0, X0, 2);
                self.asm.sdiv(X0, X0, R10);
                self.asm.add_reg(X0, XZ, X0, 2);
                self.asm.add_imm(X0, X0, 1);
            }
            Mod => {
                self.asm.ldr_slot(X0, rb);
                self.call_rt("dc_div_check"); // unboxed nonzero b in x0
                self.asm.mov(R10, X0); // divisor
                self.asm.ldr_slot(X0, la);
                self.asm.asr_imm(X0, X0, 2); // dividend
                self.asm.sdiv(11, X0, R10); // quotient -> x11
                self.asm.msub(X0, R10, 11, X0); // a - b*q
                self.asm.add_reg(X0, XZ, X0, 2);
                self.asm.add_imm(X0, X0, 1);
            }
            _ => unreachable!(),
        }
    }

    // ---------------- functions ----------------

    /// lambda expression inside a body: create closure + defer inner codegen
    fn lambda(&mut self, f: &mut FnState, lam: &Expr) {
        let mut params = Vec::new();
        let mut cur = lam;
        while let Expr::Lam(p, inner) = cur {
            params.push(p.clone());
            cur = inner;
        }
        let body: Expr = cur.clone();

        let captured_slots: Vec<(String, u32)> =
            f.scope.iter().map(|(n, s)| (n.clone(), *s)).collect();
        let captured_names: Vec<String> =
            captured_slots.iter().map(|(n, _)| n.clone()).collect();

        self.lam_counter += 1;
        let base = self.lam_counter;
        let labels: Vec<u32> = (0..params.len()).map(|_| self.new_label()).collect();
        let syms: Vec<String> = (0..params.len())
            .map(|k| if k == 0 { format!("_L{base}") } else { format!("_L{base}__{k}") })
            .collect();
        for k in 0..labels.len() {
            self.label_syms.insert(labels[k], (syms[k].clone(), false));
        }

        self.make_closure(labels[0], &captured_slots);
        self.pending.push(PendingFn {
            labels,
            syms,
            captured_names,
            params,
            body,
        });
    }

    /// local single-function recursive binding
    fn local_rec_let(&mut self, f: &mut FnState, name: &str, rhs: &Expr, body: &Expr) {
        let mut params = Vec::new();
        let mut cur = rhs;
        while let Expr::Lam(p, inner) = cur {
            params.push(p.clone());
            cur = inner;
        }
        let real_body: Expr = cur.clone();

        // capture list = visible locals + the recursive name itself
        let mut captured: Vec<(String, Option<u32>)> =
            f.scope.iter().map(|(n, s)| (n.clone(), Some(*s))).collect();
        captured.push((name.to_string(), None));
        let ncap = captured.len() as u64;

        self.lam_counter += 1;
        let base = self.lam_counter;
        let labels: Vec<u32> = (0..params.len()).map(|_| self.new_label()).collect();
        let syms: Vec<String> = (0..params.len())
            .map(|k| if k == 0 { format!("_L{base}") } else { format!("_L{base}__{k}") })
            .collect();
        for k in 0..labels.len() {
            self.label_syms.insert(labels[k], (syms[k].clone(), false));
        }

        // allocate closure manually so we can back-patch the self reference
        let words = 3 + ncap;
        self.asm.load_const_u64(X0, words * 8);
        self.call_rt("dacelo_alloc");
        self.asm.load_const_u64(crate::encoder::R9, hdr(TAG_CLOSURE, words));
        self.asm.str_off(crate::encoder::R9, X0, 0);
        self.asm.ldr_lit_label(crate::encoder::R9, labels[0]);
        self.asm.str_off(crate::encoder::R9, X0, 8);
        self.asm.load_const_u64(crate::encoder::R9, ncap);
        self.asm.str_off(crate::encoder::R9, X0, 16);
        let self_idx = captured
            .iter()
            .position(|(nm, _)| nm == name)
            .expect("self in captures");
        for (j, (_nm, src)) in captured.iter().enumerate() {
            if j == self_idx {
                continue;
            }
            self.asm.ldr_slot(crate::encoder::R9, src.unwrap());
            self.asm.str_off(crate::encoder::R9, X0, 24 + 8 * j as i64);
        }
        // back-patch self reference
        self.asm.str_off(X0, X0, 24 + 8 * self_idx as i64);

        // expose closure value under its name for subsequent code
        let holder = Self::spill_slot(f);
        self.asm.str_slot(X0, holder);
        f.scope.push((name.to_string(), holder));

        self.pending.push(PendingFn {
            labels,
            syms,
            captured_names: captured.into_iter().map(|(nm, _)| nm).collect(),
            params,
            body: real_body,
        });

        self.expr(f, body);
    }

    fn drain_pending(&mut self) {
        while let Some(pf) = self.pending.pop() {
            self.emit_fn_levels(pf);
        }
    }

    fn emit_fn_levels(&mut self, pf: PendingFn) {
        let n = pf.params.len();
        for k in 0..n {
            let mut caps_now: Vec<String> = pf.captured_names.clone();
            for p in pf.params[..k].iter() {
                if let Some(nm) = pat_name(p) {
                    caps_now.push(nm);
                }
            }
            self.asm.bind_label(pf.labels[k]);
            // label_syms registered at creation time
            self.fn_prologue(caps_now.len());
            let mut fst = FnState::fresh();
            for (i, nm) in caps_now.iter().enumerate() {
                fst.scope.push((nm.clone(), i as u32));
            }
            fst.nslots = caps_now.len() as u32;

            let fail = self.new_label();
            self.match_pat(&mut fst, &pf.params[k], X1, fail);
            if k + 1 < n {
                let caps: Vec<(String, u32)> =
                    fst.scope.iter().map(|(a, b)| (a.clone(), *b)).collect();
                self.make_closure(pf.labels[k + 1], &caps);
            } else {
                self.expr(&mut fst, &pf.body);
            }
            self.fn_epilogue(fst.nslots);
            self.asm.bind_label(fail);
            self.asm.mov(X0, X1);
            self.call_rt("dc_match_fail");
        }
    }

    /// top-level definition: emit thunk computing its value
    fn compile_def(&mut self, d: &Def) -> u32 {
        let sym = format!("_thunk_{}", d.name);
        let label = self.new_fn_label(sym, false);
        self.asm.bind_label(label);
        self.fn_prologue(0);
        let mut fst = FnState::fresh();
        let wrapped = wrap_lams(&d.params, d.body.clone());
        self.expr(&mut fst, &wrapped);
        self.fn_epilogue(fst.nslots);
        label
    }

    fn emit_init(&mut self, thunks: &[(u32 /*slot*/, u32 /*label*/)]) {
        let l = self.new_fn_label("_dc_init".to_string(), true);
        self.asm.bind_label(l);
        self.fn_prologue(0);

        // register constructors
        for (id, cname) in self.ctor_names.clone().iter().enumerate() {
            let arity = self.ctors[cname].1;
            self.asm.load_const_u64(X0, id as u64);
            self.asm.load_const_u64(X1, arity as u64);
            self.call_rt("dc_register_ctor");
        }
        // builtin partial applications
        for (idx, bname) in BUILTINS.iter().enumerate() {
            let slot = self.globals[*bname];
            self.asm.load_const_u64(X0, idx as u64);
            self.call_rt("dc_mk_partial_builtin");
            self.fill_global(slot);
        }
        // constructor partial applications
        let names_snapshot = self.ctor_names.clone();
        for cname in &names_snapshot {
            let (id, arity) = self.ctors[cname];
            if arity > 0 {
                let slot = self.globals[cname];
                self.asm.load_const_u64(X0, id as u64);
                self.call_rt("dc_mk_partial_ctor");
                self.fill_global(slot);
            }
        }
        // user definitions
        for (slot, lbl) in thunks {
            self.asm.bl(*lbl);
            self.fill_global(*slot);
        }

        self.fn_epilogue(0);
    }

    fn emit_user_main(&mut self, main_g: Option<u32>) {
        let l = self.new_fn_label("_dc_user_main".to_string(), true);
        self.asm.bind_label(l);
        self.fn_prologue(0);
        if let Some(g) = main_g {
            self.emit_gget(g);
            self.load_unit(X1);
            self.emit_apply();
        }
        self.fn_epilogue(0);
    }

    // ---------------- program ----------------

    pub fn compile_program(mut self, prog: &Program) -> ObjectBuilder {
        // global slots + thunks, in declaration order
        let mut thunks: Vec<(u32, u32)> = Vec::new();
        for item in &prog.items {
            match item {
                Item::Def(d) => {
                    let slot = self.alloc_global(&d.name);
                    let lbl = self.compile_def(d);
                    thunks.push((slot, lbl));
                }
                Item::RecGroup(defs) => {
                    for d in defs {
                        let slot = self.alloc_global(&d.name);
                        let lbl = self.compile_def(d);
                        thunks.push((slot, lbl));
                    }
                }
                Item::Ty(_) => {}
            }
        }

        self.drain_pending();

        let main_g = self.globals.get("main").cloned();
        self.emit_init(&thunks);
        self.emit_user_main(main_g);
        self.emit_ctor_names_table();

        let label_names = |lid: u32| -> String {
            self.label_syms.get(&lid).expect("label known").0.clone()
        };
        let mut asm = CodeBuf::new();
        std::mem::swap(&mut asm, &mut self.asm);
        let finished = asm.finish(&label_names);

        let mut obj = ObjectBuilder::new();
        obj.text = finished.code;
        for (lid, off) in finished.label_offsets {
            if let Some((sym, glob)) = self.label_syms.get(&lid).cloned() {
                obj.define(&sym, Section::Text, off as u64, glob);
            }
        }

        for ext in &self.externs {
            obj.add_extern(ext);
        }

        // __got entries: one quad per slot with UNSIGNED relocation
        let got_off = self.data.len() as u64;
        for (slot, (_key, target_name)) in finished.got_slots.iter().enumerate() {
            let qoff = self.data.len() as u64;
            self.data.extend_from_slice(&[0u8; 8]);
            if let Some(rest) = _key.strip_prefix("L:") {
                let lid: u32 = rest.parse().unwrap();
                let (sym, _) = self.label_syms.get(&lid).cloned().expect("got label");
                self.data_relocs.push(Reloc {
                    offset: qoff,
                    sym_name: sym,
                    kind: crate::macho::RelocKind::Unsigned,
                });
            } else {
                self.use_extern(target_name);
                self.data_relocs.push(Reloc {
                    offset: qoff,
                    sym_name: target_name.clone(),
                    kind: crate::macho::RelocKind::Unsigned,
                });
            }
            let _ = slot;
        }

        // text relocations for each ADRP/LDR pair -- straight @GOT loads of the
        // target symbol; ld synthesizes the got entry itself
        let mut refs = finished.adrp_refs.clone();
        refs.sort_by_key(|r| r.0);
        for (_key, target_name) in &finished.got_slots {
            if !_key.starts_with("L:") {
                self.use_extern(target_name);
            }
        }
        for (pos, _reg, slot) in refs {
            let target = finished.got_slots[slot].1.clone();
            obj.relocs_text.push(Reloc {
                offset: pos as u64,
                sym_name: target.clone(),
                kind: crate::macho::RelocKind::GotLoadPage21,
            });
            obj.relocs_text.push(Reloc {
                offset: (pos + 4) as u64,
                sym_name: target,
                kind: crate::macho::RelocKind::GotLoadPageOff12,
            });
        }
        if !finished.got_slots.is_empty() {
            self.data_syms.push(("_dc_got".to_string(), got_off));
        }

        obj.data = std::mem::take(&mut self.data);
        for (sym, off) in &self.data_syms {
            let glob = sym == "_dc_ctor_names";
            obj.define(sym, Section::Data, *off, glob);
        }
        obj.relocs_data = std::mem::take(&mut self.data_relocs);
        obj
    }
}
