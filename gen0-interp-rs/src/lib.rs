// dacelo Gen 0: pipeline glue (lex -> parse -> infer -> eval)

pub mod ast;
pub mod eval;
pub mod infer;
pub mod lexer;
pub mod parser;

use std::cell::RefCell;
use std::rc::Rc;

/// Run a dacelo source string. Output of print builtins goes to `out`.
pub fn run_source(src: &str, out: eval::OutSink) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    let mut inf = infer::Infer::new();
    inf.check_program(&prog)?;
    let ctors = inf.ctors.clone();
    let mut machine = eval::Machine::new(out, ctors);
    machine.run_items(&prog).map(|_| ())
}

/// Convenience for tests / embedding: run and capture printed output.
pub fn run_source_captured(src: &str) -> Result<String, String> {
    let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
    run_source(src, eval::OutSink::Buffer(buf.clone()))
        .map(|_| String::from_utf8(buf.borrow().clone()).expect("output is utf8"))
}
