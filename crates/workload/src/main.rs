//! Write a workload to a file: `arcade-workload <name> <megabytes> <path>`, or
//! `arcade-workload scrollback-fill <lines> <path>` for full 120-column lines.

use std::process::ExitCode;

use arcade_workload::{Workload, generate, scrollback_fill};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let data = match args.as_slice() {
        ["scrollback-fill", lines, _] => match lines.parse() {
            Ok(lines) => scrollback_fill(lines, 120, 1),
            Err(_) => return usage(),
        },
        [name, megabytes, _] => match (Workload::from_name(name), megabytes.parse::<usize>()) {
            (Some(workload), Ok(megabytes)) => generate(workload, megabytes * 1_000_000, 1),
            _ => return usage(),
        },
        _ => return usage(),
    };
    let path = args[2];
    if let Err(error) = std::fs::write(path, data) {
        eprintln!("cannot write {path}: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    let names: Vec<&str> = Workload::ALL.iter().map(|w| w.name()).collect();
    eprintln!(
        "usage: arcade-workload <{}> <megabytes> <path>",
        names.join("|")
    );
    eprintln!("       arcade-workload scrollback-fill <lines> <path>");
    ExitCode::from(2)
}
