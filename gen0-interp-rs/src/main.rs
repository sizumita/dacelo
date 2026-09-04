// dacelo Gen 0 CLI

use std::process::ExitCode;

fn print_types(src: &str) -> Result<(), String> {
    let toks = dacelo::lexer::lex(src)?;
    let prog = dacelo::parser::parse(toks)?;
    let mut inf = dacelo::infer::Infer::new();
    for (i, item) in prog.items.iter().enumerate() {
        let kind = match item {
            dacelo::ast::Item::Ty(t) => format!("type {}", t.name),
            dacelo::ast::Item::Def(d) => format!("let {}", d.name),
            dacelo::ast::Item::RecGroup(ds) => {
                let names: Vec<String> = ds.iter().map(|d| d.name.clone()).collect();
                format!("let rec {}", names.join(" / "))
            }
        };
        let res = inf.process_item(item);
        if let Err(e) = res {
            if std::env::var("DACELO_SCAN").is_ok() {
                inf.scan_leaks();
            }
            if std::env::var("DACELO_DUMP").is_ok() {
                let mut keys: Vec<_> = inf.env.keys().cloned().collect();
                keys.sort();
                for k in keys {
                    if let Some(sch) = inf.env.get(&k) {
                        eprintln!("DUMP {} : {}   [q={}]", k, inf.show(&sch.ty), sch.qvars.len());
                    }
                }
            }
            if std::env::var("DACELO_DEBUG").is_ok() {
                if let dacelo::ast::Item::RecGroup(ds) = item {
                    for d in ds {
                        if let Some(sch) = inf.env.get(&d.name) {
                            eprintln!("[dbg-fail] {} : {}", d.name, inf.show(&sch.ty));
                        }
                    }
                }
            }
            return Err(format!("[item {}: {}] {}", i, kind, e));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: dacelo <file.dc> [args...]");
        return ExitCode::from(2);
    }
    let path = &args[1];
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dacelo: cannot read {}: {}", path, e);
            return ExitCode::from(2);
        }
    };

    if args.len() > 2 && args[2] == "--types" {
        return match print_types(&src) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("dacelo: {}", msg);
                ExitCode::FAILURE
            }
        };
    }
    if args.len() > 2 && args[2] == "--ast" {
        let toks = match dacelo::lexer::lex(&src) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("dacelo: {}", e);
                return ExitCode::FAILURE;
            }
        };
        match dacelo::parser::parse(toks) {
            Ok(p) => println!("{:#?}", p),
            Err(e) => {
                eprintln!("dacelo: {}", e);
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }

    // run on a big-stack thread: tree-walking evaluation of deeply
    // recursive dacelo programs needs more than the default stack
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || dacelo::run_source(&src, dacelo::eval::OutSink::Stdout))
        .expect("failed to spawn worker thread");

    match handle.join() {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(msg)) => {
            eprintln!("dacelo: {}", msg);
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("dacelo: internal error (worker panicked)");
            ExitCode::from(101)
        }
    }
}
