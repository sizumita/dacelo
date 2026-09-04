// dcc -- dacelo Gen 2 compiler driver
//
// usage: dcc <input.dc> -o <output> [--run]

use std::path::Path;
use std::process::Command;

mod codegen;
mod encoder;
mod macho;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut run = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                output = args.get(i).cloned();
            }
            "--run" => run = true,
            other if input.is_none() => input = Some(other.to_string()),
            other => {
                eprintln!("dcc: unexpected argument `{}`", other);
                return std::process::ExitCode::from(2);
            }
        }
        i += 1;
    }
    let (Some(input), Some(output)) = (input, output) else {
        eprintln!("usage: dcc <file.dc> -o <output> [--run]");
        return std::process::ExitCode::from(2);
    };

    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dcc: cannot read {}: {}", input, e);
            return std::process::ExitCode::from(2);
        }
    };

    // front-end: reuse gen 0's lexer/parser/type checker
    let toks = match dacelo::lexer::lex(&src) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("dcc: {}", e);
            return std::process::ExitCode::FAILURE;
        }
    };
    let prog = match dacelo::parser::parse(toks) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dcc: {}", e);
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut inf = dacelo::infer::Infer::new();
    if let Err(e) = inf.check_program(&prog) {
        eprintln!("dcc: type error: {}", e);
        return std::process::ExitCode::FAILURE;
    }

    // constructor table in id order (Nil/Cons first, then declaration order)
    let mut ctor_entries: Vec<(u32, String, usize)> = inf
        .ctors
        .iter()
        .map(|(name, (tag, arity))| (*tag, name.clone(), *arity))
        .collect();
    ctor_entries.sort_by_key(|(tag, _, _)| *tag);
    let ctor_names: Vec<String> = ctor_entries.iter().map(|(_, n, _)| n.clone()).collect();
    let mut arities = std::collections::HashMap::new();
    for (_, n, a) in &ctor_entries {
        arities.insert(n.clone(), *a);
    }

    // codegen + object emission
    let cg = codegen::Codegen::new(&ctor_names, &arities);
    let obj = cg.compile_program(&prog);
    let obj_bytes = match obj.finish() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("dcc: {}", e);
            return std::process::ExitCode::FAILURE;
        }
    };

    let out_obj = format!("{}.o", output);
    if let Err(e) = std::fs::write(&out_obj, &obj_bytes) {
        eprintln!("dcc: cannot write {}: {}", out_obj, e);
        return std::process::ExitCode::FAILURE;
    }

    // compile the runtime and link
    let rt_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("rt/rt.c");
    let rt_obj = format!("{}.rt.o", output);
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let st = Command::new(&cc)
        .args(["-O2", "-arch", "arm64"])
        .arg("-c")
        .arg(&rt_src)
        .arg("-o")
        .arg(&rt_obj)
        .status();
    match st {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("dcc: failed to compile runtime");
            return std::process::ExitCode::FAILURE;
        }
    }
    let st = Command::new(&cc).arg(&out_obj).arg(&rt_obj).arg("-o").arg(&output).status();
    match st {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("dcc: link failed");
            return std::process::ExitCode::FAILURE;
        }
    }
    let _ = std::fs::remove_file(&rt_obj);
    if std::env::var("DACELO_KEEP_O").is_err() {
        let _ = std::fs::remove_file(&out_obj);
    }

    println!("dcc: wrote {}", output);

    if run {
        let st = Command::new(&output).status();
        match st {
            Ok(s) => return exit_code_of(s.code()),
            Err(e) => {
                eprintln!("dcc: cannot run {}: {}", output, e);
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    std::process::ExitCode::SUCCESS
}

fn exit_code_of(code: Option<i32>) -> std::process::ExitCode {
    use std::os::unix::process::ExitStatusExt;
    match code {
        Some(c) => std::process::ExitCode::from(c as u8),
        None => std::process::ExitCode::from(128 + 9),
    }
}
