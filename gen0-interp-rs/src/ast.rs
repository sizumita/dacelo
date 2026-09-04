// dacelo Gen 0 AST

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64),
    Bool(bool),
    Str(String),
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Var(String),
    PLit(Lit),
    PTuple(Vec<Pattern>),
    /// constructor application: Ctor p1 p2 ...
    PCtor(String, Vec<Pattern>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Concat, // ++
    Cons,   // ::
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

/// type expression as written in source
#[derive(Debug, Clone, PartialEq)]
pub enum TyAst {
    /// lowercase identifier: type variable
    TVar(String),
    /// named type constructor with args: Int, Tree, List a
    TCon(String, Vec<TyAst>),
    Arrow(Box<TyAst>, Box<TyAst>),
    TTuple(Vec<TyAst>),
}

impl TyAst {
    pub fn list(t: TyAst) -> TyAst {
        TyAst::TCon("List".into(), vec![t])
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Lit(Lit),
    Var(String),
    Ctor(String),
    Lam(Pattern, Box<Expr>),
    App(Box<Expr>, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Case(Box<Expr>, Vec<(Pattern, Expr)>),
    Let {
        is_rec: bool,
        name: String,
        rhs: Box<Expr>,
        body: Box<Expr>,
    },
    /// sequencing: evaluate left (must be Unit), discard, evaluate right
    Seq(Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    Ann(Box<Expr>, TyAst),
}

#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub params: Vec<Pattern>,
    pub ann: Option<TyAst>,
    pub body: Expr,
    pub is_rec: bool,
}

#[derive(Debug, Clone)]
pub struct CtorDecl {
    pub name: String,
    pub fields: Vec<TyAst>,
}

#[derive(Debug, Clone)]
pub struct TyDecl {
    pub name: String,
    pub params: Vec<String>,
    pub ctors: Vec<CtorDecl>,
}

#[derive(Debug, Clone)]
pub enum Item {
    /// single definition (rec or not)
    Def(Def),
    /// mutually recursive group: `let rec f ... = ... and g ... = ...`
    RecGroup(Vec<Def>),
    Ty(TyDecl),
}

#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}
