// dacelo Gen 0 lexer

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Str(String),
    Ident(String),
    Upper(String),
    Kw(&'static str),
    Sym(&'static str),
    Eof,
}

pub const KEYWORDS: &[&str] = &[
    "let", "rec", "and", "in", "if", "then", "else", "case", "of", "fun", "true", "false", "type",
];

const SYMS2: &[&str] = &[
    "->", "==", "!=", "<=", ">=", "&&", "||", "++", "::",
];

const SYMS1: &str = "()[] ,;:|_<>=+-*/%";

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for Tok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tok::Int(n) => write!(f, "int {}", n),
            Tok::Str(s) => write!(f, "string {:?}", s),
            Tok::Ident(s) => write!(f, "ident `{}`", s),
            Tok::Upper(s) => write!(f, "ctor `{}`", s),
            Tok::Kw(s) => write!(f, "keyword `{}`", s),
            Tok::Sym(s) => write!(f, "`{}`", s),
            Tok::Eof => write!(f, "end of input"),
        }
    }
}

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let mut toks = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    macro_rules! bump {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        // whitespace
        if c.is_whitespace() {
            bump!();
            continue;
        }
        // comment
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            while i < chars.len() && chars[i] != '\n' {
                bump!();
            }
            continue;
        }
        let (l0, c0) = (line, col);
        // int literal
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                bump!();
            }
            let s: String = chars[start..i].iter().collect();
            let n: i64 = s
                .parse()
                .map_err(|_| format!("{}:{}: integer literal overflow `{}`", l0, c0, s))?;
            toks.push(Token { tok: Tok::Int(n), line: l0, col: c0 });
            continue;
        }
        // string literal
        if c == '"' {
            bump!(); // consume "
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err(format!("{}:{}: unterminated string literal", l0, c0));
                }
                let ch = chars[i];
                if ch == '"' {
                    bump!();
                    break;
                }
                if ch == '\\' {
                    bump!();
                    if i >= chars.len() {
                        return Err(format!("{}:{}: unterminated string literal", l0, c0));
                    }
                    match chars[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '0' => s.push('\0'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        other => {
                            return Err(format!(
                                "{}:{}: unknown escape `\\{}` in string literal",
                                line, col, other
                            ))
                        }
                    }
                    bump!();
                    continue;
                }
                s.push(ch);
                bump!();
            }
            toks.push(Token { tok: Tok::Str(s), line: l0, col: c0 });
            continue;
        }
        // identifiers / keywords / constructors
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '\'')
            {
                bump!();
            }
            let s: String = chars[start..i].iter().collect();
            let tok = if let Some(kw) = KEYWORDS.iter().find(|k| **k == s) {
                Tok::Kw(kw)
            } else if s.chars().next().unwrap().is_ascii_uppercase() {
                Tok::Upper(s)
            } else {
                Tok::Ident(s)
            };
            toks.push(Token { tok, line: l0, col: c0 });
            continue;
        }
        // symbols (two-char first)
        let rest: String = chars[i..std::cmp::min(i + 2, chars.len())].iter().collect();
        if let Some(s2) = SYMS2.iter().find(|s| rest.starts_with(**s)) {
            for _ in 0..s2.len() {
                bump!();
            }
            toks.push(Token { tok: Tok::Sym(s2), line: l0, col: c0 });
            continue;
        }
        if SYMS1.contains(c) {
            let sym: &'static str = match c {
                '(' => "(",
                ')' => ")",
                '[' => "[",
                ']' => "]",
                ',' => ",",
                ';' => ";",
                ':' => ":",
                '|' => "|",
                '_' => "_",
                '<' => "<",
                '>' => ">",
                '=' => "=",
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                _ => unreachable!(),
            };
            bump!();
            toks.push(Token { tok: Tok::Sym(sym), line: l0, col: c0 });
            continue;
        }
        return Err(format!("{}:{}: unexpected character `{}`", l0, c0, c));
    }

    let last_line = line;
    let last_col = col;
    toks.push(Token { tok: Tok::Eof, line: last_line, col: last_col });
    Ok(toks)
}
