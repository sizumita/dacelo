// dacelo Gen 0: tree-walking evaluator

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;

extern "C" {
    fn system(cmd: *const std::ffi::c_char) -> i32;
}

// ---------- values ----------

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(Rc<String>),
    Unit,
    Tuple(Rc<Vec<Value>>),
    Fun(Rc<Callable>),
    /// fully applied ADT constructor value (also lists: Nil / Cons)
    Adt(Rc<CtorVal>),
}

#[derive(Clone)]
pub struct ClosureData {
    /// mutually recursive group: filled after construction, bound into env
    /// at every call. Shared by all members of the group.
    pub group: Option<Rc<RefCell<Vec<(String, Value)>>>>,
    pub pat: Pattern,
    pub body: Rc<Expr>,
    pub env: Env,
}

#[derive(Clone)]
pub enum Callable {
    Closure(ClosureData),
    Builtin { kind: BuiltinKind, args: Vec<Value> },
    CtorPartial { name: String, tag: u32, arity: usize, args: Vec<Value> },
}

#[derive(Clone)]
pub struct CtorVal {
    pub name: String,
    pub tag: u32,
    pub fields: Vec<Value>,
}

// ---------- environment ----------

#[derive(Clone)]
pub struct Env(Option<Rc<EnvNode>>);

struct EnvNode {
    parent: Env,
    name: String,
    val: Value,
}

impl Env {
    pub fn empty() -> Env {
        Env(None)
    }

    pub fn bind(&self, name: &str, val: Value) -> Env {
        Env(Some(Rc::new(EnvNode {
            parent: self.clone(),
            name: name.to_string(),
            val,
        })))
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        let mut cur = self;
        loop {
            match cur {
                Env(Some(node)) => {
                    if node.name == name {
                        return Some(node.val.clone());
                    }
                    cur = &node.parent;
                }
                Env(None) => return None,
            }
        }
    }
}

// ---------- output ----------

#[derive(Clone)]
pub enum OutSink {
    Stdout,
    Buffer(Rc<RefCell<Vec<u8>>>),
}

impl OutSink {
    fn write(&self, s: &str) {
        match self {
            OutSink::Stdout => print!("{}", s),
            OutSink::Buffer(b) => b.borrow_mut().extend_from_slice(s.as_bytes()),
        }
    }
}

// ---------- builtins ----------

#[derive(Clone, Copy)]
pub enum BuiltinKind {
    PrintInt,
    PrintStr,
    IntToStr,
    BoolToStr,
    StrLen,
    ReadFile,
    WriteFile,
    Exit,
    Chr,
    Ord,
    StrGet,
    Substring,
    StrToInt,
    Error,
    Show,
    Argv,
    System,
}

impl BuiltinKind {
    fn arity(self) -> usize {
        match self {
            BuiltinKind::WriteFile => 2,
            BuiltinKind::StrGet => 2,
            BuiltinKind::Substring => 3,
            _ => 1,
        }
    }

    pub const ALL: &'static [(BuiltinKind, &'static str)] = &[
        (BuiltinKind::PrintInt, "print_int"),
        (BuiltinKind::PrintStr, "print_string"),
        (BuiltinKind::IntToStr, "int_to_string"),
        (BuiltinKind::BoolToStr, "bool_to_string"),
        (BuiltinKind::StrLen, "string_length"),
        (BuiltinKind::ReadFile, "read_file"),
        (BuiltinKind::WriteFile, "write_file"),
        (BuiltinKind::Exit, "exit"),
        (BuiltinKind::Chr, "chr"),
        (BuiltinKind::Ord, "ord"),
        (BuiltinKind::StrGet, "string_get"),
        (BuiltinKind::Substring, "substring"),
        (BuiltinKind::StrToInt, "string_to_int"),
        (BuiltinKind::Error, "error"),
        (BuiltinKind::Show, "show"),
        (BuiltinKind::Argv, "argv"),
        (BuiltinKind::System, "system"),
    ];
}

// ---------- machine ----------

pub struct Machine {
    pub out: OutSink,
    /// constructor name -> (tag, arity)
    pub ctors: HashMap<String, (u32, usize)>,
}

type EResult = Result<Value, String>;
type GResult = Result<Env, String>;

impl Machine {
    pub fn new(out: OutSink, ctors: HashMap<String, (u32, usize)>) -> Machine {
        Machine { out, ctors }
    }

    // ---------- program execution ----------

    pub fn run_items(&mut self, prog: &Program) -> EResult {
        let mut globals = Env::empty();
        // seed builtin function values
        for (kind, name) in BuiltinKind::ALL {
            globals = globals.bind(
                name,
                Value::Fun(Rc::new(Callable::Builtin { kind: *kind, args: vec![] })),
            );
        }
        for item in &prog.items {
            match item {
                Item::Ty(_) => {}
                Item::Def(d) => {
                    globals = self.define_rec_group(std::slice::from_ref(d), &globals)?;
                }
                Item::RecGroup(defs) => {
                    globals = self.define_rec_group(defs, &globals)?;
                }
            }
        }
        if let Some(mainv) = globals.lookup("main") {
            if matches!(mainv, Value::Fun(_)) {
                self.apply(&mainv, Value::Unit)?;
            }
        }
        Ok(Value::Unit)
    }

    /// define one def or a `let rec ... and ...` group in the global env
    fn define_rec_group(&mut self, defs: &[Def], globals: &Env) -> GResult {
        // duplicate name check
        for (i, d) in defs.iter().enumerate() {
            for (j, e) in defs.iter().enumerate() {
                if i < j && d.name == e.name {
                    return Err(format!("duplicate definition `{}`", d.name));
                }
            }
        }
        let group_cell = Rc::new(RefCell::new(Vec::new()));
        let mut pairs: Vec<(String, Value)> = Vec::new();
        for d in defs {
            let wrapped = wrap_params_ast(d.params.clone(), d.body.clone());
            let v = self.eval(&wrapped, globals)?;
            let v = match v {
                Value::Fun(rc) => {
                    let mut c = (*rc).clone();
                    if let Callable::Closure(cd) = &mut c {
                        cd.group = Some(group_cell.clone());
                    } else {
                        return Err(format!("`let rec {}` must be a function", d.name));
                    }
                    Value::Fun(Rc::new(c))
                }
                other => other,
            };
            pairs.push((d.name.clone(), v));
        }
        *group_cell.borrow_mut() = pairs.clone();
        let mut env = globals.clone();
        for (name, v) in pairs {
            env = env.bind(&name, v);
        }
        Ok(env)
    }

    // ---------- application ----------

    fn apply(&mut self, f: &Value, arg: Value) -> EResult {
        match f {
            Value::Fun(rc) => match &**rc {
                Callable::Closure(cd) => {
                    let mut base = cd.env.clone();
                    if let Some(group) = &cd.group {
                        for (n, v) in group.borrow().iter() {
                            base = base.bind(n, v.clone());
                        }
                    }
                    let binds = self.match_pattern(&cd.pat, &arg)?;
                    let mut env = base;
                    for (n, v) in binds {
                        env = env.bind(&n, v);
                    }
                    self.eval(&cd.body, &env)
                }
                Callable::Builtin { kind, args } => {
                    let mut args = args.clone();
                    args.push(arg);
                    if args.len() == kind.arity() {
                        self.call_builtin(*kind, args)
                    } else {
                        Ok(Value::Fun(Rc::new(Callable::Builtin { kind: *kind, args })))
                    }
                }
                Callable::CtorPartial { name, tag, arity, args } => {
                    let mut args = args.clone();
                    args.push(arg);
                    if args.len() == *arity {
                        Ok(Value::Adt(Rc::new(CtorVal {
                            name: name.clone(),
                            tag: *tag,
                            fields: args,
                        })))
                    } else {
                        Ok(Value::Fun(Rc::new(Callable::CtorPartial {
                            name: name.clone(),
                            tag: *tag,
                            arity: *arity,
                            args,
                        })))
                    }
                }
            },
            other => Err(format!("cannot apply non-function value {}", show_value(other))),
        }
    }

    fn call_builtin(&mut self, kind: BuiltinKind, args: Vec<Value>) -> EResult {
        use BuiltinKind::*;
        match kind {
            PrintInt => {
                if let Value::Int(n) = &args[0] {
                    self.out.write(&n.to_string());
                }
                Ok(Value::Unit)
            }
            PrintStr => {
                if let Value::Str(s) = &args[0] {
                    let s = s.as_ref().clone();
                    self.out.write(&s);
                }
                Ok(Value::Unit)
            }
            IntToStr => match &args[0] {
                Value::Int(n) => Ok(Value::Str(Rc::new(n.to_string()))),
                _ => unreachable!(),
            },
            BoolToStr => match &args[0] {
                Value::Bool(b) => Ok(Value::Str(Rc::new(b.to_string()))),
                _ => unreachable!(),
            },
            StrLen => match &args[0] {
                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                _ => unreachable!(),
            },
            ReadFile => match &args[0] {
                Value::Str(p) => {
                    std::fs::read_to_string(p.as_ref().clone())
                        .map(|s| Value::Str(Rc::new(s)))
                        .map_err(|e| format!("read_file: {}", e))
                }
                _ => unreachable!(),
            },
            WriteFile => match (&args[0], &args[1]) {
                (Value::Str(p), Value::Str(data)) => {
                    let p = p.as_ref().clone();
                    let data = data.as_ref().clone();
                    std::fs::write(p, data.as_bytes())
                        .map(|_| Value::Unit)
                        .map_err(|e| format!("write_file: {}", e))
                }
                _ => unreachable!(),
            },
            Exit => match &args[0] {
                Value::Int(n) => std::process::exit(*n as i32),
                _ => unreachable!(),
            },
            Chr => match &args[0] {
                Value::Int(n) => {
                    let c = u32::try_from(*n)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| format!("chr: {} is not a valid character", n))?;
                    Ok(Value::Str(Rc::new(c.to_string())))
                }
                _ => unreachable!(),
            },
            Ord => match &args[0] {
                Value::Str(s) => Ok(Value::Int(s.as_bytes().first().map(|b| *b as i64).unwrap_or(-1))),
                _ => unreachable!(),
            },
            StrGet => match (&args[0], &args[1]) {
                (Value::Str(s), Value::Int(i)) => {
                    let bytes = s.as_bytes();
                    let idx = *i;
                    if idx < 0 || idx as usize >= bytes.len() {
                        Ok(Value::Int(-1))
                    } else {
                        Ok(Value::Int(bytes[idx as usize] as i64))
                    }
                }
                _ => unreachable!(),
            },
            Substring => match (&args[0], &args[1], &args[2]) {
                (Value::Str(s), Value::Int(start), Value::Int(len)) => {
                    let start = *start;
                    let len = *len;
                    if start < 0 || len < 0 || start as usize > s.len() || start + len > s.len() as i64 {
                        return Err(format!(
                            "substring: range [{},{}) out of bounds for length {}",
                            start,
                            start + len,
                            s.len()
                        ));
                    }
                    let st = start as usize;
                    let en = (start + len) as usize;
                    if !s.is_char_boundary(st) || !s.is_char_boundary(en) {
                        return Err("substring: not a char boundary".into());
                    }
                    Ok(Value::Str(Rc::new(s[st..en].to_string())))
                }
                _ => unreachable!(),
            },
            StrToInt => match &args[0] {
                Value::Str(s) => s.trim().parse::<i64>().map(Value::Int).map_err(|e| {
                    format!("string_to_int: cannot parse {:?}: {}", s, e)
                }),
                _ => unreachable!(),
            },
            Error => match &args[0] {
                Value::Str(s) => Err(format!("error: {}", s)),
                _ => unreachable!(),
            },
            Show => Ok(Value::Str(Rc::new(show_value(&args[0])))),
            Argv => match &args[0] {
                Value::Int(i) => {
                    // argv 0 is the running script itself; user args follow
                    // (`dacelo script.dc a b` => script sees argv 0=script,
                    //  1=a, 2=b)
                    let s = std::env::args()
                        .nth((*i + 1) as usize)
                        .unwrap_or_default();
                    Ok(Value::Str(Rc::new(s)))
                }
                _ => unreachable!(),
            },
            System => match &args[0] {
                Value::Str(cmd) => {
                    let cstr = std::ffi::CString::new(cmd.as_ref().clone())
                        .expect("system: bad command");
                    let rc = unsafe { system(cstr.as_ptr()) };
                    Ok(Value::Int(rc as i64))
                }
                _ => unreachable!(),
            },
        }
    }

    // ---------- pattern matching ----------

    fn match_pattern(
        &self,
        pat: &Pattern,
        val: &Value,
    ) -> Result<Vec<(String, Value)>, String> {
        let mut binds = Vec::new();
        if self.pat_match(pat, val, &mut binds)? {
            Ok(binds)
        } else {
            Err(format!("pattern match failed: {}", show_value(val)))
        }
    }

    fn pat_match(
        &self,
        pat: &Pattern,
        val: &Value,
        binds: &mut Vec<(String, Value)>,
    ) -> Result<bool, String> {
        match pat {
            Pattern::Wildcard => Ok(true),
            Pattern::Var(n) => {
                binds.push((n.clone(), val.clone()));
                Ok(true)
            }
            Pattern::PLit(Lit::Int(n)) => Ok(matches!(val, Value::Int(x) if x == n)),
            Pattern::PLit(Lit::Bool(b)) => Ok(matches!(val, Value::Bool(x) if x == b)),
            Pattern::PLit(Lit::Str(s)) => Ok(matches!(val, Value::Str(x) if **x == *s)),
            Pattern::PLit(Lit::Unit) => Ok(matches!(val, Value::Unit)),
            Pattern::PTuple(ps) => match val {
                Value::Tuple(vs) if vs.len() == ps.len() => {
                    for (p, v) in ps.iter().zip(vs.iter()) {
                        if !self.pat_match(p, v, binds)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                _ => Ok(false),
            },
            Pattern::PCtor(name, ps) => {
                let expected_tag = self
                    .ctors
                    .get(name)
                    .ok_or_else(|| format!("unknown constructor `{}`", name))?
                    .0;
                match val {
                    Value::Adt(cv) if cv.tag == expected_tag && cv.fields.len() == ps.len() => {
                        for (p, v) in ps.iter().zip(cv.fields.iter()) {
                            if !self.pat_match(p, v, binds)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    // ---------- expression evaluation ----------

    pub fn eval(&mut self, e: &Expr, env: &Env) -> EResult {
        match e {
            Expr::Lit(Lit::Int(n)) => Ok(Value::Int(*n)),
            Expr::Lit(Lit::Bool(b)) => Ok(Value::Bool(*b)),
            Expr::Lit(Lit::Str(s)) => Ok(Value::Str(Rc::new(s.clone()))),
            Expr::Lit(Lit::Unit) => Ok(Value::Unit),
            Expr::Var(n) => env
                .lookup(n)
                .ok_or_else(|| format!("unbound variable `{}`", n)),
            Expr::Ctor(name) => {
                let (tag, arity) = *self
                    .ctors
                    .get(name)
                    .ok_or_else(|| format!("unknown constructor `{}`", name))?;
                if arity == 0 {
                    Ok(Value::Adt(Rc::new(CtorVal {
                        name: name.clone(),
                        tag,
                        fields: vec![],
                    })))
                } else {
                    Ok(Value::Fun(Rc::new(Callable::CtorPartial {
                        name: name.clone(),
                        tag,
                        arity,
                        args: vec![],
                    })))
                }
            }
            Expr::Lam(pat, body) => Ok(Value::Fun(Rc::new(Callable::Closure(ClosureData {
                group: None,
                pat: pat.clone(),
                body: Rc::new((**body).clone()),
                env: env.clone(),
            })))),
            Expr::App(f, x) => {
                let fv = self.eval(f, env)?;
                let xv = self.eval(x, env)?;
                self.apply(&fv, xv)
            }
            Expr::Bin(op, l, r) => {
                use BinOp::*;
                match op {
                    And => {
                        let lv = self.eval(l, env)?;
                        if let Value::Bool(false) = lv {
                            return Ok(Value::Bool(false));
                        }
                        let rv = self.eval(r, env)?;
                        match rv {
                            Value::Bool(b) => Ok(Value::Bool(b)),
                            _ => unreachable!(),
                        }
                    }
                    Or => {
                        let lv = self.eval(l, env)?;
                        if let Value::Bool(true) = lv {
                            return Ok(Value::Bool(true));
                        }
                        let rv = self.eval(r, env)?;
                        match rv {
                            Value::Bool(b) => Ok(Value::Bool(b)),
                            _ => unreachable!(),
                        }
                    }
                    _ => {
                        let lv = self.eval(l, env)?;
                        let rv = self.eval(r, env)?;
                        self.binop(*op, lv, rv)
                    }
                }
            }
            Expr::If(c, t, e) => {
                let cv = self.eval(c, env)?;
                match cv {
                    Value::Bool(true) => self.eval(t, env),
                    Value::Bool(false) => self.eval(e, env),
                    _ => unreachable!(),
                }
            }
            Expr::Case(scrut, branches) => {
                let sv = self.eval(scrut, env)?;
                for (pat, body) in branches {
                    let mut binds = Vec::new();
                    if self.pat_match(pat, &sv, &mut binds)? {
                        let mut benv = env.clone();
                        for (n, v) in binds {
                            benv = benv.bind(&n, v);
                        }
                        return self.eval(body, &benv);
                    }
                }
                Err(format!("non-exhaustive pattern match on {}", show_value(&sv)))
            }
            Expr::Let { is_rec, name, rhs, body } => {
                let rv = self.eval(rhs, env)?;
                let rv = if *is_rec {
                    match rv {
                        Value::Fun(rc) => {
                            // local single-function recursion: shared group
                            // cell filled with the patched closure itself
                            let cell = Rc::new(RefCell::new(Vec::new()));
                            let mut c = (*rc).clone();
                            if let Callable::Closure(cd) = &mut c {
                                cd.group = Some(cell.clone());
                            } else {
                                return Err(format!("`let rec {}` must be a function", name));
                            }
                            let patched = Value::Fun(Rc::new(c));
                            *cell.borrow_mut() = vec![(name.clone(), patched.clone())];
                            patched
                        }
                        other => other,
                    }
                } else {
                    rv
                };
                let nenv = env.bind(name, rv);
                self.eval(body, &nenv)
            }
            Expr::Seq(l, r) => {
                self.eval(l, env)?;
                self.eval(r, env)
            }
            Expr::Tuple(es) => {
                let mut vs = Vec::new();
                for x in es {
                    vs.push(self.eval(x, env)?);
                }
                Ok(Value::Tuple(Rc::new(vs)))
            }
            Expr::Ann(e, _) => self.eval(e, env),
        }
    }

    fn binop(&mut self, op: BinOp, l: Value, r: Value) -> EResult {
        use BinOp::*;
        match op {
            Add => int_op(l, r, |a, b| a.wrapping_add(b)),
            Sub => int_op(l, r, |a, b| a.wrapping_sub(b)),
            Mul => int_op(l, r, |a, b| a.wrapping_mul(b)),
            Div => int_op_checked(l, r, |a, b| {
                if b == 0 {
                    Err("division by zero".into())
                } else {
                    Ok(a.wrapping_div(b))
                }
            }),
            Mod => int_op_checked(l, r, |a, b| {
                if b == 0 {
                    Err("modulo by zero".into())
                } else {
                    Ok(a.wrapping_rem(b))
                }
            }),
            Concat => match (&l, &r) {
                (Value::Str(a), Value::Str(b)) => {
                    let s = format!("{}{}", a, b);
                    Ok(Value::Str(Rc::new(s)))
                }
                _ => unreachable!(),
            },
            Cons => Ok(Value::Adt(Rc::new(CtorVal {
                name: "Cons".into(),
                tag: self.ctors.get("Cons").unwrap().0,
                fields: vec![l, r],
            }))),
            Eq | Neq => {
                let eq = eq_value(&l, &r)?;
                Ok(Value::Bool(if op == Eq { eq } else { !eq }))
            }
            Lt | Gt | Le | Ge => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => {
                    let res = match op {
                        Lt => a < b,
                        Gt => a > b,
                        Le => a <= b,
                        Ge => a >= b,
                        _ => unreachable!(),
                    };
                    Ok(Value::Bool(res))
                }
                _ => unreachable!(),
            },
            And | Or => unreachable!(),
        }
    }
}

/// wrap parameter patterns into a lambda chain (runtime counterpart of
/// multi-parameter definitions)
fn wrap_params_ast(params: Vec<Pattern>, mut body: Expr) -> Expr {
    for p in params.into_iter().rev() {
        body = Expr::Lam(p, Box::new(body));
    }
    body
}

fn int_op(l: Value, r: Value, f: impl FnOnce(i64, i64) -> i64) -> EResult {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(f(a, b))),
        _ => unreachable!(),
    }
}

fn int_op_checked(l: Value, r: Value, f: impl FnOnce(i64, i64) -> Result<i64, String>) -> EResult {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => f(a, b).map(Value::Int),
        _ => unreachable!(),
    }
}

fn eq_value(l: &Value, r: &Value) -> Result<bool, String> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(a == b),
        (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
        (Value::Str(a), Value::Str(b)) => Ok(a == b),
        (Value::Unit, Value::Unit) => Ok(true),
        (Value::Tuple(xs), Value::Tuple(ys)) => {
            if xs.len() != ys.len() {
                return Ok(false);
            }
            for (x, y) in xs.iter().zip(ys.iter()) {
                if !eq_value(x, y)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Adt(x), Value::Adt(y)) => {
            if x.tag != y.tag {
                return Ok(false);
            }
            for (a, b) in x.fields.iter().zip(y.fields.iter()) {
                if !eq_value(a, b)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Fun(_), _) | (_, Value::Fun(_)) => {
            Err("cannot compare functions with == ".into())
        }
        _ => Ok(false),
    }
}

/// pretty printer used by `show` and runtime errors
pub fn show_value(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => format!("{:?}", s),
        Value::Unit => "()".into(),
        Value::Tuple(vs) => {
            let inner: Vec<String> = vs.iter().map(show_value).collect();
            format!("({})", inner.join(", "))
        }
        Value::Adt(cv) if cv.name == "Cons" || cv.name == "Nil" => {
            let mut elems = Vec::new();
            let mut cur = v.clone();
            loop {
                match cur {
                    Value::Adt(cv) if cv.name == "Cons" => {
                        elems.push(show_value(&cv.fields[0]));
                        cur = cv.fields[1].clone();
                    }
                    Value::Adt(cv) if cv.name == "Nil" => break,
                    other => {
                        elems.push(format!("...{}", show_value(&other)));
                        break;
                    }
                }
            }
            format!("[{}]", elems.join(", "))
        }
        Value::Adt(cv) => {
            if cv.fields.is_empty() {
                cv.name.clone()
            } else {
                let inner: Vec<String> = cv.fields.iter().map(show_value).collect();
                format!("({} {})", cv.name, inner.join(" "))
            }
        }
        Value::Fun(_) => "<fun>".into(),
    }
}
