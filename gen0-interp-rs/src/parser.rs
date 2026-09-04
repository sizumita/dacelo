// dacelo Gen 0 parser: recursive descent

use crate::ast::*;
use crate::lexer::{Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

type PResult<T> = Result<T, String>;

pub fn parse(toks: Vec<Token>) -> PResult<Program> {
    let mut p = Parser { toks, pos: 0 };
    let mut items = Vec::new();
    loop {
        match p.peek_tok() {
            Tok::Eof => break,
            _ => items.push(p.parse_item()?),
        }
    }
    Ok(Program { items })
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_tok(&self) -> &Tok {
        &self.peek().tok
    }

    fn peek2_tok(&self) -> &Tok {
        let i = (self.pos + 1).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn next(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, msg: impl std::fmt::Display) -> PResult<T> {
        let t = self.peek();
        Err(format!("{}:{}: {} (found {})", t.line, t.col, msg, t.tok))
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if matches!(self.peek_tok(), Tok::Sym(x) if *x == s) {
            self.next();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: &'static str) -> bool {
        if matches!(self.peek_tok(), Tok::Kw(x) if *x == k) {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, s: &str) -> PResult<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            self.err(format!("expected `{}`", s))
        }
    }

    fn expect_kw(&mut self, k: &'static str) -> PResult<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected `{}`", k))
        }
    }

    // ---- items ----

    fn parse_item(&mut self) -> PResult<Item> {
        match self.peek_tok() {
            Tok::Kw("let") => {
                let _ = self.expect_kw("let");
                let is_rec = self.eat_kw("rec");
                let first = self.parse_let_tail(is_rec)?;
                if is_rec && matches!(self.peek_tok(), Tok::Kw("and")) {
                    let mut group = vec![first];
                    while self.eat_kw("and") {
                        group.push(self.parse_let_tail(true)?);
                    }
                    Ok(Item::RecGroup(group))
                } else if matches!(self.peek_tok(), Tok::Kw("and")) {
                    self.err("`and` is only allowed after `let rec`")
                } else {
                    Ok(Item::Def(first))
                }
            }
            Tok::Kw("type") => self.parse_tydef(),
            _ => self.err("expected item (`let` or `type`)"),
        }
    }

    fn parse_tydef(&mut self) -> PResult<Item> {
        let _ = self.expect_kw("type");
        let name = self.parse_upper()?;
        let mut params = Vec::new();
        while matches!(self.peek_tok(), Tok::Ident(_)) {
            params.push(self.parse_ident()?);
        }
        self.expect_sym("=")?;
        let _ = self.eat_sym("|");
        let mut ctors = Vec::new();
        loop {
            let cname = self.parse_upper()?;
            let mut fields = Vec::new();
            while self.starts_type_atom() {
                fields.push(self.parse_atomty()?);
            }
            ctors.push(CtorDecl { name: cname, fields });
            if !self.eat_sym("|") {
                break;
            }
        }
        Ok(Item::Ty(TyDecl { name, params, ctors }))
    }

    fn starts_type_atom(&self) -> bool {
        matches!(self.peek_tok(), Tok::Ident(_) | Tok::Upper(_) | Tok::Sym("(") | Tok::Sym("["))
    }

    /// top-level def has no `in`; local def requires `in`
    /// (assumes the leading `let` and optional `rec` are already consumed)
    fn parse_let_tail(&mut self, is_rec: bool) -> PResult<Def> {
        let name = self.parse_ident()?;
        let mut params = Vec::new();
        while matches!(
            self.peek_tok(),
            Tok::Ident(_) | Tok::Sym("_") | Tok::Sym("(")
        ) {
            params.push(self.parse_pat_atom()?);
        }
        let ann = if self.eat_sym(":") {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect_sym("=")?;
        let body = self.parse_expr()?;
        Ok(Def { name, params, ann, body, is_rec })
    }

    /// local let: consumes `let` and requires `in`
    fn parse_local_def(&mut self) -> PResult<Expr> {
        self.expect_kw("let")?;
        // `let (a, b) = rhs in body`: pattern binding, desugars to case
        if matches!(self.peek_tok(), Tok::Sym("(")) {
            let pat = self.parse_pat_atom()?;
            self.expect_sym("=")?;
            let rhs = self.parse_expr()?;
            self.expect_kw("in")?;
            let body = self.parse_expr()?;
            return Ok(Expr::Case(Box::new(rhs), vec![(pat, body)]));
        }
        let is_rec = self.eat_kw("rec");
        let d = self.parse_let_tail(is_rec)?;
        if matches!(self.peek_tok(), Tok::Kw("and")) {
            return Err(format!(
                "{}:{}: `and` groups are not supported in local lets yet",
                self.peek().line,
                self.peek().col
            ));
        }
        self.expect_kw("in")?;
        let rest = self.parse_expr()?;
        let rhs = wrap_params(d.params, d.body);
        Ok(Expr::Let { is_rec, name: d.name, rhs: Box::new(rhs), body: Box::new(rest) })
    }

    // ---- expressions ----

    pub fn parse_expr(&mut self) -> PResult<Expr> {
        match self.peek_tok() {
            Tok::Kw("fun") => {
                self.next();
                let mut ps = vec![self.parse_pat_atom()?];
                while matches!(self.peek_tok(), Tok::Ident(_) | Tok::Sym("_") | Tok::Sym("(")) {
                    ps.push(self.parse_pat_atom()?);
                }
                self.expect_sym("->")?;
                let body = self.parse_expr()?;
                Ok(wrap_params(ps, body))
            }
            Tok::Kw("if") => {
                self.next();
                let cond = self.parse_expr()?;
                self.expect_kw("then")?;
                let th = self.parse_expr()?;
                self.expect_kw("else")?;
                let el = self.parse_expr()?;
                Ok(Expr::If(Box::new(cond), Box::new(th), Box::new(el)))
            }
            Tok::Kw("case") => {
                self.next();
                let scrut = self.parse_expr()?;
                self.expect_kw("of")?;
                let _ = self.eat_sym("|");
                let mut branches = Vec::new();
                loop {
                    let pat = self.parse_pattern()?;
                    self.expect_sym("->")?;
                    let body = self.parse_expr()?;
                    branches.push((pat, body));
                    if !self.eat_sym("|") {
                        break;
                    }
                }
                Ok(Expr::Case(Box::new(scrut), branches))
            }
            Tok::Kw("let") => {
                // local definition: must have `in`
                self.parse_local_def()
            }
            _ => self.parse_seq(),
        }
    }

    fn parse_seq(&mut self) -> PResult<Expr> {
        let l = self.parse_ann()?;
        if self.eat_sym(";") {
            // right side is a full expression so that `;` can be followed
            // by let/if/case/fun forms
            let r = self.parse_expr()?;
            Ok(Expr::Seq(Box::new(l), Box::new(r)))
        } else {
            Ok(l)
        }
    }

    fn parse_ann(&mut self) -> PResult<Expr> {
        let l = self.parse_or()?;
        if self.eat_sym(":") {
            let t = self.parse_type()?;
            Ok(Expr::Ann(Box::new(l), t))
        } else {
            Ok(l)
        }
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut l = self.parse_and()?;
        while matches!(self.peek_tok(), Tok::Sym("||")) {
            self.next();
            let r = self.parse_and()?;
            l = Expr::Bin(BinOp::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut l = self.parse_cmp()?;
        while matches!(self.peek_tok(), Tok::Sym("&&")) {
            self.next();
            let r = self.parse_cmp()?;
            l = Expr::Bin(BinOp::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn parse_cmp(&mut self) -> PResult<Expr> {
        let l = self.parse_cons()?;
        let op = match self.peek_tok() {
            Tok::Sym("==") => Some(BinOp::Eq),
            Tok::Sym("!=") => Some(BinOp::Neq),
            Tok::Sym("<") => Some(BinOp::Lt),
            Tok::Sym(">") => Some(BinOp::Gt),
            Tok::Sym("<=") => Some(BinOp::Le),
            Tok::Sym(">=") => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let r = self.parse_cons()?;
            Ok(Expr::Bin(op, Box::new(l), Box::new(r)))
        } else {
            Ok(l)
        }
    }

    /// `::` and `++`: right associative
    fn parse_cons(&mut self) -> PResult<Expr> {
        let l = self.parse_add()?;
        let op = match self.peek_tok() {
            Tok::Sym("::") => Some(BinOp::Cons),
            Tok::Sym("++") => Some(BinOp::Concat),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            let r = self.parse_cons()?;
            Ok(Expr::Bin(op, Box::new(l), Box::new(r)))
        } else {
            Ok(l)
        }
    }

    fn parse_add(&mut self) -> PResult<Expr> {
        let mut l = self.parse_mul()?;
        loop {
            let op = match self.peek_tok() {
                Tok::Sym("+") => BinOp::Add,
                Tok::Sym("-") => BinOp::Sub,
                _ => break,
            };
            self.next();
            let r = self.parse_mul()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn parse_mul(&mut self) -> PResult<Expr> {
        let mut l = self.parse_app()?;
        loop {
            let op = match self.peek_tok() {
                Tok::Sym("*") => BinOp::Mul,
                Tok::Sym("/") => BinOp::Div,
                Tok::Sym("%") => BinOp::Mod,
                _ => break,
            };
            self.next();
            let r = self.parse_app()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    /// function application: juxtaposition of atoms
    fn parse_app(&mut self) -> PResult<Expr> {
        let mut e = self.parse_atom()?;
        while self.starts_atom() {
            let arg = self.parse_atom()?;
            e = Expr::App(Box::new(e), Box::new(arg));
        }
        Ok(e)
    }

    fn starts_atom(&self) -> bool {
        matches!(
            self.peek_tok(),
            Tok::Int(_) | Tok::Str(_) | Tok::Ident(_) | Tok::Upper(_) |
            Tok::Kw("true") | Tok::Kw("false") | Tok::Sym("(") | Tok::Sym("[")
        )
    }

    fn parse_atom(&mut self) -> PResult<Expr> {
        match self.peek_tok().clone() {
            Tok::Int(n) => {
                self.next();
                Ok(Expr::Lit(Lit::Int(n)))
            }
            Tok::Str(s) => {
                self.next();
                Ok(Expr::Lit(Lit::Str(s)))
            }
            Tok::Kw("true") => {
                self.next();
                Ok(Expr::Lit(Lit::Bool(true)))
            }
            Tok::Kw("false") => {
                self.next();
                Ok(Expr::Lit(Lit::Bool(false)))
            }
            Tok::Sym("-") => {
                // negative integer literal (atom position only)
                if matches!(self.peek2_tok(), Tok::Int(_)) {
                    self.next();
                    if let Tok::Int(n) = self.next().tok {
                        return Ok(Expr::Lit(Lit::Int(-n)));
                    }
                    unreachable!()
                }
                self.err("expected expression")
            }
            Tok::Ident(name) => {
                self.next();
                Ok(Expr::Var(name))
            }
            Tok::Upper(name) => {
                self.next();
                Ok(Expr::Ctor(name))
            }
            Tok::Sym("(") => {
                self.next();
                if self.eat_sym(")") {
                    return Ok(Expr::Lit(Lit::Unit));
                }
                let e = self.parse_seq_in_parens()?;
                if self.eat_sym(",") {
                    let mut elems = vec![e];
                    loop {
                        elems.push(self.parse_seq_in_parens()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    Ok(Expr::Tuple(elems))
                } else {
                    self.expect_sym(")")?;
                    Ok(e)
                }
            }
            Tok::Sym("[") => {
                self.next();
                if self.eat_sym("]") {
                    return Ok(nil());
                }
                let mut elems = vec![self.parse_seq_in_parens()?];
                while self.eat_sym(",") {
                    elems.push(self.parse_seq_in_parens()?);
                }
                self.expect_sym("]")?;
                Ok(elems.into_iter().rev().fold(nil(), |acc, e| cons(e, acc)))
            }
            _ => self.err("expected expression"),
        }
    }

    /// expression inside parens/list: allows all expression forms
    fn parse_seq_in_parens(&mut self) -> PResult<Expr> {
        self.parse_expr()
    }

    // ---- patterns ----

    fn parse_pattern(&mut self) -> PResult<Pattern> {
        let l = self.parse_pat_app()?;
        if self.eat_sym("::") {
            let r = self.parse_pattern()?;
            Ok(Pattern::PCtor("Cons".into(), vec![l, r]))
        } else {
            Ok(l)
        }
    }

    fn parse_pat_app(&mut self) -> PResult<Pattern> {
        if let Tok::Upper(name) = self.peek_tok().clone() {
            self.next();
            let mut args = Vec::new();
            while self.starts_pat_atom() {
                args.push(self.parse_pat_atom()?);
            }
            return Ok(Pattern::PCtor(name, args));
        }
        self.parse_pat_atom()
    }

    fn starts_pat_atom(&self) -> bool {
        matches!(
            self.peek_tok(),
            Tok::Int(_) | Tok::Str(_) | Tok::Ident(_) | Tok::Upper(_) |
            Tok::Kw("true") | Tok::Kw("false") | Tok::Sym("(") | Tok::Sym("[")
        )
    }

    fn parse_pat_atom(&mut self) -> PResult<Pattern> {
        match self.peek_tok().clone() {
            Tok::Sym("_") => {
                self.next();
                Ok(Pattern::Wildcard)
            }
            Tok::Ident(name) => {
                self.next();
                Ok(Pattern::Var(name))
            }
            Tok::Int(n) => {
                self.next();
                Ok(Pattern::PLit(Lit::Int(n)))
            }
            Tok::Str(s) => {
                self.next();
                Ok(Pattern::PLit(Lit::Str(s)))
            }
            Tok::Kw("true") => {
                self.next();
                Ok(Pattern::PLit(Lit::Bool(true)))
            }
            Tok::Kw("false") => {
                self.next();
                Ok(Pattern::PLit(Lit::Bool(false)))
            }
            Tok::Sym("(") => {
                self.next();
                if self.eat_sym(")") {
                    return Ok(Pattern::PLit(Lit::Unit));
                }
                let p = self.parse_pattern()?;
                if self.eat_sym(",") {
                    let mut elems = vec![p];
                    loop {
                        elems.push(self.parse_pattern()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    Ok(Pattern::PTuple(elems))
                } else {
                    self.expect_sym(")")?;
                    Ok(p)
                }
            }
            Tok::Sym("[") => {
                self.next();
                if self.eat_sym("]") {
                    return Ok(Pattern::PCtor("Nil".into(), vec![]));
                }
                let mut elems = vec![self.parse_pattern()?];
                while self.eat_sym(",") {
                    elems.push(self.parse_pattern()?);
                }
                self.expect_sym("]")?;
                Ok(elems.into_iter().rev().fold(pat_nil(), |acc, p| {
                    Pattern::PCtor("Cons".into(), vec![p, acc])
                }))
            }
            _ => self.err("expected pattern"),
        }
    }

    // ---- types ----

    fn parse_type(&mut self) -> PResult<TyAst> {
        let l = self.parse_app_ty()?;
        if self.eat_sym("->") {
            let r = self.parse_type()?;
            Ok(TyAst::Arrow(Box::new(l), Box::new(r)))
        } else {
            Ok(l)
        }
    }

    fn parse_app_ty(&mut self) -> PResult<TyAst> {
        let head = self.parse_atomty()?;
        let mut args = Vec::new();
        while self.starts_type_atom() {
            args.push(self.parse_atomty()?);
        }
        if args.is_empty() {
            Ok(head)
        } else {
            match head {
                TyAst::TCon(name, existing) if existing.is_empty() => {
                    Ok(TyAst::TCon(name, args))
                }
                other => Err(format!("invalid type application on `{}`", format_tyast(&other))),
            }
        }
    }

    fn parse_atomty(&mut self) -> PResult<TyAst> {
        match self.peek_tok().clone() {
            Tok::Ident(v) => {
                self.next();
                Ok(TyAst::TVar(v))
            }
            Tok::Upper(c) => {
                self.next();
                Ok(TyAst::TCon(c, vec![]))
            }
            Tok::Sym("(") => {
                self.next();
                if self.eat_sym(")") {
                    return Ok(TyAst::TCon("Unit".into(), vec![]));
                }
                let t = self.parse_type()?;
                if self.eat_sym(",") {
                    let mut elems = vec![t];
                    loop {
                        elems.push(self.parse_type()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                    self.expect_sym(")")?;
                    Ok(TyAst::TTuple(elems))
                } else {
                    self.expect_sym(")")?;
                    Ok(t)
                }
            }
            Tok::Sym("[") => {
                self.next();
                let t = self.parse_type()?;
                self.expect_sym("]")?;
                Ok(TyAst::list(t))
            }
            _ => self.err("expected type"),
        }
    }

    // ---- helpers ----

    fn parse_ident(&mut self) -> PResult<String> {
        match self.peek_tok().clone() {
            Tok::Ident(s) => {
                self.next();
                Ok(s)
            }
            _ => self.err("expected identifier"),
        }
    }

    fn parse_upper(&mut self) -> PResult<String> {
        match self.peek_tok().clone() {
            Tok::Upper(s) => {
                self.next();
                Ok(s)
            }
            _ => self.err("expected constructor/type name"),
        }
    }
}

fn wrap_params(params: Vec<Pattern>, mut body: Expr) -> Expr {
    for p in params.into_iter().rev() {
        body = Expr::Lam(p, Box::new(body));
    }
    body
}

fn nil() -> Expr {
    Expr::Ctor("Nil".into())
}

fn cons(h: Expr, t: Expr) -> Expr {
    Expr::App(Box::new(Expr::App(Box::new(Expr::Ctor("Cons".into())), Box::new(h))), Box::new(t))
}

fn pat_nil() -> Pattern {
    Pattern::PCtor("Nil".into(), vec![])
}

pub fn format_tyast(t: &TyAst) -> String {
    match t {
        TyAst::TVar(v) => v.clone(),
        TyAst::TCon(name, args) if args.is_empty() => name.clone(),
        TyAst::TCon(name, args) => {
            format!("{} {}", name, args.iter().map(format_tyast).collect::<Vec<_>>().join(" "))
        }
        TyAst::Arrow(a, b) => format!("{} -> {}", format_tyast(a), format_tyast(b)),
        TyAst::TTuple(ts) => format!("({})", ts.iter().map(format_tyast).collect::<Vec<_>>().join(", ")),
    }
}
