// dacelo Gen 0: Hindley-Milner type inference (Algorithm J)

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::ast::*;
use crate::parser::format_tyast;

// ---------- type representation ----------

#[derive(Clone)]
pub enum Ty {
    Con(&'static str), // "Int" "Bool" "String" "Unit"
    Var(Tv),
    Arrow(Rc<Ty>, Rc<Ty>),
    Tuple(Vec<Rc<Ty>>),
    /// named application: List [t], Tree [], Pair [a, b]
    App(String, Vec<Rc<Ty>>),
}

pub struct TvCell {
    id: usize,
    link: RefCell<Option<Ty>>,
}

#[derive(Clone)]
pub struct Tv(Rc<TvCell>);

impl Ty {
    pub fn con(s: &'static str) -> Ty {
        Ty::Con(s)
    }
    pub fn int() -> Ty {
        Ty::con("Int")
    }
    pub fn bool() -> Ty {
        Ty::con("Bool")
    }
    pub fn string() -> Ty {
        Ty::con("String")
    }
    pub fn unit() -> Ty {
        Ty::con("Unit")
    }
    pub fn list(elem: Ty) -> Ty {
        Ty::App("List".into(), vec![Rc::new(elem)])
    }
    pub fn arrow(a: Ty, b: Ty) -> Ty {
        Ty::Arrow(Rc::new(a), Rc::new(b))
    }
}

#[derive(Clone)]
pub struct Scheme {
    pub qvars: BTreeSet<usize>,
    pub ty: Ty,
}

fn prune(t: &Ty) -> Ty {
    match t {
        Ty::Var(tv) => {
            let link = tv.0.link.borrow().clone();
            match link {
                Some(t2) => prune(&t2),
                None => t.clone(),
            }
        }
        other => other.clone(),
    }
}

pub struct Infer {
    counter: Cell<usize>,
    pub env: HashMap<String, Scheme>,
    /// type constructor arities: name -> number of params
    pub tydefs: HashMap<String, usize>,
    /// runtime constructor table: name -> (tag, arity)
    pub ctors: HashMap<String, (u32, usize)>,
    next_tag: u32,
    cur_def: RefCell<String>,
    app_ctr: Cell<usize>,
}

type TResult<T> = Result<T, String>;

impl Infer {
    pub fn new() -> Self {
        let mut inf = Infer {
            counter: Cell::new(0),
            env: HashMap::new(),
            tydefs: HashMap::new(),
            ctors: HashMap::new(),
            next_tag: 0,
            cur_def: RefCell::new("?".into()),
            app_ctr: Cell::new(0),
        };
        // built-in List ADT: type List a = Nil | Cons a (List a)
        inf.tydefs.insert("List".into(), 1);
        let a = inf.fresh();
        let nil_ty = Ty::list(Ty::Var(a.clone()));
        inf.env.insert(
            "Nil".into(),
            Scheme { qvars: free_vars_of(&nil_ty), ty: nil_ty },
        );
        inf.ctors.insert("Nil".into(), (inf.next_tag, 0));
        inf.next_tag += 1;
        let a = inf.fresh();
        let cons_ty = Ty::arrow(
            Ty::Var(a.clone()),
            Ty::arrow(Ty::list(Ty::Var(a.clone())), Ty::list(Ty::Var(a))),
        );
        inf.env.insert(
            "Cons".into(),
            Scheme { qvars: free_vars_of(&cons_ty), ty: cons_ty },
        );
        inf.ctors.insert("Cons".into(), (inf.next_tag, 2));
        inf.next_tag += 1;

        let b = [
            ("print_int", Ty::arrow(Ty::int(), Ty::unit())),
            ("print_string", Ty::arrow(Ty::string(), Ty::unit())),
            ("int_to_string", Ty::arrow(Ty::int(), Ty::string())),
            ("bool_to_string", Ty::arrow(Ty::bool(), Ty::string())),
            ("string_length", Ty::arrow(Ty::string(), Ty::int())),
            ("str_concat", Ty::arrow(Ty::string(), Ty::arrow(Ty::string(), Ty::string()))),
            ("read_file", Ty::arrow(Ty::string(), Ty::string())),
            ("write_file", Ty::arrow(Ty::string(), Ty::arrow(Ty::string(), Ty::unit()))),
            ("exit", Ty::arrow(Ty::int(), Ty::unit())),
            ("chr", Ty::arrow(Ty::int(), Ty::string())),
            ("ord", Ty::arrow(Ty::string(), Ty::int())),
            ("string_get", Ty::arrow(Ty::string(), Ty::arrow(Ty::int(), Ty::int()))),
            ("substring", Ty::arrow(
                Ty::string(),
                Ty::arrow(Ty::int(), Ty::arrow(Ty::int(), Ty::string())),
            )),
            ("string_to_int", Ty::arrow(Ty::string(), Ty::int())),
            ("argv", Ty::arrow(Ty::int(), Ty::string())),
            ("system", Ty::arrow(Ty::string(), Ty::int())),
        ];
        for (name, ty) in b {
            inf.env.insert(name.into(), Scheme { qvars: BTreeSet::new(), ty });
        }
        // error : forall a. String -> a
        let a = inf.fresh();
        let err_ty = Ty::arrow(Ty::string(), Ty::Var(a));
        inf.env.insert(
            "error".into(),
            Scheme { qvars: free_vars_of(&err_ty), ty: err_ty },
        );
        // show : forall a. a -> String
        let a = inf.fresh();
        let show_ty_ = Ty::arrow(Ty::Var(a), Ty::string());
        inf.env.insert(
            "show".into(),
            Scheme { qvars: free_vars_of(&show_ty_), ty: show_ty_ },
        );
        inf
    }

    fn fresh(&self) -> Tv {
        let id = self.counter.get();
        self.counter.set(id + 1);
        if std::env::var("DACELO_VARBIRTH").is_ok() {
            eprintln!("VARBIRTH {} @ {}", id, self.cur_def.borrow());
        }
        Tv(Rc::new(TvCell { id, link: RefCell::new(None) }))
    }

    // ---------- unification ----------

    fn occurs(&self, id: usize, t: &Ty) -> bool {
        match prune(t) {
            Ty::Var(tv) => tv.0.id == id,
            Ty::Con(_) => false,
            Ty::Arrow(a, b) => self.occurs(id, &a) || self.occurs(id, &b),
            Ty::Tuple(ts) => ts.iter().any(|x| self.occurs(id, x)),
            Ty::App(_, ts) => ts.iter().any(|x| self.occurs(id, x)),
        }
    }

    pub fn unify(&self, a: &Ty, b: &Ty) -> TResult<()> {
        let a = prune(a);
        let b = prune(b);
        match (&a, &b) {
            (Ty::Var(va), Ty::Var(vb)) if va.0.id == vb.0.id => Ok(()),
            (Ty::Var(va), _) => {
                if self.occurs(va.0.id, &b) {
                    Err(format!("infinite type: cannot unify `{}` with `{}`", self.show(&a), self.show(&b)))
                } else {
                    if std::env::var("DACELO_TRACE").is_ok() {
                        eprintln!("[link@{}] #{} := {}", self.cur_def.borrow(), va.0.id, self.show(&b));
                    }
                    *va.0.link.borrow_mut() = Some(b.clone());
                    Ok(())
                }
            }
            (_, Ty::Var(_)) => self.unify(&b, &a),
            (Ty::Con(x), Ty::Con(y)) if x == y => Ok(()),
            (Ty::Arrow(a1, a2), Ty::Arrow(b1, b2)) => {
                self.unify(a1, b1)?;
                self.unify(a2, b2)
            }
            (Ty::Tuple(xs), Ty::Tuple(ys)) if xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (Ty::App(n1, xs), Ty::App(n2, ys)) if n1 == n2 && xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            _ => Err(format!("type mismatch: expected `{}`, found `{}`", self.show(&b), self.show(&a))),
        }
    }

    // ---------- generalization / instantiation ----------

    fn free_in_env(&self, env: &HashMap<String, Scheme>) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for sch in env.values() {
            let mut fv = BTreeSet::new();
            collect_free(&sch.ty, &mut fv);
            for v in fv {
                if !sch.qvars.contains(&v) {
                    out.insert(v);
                }
            }
        }
        out
    }

    fn generalize(&self, env: &HashMap<String, Scheme>, t: &Ty) -> Scheme {
        let mut fv = BTreeSet::new();
        collect_free(t, &mut fv);
        let env_fv = self.free_in_env(env);
        let qvars: BTreeSet<usize> = fv.difference(&env_fv).cloned().collect();
        Scheme { qvars, ty: t.clone() }
    }

    fn instantiate(&self, s: &Scheme) -> Ty {
        inst(&s.ty, &s.qvars, &self)
    }

    // ---------- type display ----------

    pub fn show(&self, t: &Ty) -> String {
        let mut names = HashMap::new();
        show_ty(&prune(t), &mut names)
    }

    // ---------- source type conversion ----------

    fn resolve_tyast(&self, ast: &TyAst, scope: &HashMap<String, Tv>) -> TResult<Ty> {
        match ast {
            TyAst::TVar(v) => scope
                .get(v)
                .map(|tv| Ty::Var(tv.clone()))
                .ok_or_else(|| format!("unbound type variable `{}`", v)),
            TyAst::TCon(name, args) => {
                if name == "Int" && args.is_empty() {
                    return Ok(Ty::int());
                }
                if name == "Bool" && args.is_empty() {
                    return Ok(Ty::bool());
                }
                if name == "String" && args.is_empty() {
                    return Ok(Ty::string());
                }
                if name == "Unit" && args.is_empty() {
                    return Ok(Ty::unit());
                }
                if name == "List" {
                    if args.len() != 1 {
                        return Err(format!("`List` expects exactly one argument"));
                    }
                    let e = self.resolve_tyast(&args[0], scope)?;
                    return Ok(Ty::list(e));
                }
                match self.tydefs.get(name.as_str()) {
                    Some(ar) if *ar == args.len() => {
                        let mut as_ = Vec::new();
                        for a in args {
                            as_.push(Rc::new(self.resolve_tyast(a, scope)?));
                        }
                        Ok(Ty::App(name.clone(), as_))
                    }
                    Some(ar) => Err(format!(
                        "type `{}` expects {} arguments, got {}",
                        name,
                        ar,
                        args.len()
                    )),
                    None => Err(format!("unknown type `{}`", name)),
                }
            }
            TyAst::Arrow(a, b) => Ok(Ty::arrow(
                self.resolve_tyast(a, scope)?,
                self.resolve_tyast(b, scope)?,
            )),
            TyAst::TTuple(ts) => {
                let mut out = Vec::new();
                for t in ts {
                    out.push(Rc::new(self.resolve_tyast(t, scope)?));
                }
                Ok(Ty::Tuple(out))
            }
        }
    }

    fn fresh_scope_for_ann(&self, ast: &TyAst, scope: &mut HashMap<String, Tv>) {
        fn walk(ast: &TyAst, scope: &mut HashMap<String, Tv>, inf: &Infer) {
            match ast {
                TyAst::TVar(v) => {
                    scope.entry(v.clone()).or_insert_with(|| inf.fresh());
                }
                TyAst::TCon(_, args) | TyAst::TTuple(args) => {
                    for a in args {
                        walk(a, scope, inf);
                    }
                }
                TyAst::Arrow(a, b) => {
                    walk(a, scope, inf);
                    walk(b, scope, inf);
                }
            }
        }
        walk(ast, scope, self);
    }

    // ---------- program checking ----------

    /// scan env for schemes containing FREE (unquantified) type variables --
    /// these share global var nodes and cause cross-definition contamination
    pub fn scan_leaks(&self) {
        use std::collections::BTreeSet;
        for (name, sch) in &self.env {
            let mut fv = BTreeSet::new();
            collect_free(&sch.ty, &mut fv);
            let leaked: Vec<_> = fv.iter().filter(|v| !sch.qvars.contains(v)).collect();
            if !leaked.is_empty() {
                eprintln!(
                    "LEAK {} : {}   [free-but-unquantified: {:?}]",
                    name,
                    self.show(&sch.ty),
                    leaked
                );
            }
        }
    }

    pub fn process_item(&mut self, item: &Item) -> TResult<()> {
        match item {
            Item::Ty(td) => self.check_tydef(td),
            Item::Def(d) => {
                let schs = self.infer_def_group(std::slice::from_ref(d))?;
                for (name, sch) in schs {
                    self.env.insert(name, sch);
                }
                Ok(())
            }
            Item::RecGroup(defs) => {
                let schs = self.infer_def_group(defs)?;
                for (name, sch) in schs {
                    self.env.insert(name, sch);
                }
                Ok(())
            }
        }
    }

    pub fn check_program(&mut self, prog: &Program) -> TResult<()> {
        for item in &prog.items {
            self.process_item(item)?;
        }
        Ok(())
    }

    fn check_tydef(&mut self, td: &TyDecl) -> TResult<()> {
        if matches!(
            td.name.as_str(),
            "Int" | "Bool" | "String" | "Unit" | "List"
        ) {
            return Err(format!("cannot redefine built-in type `{}`", td.name));
        }
        if self.tydefs.contains_key(&td.name) {
            return Err(format!("duplicate type definition `{}`", td.name));
        }
        let mut scope: HashMap<String, Tv> = HashMap::new();
        for p in &td.params {
            scope.insert(p.clone(), self.fresh());
        }
        self.tydefs.insert(td.name.clone(), td.params.len());

        let mut ctor_schemes = Vec::new();
        for cd in &td.ctors {
            let mut field_tys = Vec::new();
            for f in &cd.fields {
                field_tys.push(self.resolve_tyast(f, &scope)?);
            }
            let mut res_params = Vec::new();
            for p in &td.params {
                res_params.push(Rc::new(Ty::Var(scope[p].clone())));
            }
            let mut t = Ty::App(td.name.clone(), res_params);
            for f in field_tys.iter().rev() {
                t = Ty::arrow(f.clone(), t);
            }
            let sch = Scheme { qvars: free_vars_of(&t), ty: t };
            ctor_schemes.push((cd.name.clone(), sch));
        }
        for (name, sch) in ctor_schemes {
            let arity = count_arrows(&sch.ty);
            let tag = self.next_tag;
            self.next_tag += 1;
            self.ctors.insert(name.clone(), (tag, arity));
            self.env.insert(name, sch);
        }
        Ok(())
    }

    /// infer one def or a mutually recursive group (`let rec ... and ...`)
    fn infer_def_group(&mut self, defs: &[Def]) -> TResult<Vec<(String, Scheme)>> {
        // duplicate name check
        for (i, d) in defs.iter().enumerate() {
            for (j, e) in defs.iter().enumerate() {
                if i < j && d.name == e.name {
                    return Err(format!("duplicate definition `{}`", d.name));
                }
            }
        }
        let outer = self.env.clone();

        // 1. parameter types & pattern bindings per def
        let mut all_param_tys: Vec<Vec<Tv>> = Vec::new();
        let mut all_binds: Vec<HashMap<String, Scheme>> = Vec::new();
        for d in defs {
            let mut param_tys = Vec::new();
            let mut binds: HashMap<String, Scheme> = HashMap::new();
            for p in &d.params {
                let tv = self.fresh();
                param_tys.push(tv.clone());
                self.pat_type(p, &Ty::Var(tv), &mut binds)?;
            }
            all_param_tys.push(param_tys);
            all_binds.push(binds);
        }

        // 2. monomorphic placeholders for every member (rec group)
        let mut result_tvs = Vec::new();
        for (i, d) in defs.iter().enumerate() {
            if !d.is_rec && defs.len() > 1 {
                return Err(format!("`{}`: all members of an `and` group must be recursive", d.name));
            }
            let rtv = self.fresh();
            result_tvs.push(rtv.clone());
            if d.is_rec {
                let mut t = Ty::Var(rtv);
                for pt in all_param_tys[i].iter().rev() {
                    t = Ty::arrow(Ty::Var(pt.clone()), t);
                }
                self.env
                    .insert(d.name.clone(), Scheme { qvars: BTreeSet::new(), ty: t });
            }
        }

        // 3. (removed) parameter bindings are injected per-member during
        // step 4 so each body sees ONLY its own parameters. Injecting every
        // member's binds up front let same-named parameters of other members
        // leak into unrelated bodies (e.g. `toks`/`acc`/`f`) and poison the
        // shared unification variables.

        // 4. infer bodies, unify with placeholders.
        // Each member's body sees ONLY its own parameter bindings (members
        // often reuse names like `toks`/`acc`); the rec placeholders of all
        // members stay visible throughout.
        let mut body_tys = Vec::new();
        for (i, d) in defs.iter().enumerate() {
            *self.cur_def.borrow_mut() = d.name.clone();
            let pre = self.env.clone();
            for (k, v) in &all_binds[i] {
                self.env.insert(k.clone(), v.clone());
            }
            let bt = self.expr_type(&d.body)
                .map_err(|e| format!("in def `{}`: {}", d.name, e))?;
            self.env = pre;
            body_tys.push(bt.clone());
            if d.is_rec {
                let mut full = bt;
                for pt in all_param_tys[i].iter().rev() {
                    full = Ty::arrow(Ty::Var(pt.clone()), full);
                }
                let recorded = self.env.get(&d.name).unwrap().ty.clone();
                self.unify(&recorded, &full)
                    .map_err(|e| format!("in `{}`: {}", d.name, e))?;
            }
        }

        // 5. type annotations
        for (i, d) in defs.iter().enumerate() {
            if let Some(ann) = &d.ann {
                let mut scope = HashMap::new();
                self.fresh_scope_for_ann(ann, &mut scope);
                let ann_ty = self.resolve_tyast(ann, &scope)?;
                let mut full = body_tys[i].clone();
                for pt in all_param_tys[i].iter().rev() {
                    full = Ty::arrow(Ty::Var(pt.clone()), full);
                }
                self.unify(&full, &ann_ty)
                    .map_err(|e| format!("annotation on `{}`: {}", d.name, e))?;
            }
        }

        // 6. restore outer env and generalize each member
        self.env = outer.clone();
        let mut out = Vec::new();
        for (i, d) in defs.iter().enumerate() {
            let mut full = body_tys[i].clone();
            for pt in all_param_tys[i].iter().rev() {
                full = Ty::arrow(Ty::Var(pt.clone()), full);
            }
            if std::env::var("DACELO_DEBUG").is_ok() {
                eprintln!("[dbg] def {}: full={}", d.name, self.show(&full));
            }
            out.push((d.name.clone(), self.generalize(&outer, &full)));
        }
        Ok(out)
    }

    // ---------- pattern typing ----------

    fn pat_type(&self, pat: &Pattern, ty: &Ty, binds: &mut HashMap<String, Scheme>) -> TResult<()> {
        match pat {
            Pattern::Wildcard => Ok(()),
            Pattern::Var(n) => {
                binds.insert(n.clone(), Scheme { qvars: BTreeSet::new(), ty: ty.clone() });
                Ok(())
            }
            Pattern::PLit(Lit::Int(_)) => self.unify(ty, &Ty::int()),
            Pattern::PLit(Lit::Bool(_)) => self.unify(ty, &Ty::bool()),
            Pattern::PLit(Lit::Str(_)) => self.unify(ty, &Ty::string()),
            Pattern::PLit(Lit::Unit) => self.unify(ty, &Ty::unit()),
            Pattern::PTuple(ps) => {
                let ts: Vec<Rc<Ty>> = ps.iter().map(|_| Rc::new(Ty::Var(self.fresh()))).collect();
                self.unify(ty, &Ty::Tuple(ts.clone()))?;
                for (p, t) in ps.iter().zip(ts.iter()) {
                    self.pat_type(p, t, binds)?;
                }
                Ok(())
            }
            Pattern::PCtor(name, ps) => {
                let sch = self
                    .env
                    .get(name)
                    .ok_or_else(|| format!("unknown constructor `{}`", name))?
                    .clone();
                let instantiated = self.instantiate(&sch);
                // decompose arg types
                let mut args = Vec::new();
                let mut cur = instantiated;
                while let Ty::Arrow(a, b) = cur {
                    args.push((*a).clone());
                    cur = (*b).clone();
                }
                if args.len() != ps.len() {
                    return Err(format!(
                        "constructor `{}` expects {} argument(s), pattern gives {}",
                        name,
                        args.len(),
                        ps.len()
                    ));
                }
                self.unify(ty, &cur)?;
                for (p, t) in ps.iter().zip(args.iter()) {
                    self.pat_type(p, t, binds)?;
                }
                Ok(())
            }
        }
    }

    // ---------- expression typing ----------

    pub fn expr_type(&mut self, e: &Expr) -> TResult<Ty> {
        match e {
            Expr::Lit(Lit::Int(_)) => Ok(Ty::int()),
            Expr::Lit(Lit::Bool(_)) => Ok(Ty::bool()),
            Expr::Lit(Lit::Str(_)) => Ok(Ty::string()),
            Expr::Lit(Lit::Unit) => Ok(Ty::unit()),
            Expr::Var(n) | Expr::Ctor(n) => {
                let sch = self
                    .env
                    .get(n)
                    .ok_or_else(|| format!("unbound variable `{}`", n))?
                    .clone();
                Ok(self.instantiate(&sch))
            }
            Expr::Lam(p, body) => {
                let tv = self.fresh();
                let mut binds = HashMap::new();
                self.pat_type(p, &Ty::Var(tv.clone()), &mut binds)?;
                let outer = self.env.clone();
                for (k, v) in binds {
                    self.env.insert(k, v);
                }
                let bt = self.expr_type(body)?;
                self.env = outer;
                Ok(Ty::arrow(Ty::Var(tv), bt))
            }
            Expr::App(f, x) => {
                let tf = self.expr_type(f)?;
                let tx = self.expr_type(x)?;
                let r = self.fresh();
                let ctr = {
                    let c = self.app_ctr.get();
                    self.app_ctr.set(c + 1);
                    c
                };
                self.unify(&tf, &Ty::arrow(tx, Ty::Var(r.clone())))
                    .map_err(|e| {
                        if std::env::var("DACELO_DEBUG").is_ok() {
                            format!("in application #{} `{:#?}` applied to `{:#?}`: {}", ctr, f, x, e)
                        } else {
                            format!("in application #{}: {}", ctr, e)
                        }
                    })?;
                Ok(Ty::Var(r))
            }
            Expr::Bin(op, l, r) => {
                let tl = self.expr_type(l)?;
                let tr = self.expr_type(r)?;
                use BinOp::*;
                match op {
                    Add | Sub | Mul | Div | Mod => {
                        self.bin_check(op_name(*op), &tl, &tr, Ty::int())?;
                        Ok(Ty::int())
                    }
                    Concat => {
                        self.bin_check(op_name(*op), &tl, &tr, Ty::string())?;
                        Ok(Ty::string())
                    }
                    Cons => {
                        let a = self.fresh();
                        let tl_expect = Ty::Var(a.clone());
                        let tr_expect = Ty::list(Ty::Var(a));
                        self.unify(&tl, &tl_expect)
                            .map_err(|e| format!("`::`: {}", e))?;
                        self.unify(&tr, &tr_expect)
                            .map_err(|e| format!("`::`: {}", e))?;
                        Ok(tr_expect)
                    }
                    Eq | Neq => {
                        self.unify(&tl, &tr)
                            .map_err(|e| format!("`{}`: {}", op_name(*op), e))?;
                        Ok(Ty::bool())
                    }
                    Lt | Gt | Le | Ge => {
                        self.bin_check(op_name(*op), &tl, &tr, Ty::int())?;
                        Ok(Ty::bool())
                    }
                    And | Or => {
                        self.bin_check(op_name(*op), &tl, &tr, Ty::bool())?;
                        Ok(Ty::bool())
                    }
                }
            }
            Expr::If(c, t, e) => {
                let tc = self.expr_type(c)?;
                self.unify(&tc, &Ty::bool()).map_err(|_| "`if` condition must be Bool".to_string())?;
                let tt = self.expr_type(t)?;
                let te = self.expr_type(e)?;
                self.unify(&tt, &te)
                    .map_err(|e| format!("if branches differ: {}", e))?;
                Ok(tt)
            }
            Expr::Case(scrut, branches) => {
                let st = self.expr_type(scrut)?;
                if std::env::var("DACELO_DEBUG").is_ok() {
                    eprintln!("[dbg] case scrut={}", self.show(&st));
                }
                let result = self.fresh();
                let outer = self.env.clone();
                for (i, (pat, body)) in branches.iter().enumerate() {
                    let mut binds = HashMap::new();
                    self.pat_type(pat, &st, &mut binds)
                        .map_err(|e| format!("case branch {}: {}", i, e))?;
                    for (k, v) in binds {
                        self.env.insert(k, v);
                    }
                    let bt = self.expr_type(body)?;
                    self.env = outer.clone();
                    if std::env::var("DACELO_DEBUG").is_ok() {
                        eprintln!("[dbg]   branch {}: pat_ty={}, body={}", i, self.show(&st), self.show(&bt));
                    }
                    self.unify(&bt, &Ty::Var(result.clone()))
                        .map_err(|e| format!("case branch {}: {}", i, e))?;
                }
                Ok(Ty::Var(result))
            }
            Expr::Let { is_rec, name, rhs, body } => {
                let outer = self.env.clone();
                let rt = if *is_rec {
                    let tv = self.fresh();
                    self.env
                        .insert(name.clone(), Scheme { qvars: BTreeSet::new(), ty: Ty::Var(tv.clone()) });
                    let t = self.expr_type(rhs)?;
                    let recorded = self.env.get(name).unwrap().ty.clone();
                    self.unify(&recorded, &t)?;
                    t
                } else {
                    self.expr_type(rhs)?
                };
                let sch = self.generalize(&outer, &rt);
                self.env = outer;
                self.env.insert(name.clone(), sch);
                self.expr_type(body)
            }
            Expr::Seq(l, r) => {
                let lt = self.expr_type(l)?;
                self.unify(&lt, &Ty::unit())
                    .map_err(|e| format!("left side of `;` must be Unit: {}", e))?;
                self.expr_type(r)
            }
            Expr::Tuple(es) => {
                let mut ts = Vec::new();
                for x in es {
                    ts.push(Rc::new(self.expr_type(x)?));
                }
                Ok(Ty::Tuple(ts))
            }
            Expr::Ann(e, t) => {
                let mut scope = HashMap::new();
                self.fresh_scope_for_ann(t, &mut scope);
                let at = self.resolve_tyast(t, &scope)?;
                let et = self.expr_type(e)?;
                self.unify(&et, &at)?;
                Ok(at)
            }
        }
    }

    fn bin_check(&self, op: &str, tl: &Ty, tr: &Ty, want: Ty) -> TResult<()> {
        self.unify(tl, &want).map_err(|_| {
            format!(
                "operator `{}` expects `{}` operands, got left `{}`",
                op,
                self.show(&want),
                self.show(tl)
            )
        })?;
        self.unify(tr, &want).map_err(|_| {
            format!(
                "operator `{}` expects `{}` operands, got right `{}`",
                op,
                self.show(&want),
                self.show(tr)
            )
        })
    }
}

fn op_name(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Concat => "++",
        Cons => "::",
        Eq => "==",
        Neq => "!=",
        Lt => "<",
        Gt => ">",
        Le => "<=",
        Ge => ">=",
        And => "&&",
        Or => "||",
    }
}

fn collect_free(t: &Ty, out: &mut BTreeSet<usize>) {
    match prune(t) {
        Ty::Var(tv) => {
            out.insert(tv.0.id);
        }
        Ty::Con(_) => {}
        Ty::Arrow(a, b) => {
            collect_free(&a, out);
            collect_free(&b, out);
        }
        Ty::Tuple(ts) => {
            for x in ts {
                collect_free(&x, out);
            }
        }
        Ty::App(_, ts) => {
            for x in ts {
                collect_free(&x, out);
            }
        }
    }
}

fn free_vars_of(t: &Ty) -> BTreeSet<usize> {
    let mut s = BTreeSet::new();
    collect_free(t, &mut s);
    s
}

fn count_arrows(t: &Ty) -> usize {
    match prune(t) {
        Ty::Arrow(_, b) => 1 + count_arrows(&b),
        _ => 0,
    }
}

fn inst(t: &Ty, qvars: &BTreeSet<usize>, inf: &Infer) -> Ty {
    fn go(t: &Ty, map: &mut HashMap<usize, Ty>, qvars: &BTreeSet<usize>, inf: &Infer) -> Ty {
        let t = prune(t);
        match &t {
            Ty::Var(tv) => {
                if qvars.contains(&tv.0.id) {
                    map.entry(tv.0.id).or_insert_with(|| Ty::Var(inf.fresh())).clone()
                } else {
                    t
                }
            }
            Ty::Con(_) => t,
            Ty::Arrow(a, b) => Ty::Arrow(Rc::new(go(a, map, qvars, inf)), Rc::new(go(b, map, qvars, inf))),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|x| Rc::new(go(x, map, qvars, inf))).collect()),
            Ty::App(n, ts) => Ty::App(n.clone(), ts.iter().map(|x| Rc::new(go(x, map, qvars, inf))).collect()),
        }
    }
    let mut map = HashMap::new();
    go(t, &mut map, qvars, inf)
}

/// render a type with variable names a, b, c...
pub fn show_ty(t: &Ty, names: &mut HashMap<usize, String>) -> String {
    let t = &prune(t);
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let name_for = |id: usize, names: &mut HashMap<usize, String>| -> String {
        if let Some(n) = names.get(&id) {
            return n.clone();
        }
        let n = names.len();
        let s = if n < LETTERS.len() {
            (LETTERS[n] as char).to_string()
        } else {
            format!("{}{}", LETTERS[n % LETTERS.len()] as char, n / LETTERS.len())
        };
        names.insert(id, s.clone());
        s
    };
    match t {
        Ty::Var(tv) => name_for(tv.0.id, names),
        Ty::Con(s) => s.to_string(),
        Ty::Arrow(a, b) => {
            let lhs = match &**a {
                Ty::Arrow(..) => format!("({})", show_ty(a, names)),
                other => show_ty(other, names),
            };
            format!("{} -> {}", lhs, show_ty(b, names))
        }
        Ty::Tuple(ts) => {
            let inner: Vec<String> = ts.iter().map(|x| show_ty(x, names)).collect();
            format!("({})", inner.join(", "))
        }
        Ty::App(n, ts) if ts.is_empty() => n.clone(),
        Ty::App(n, ts) if n == "List" => format!("[{}]", show_ty(&ts[0], names)),
        Ty::App(n, ts) => {
            let inner: Vec<String> = ts.iter().map(|x| show_ty(x, names)).collect();
            format!("({} {})", n, inner.join(" "))
        }
    }
}

// silence unused warning for parser re-export usage
#[allow(dead_code)]
fn _unused(t: &TyAst) -> String {
    format_tyast(t)
}
