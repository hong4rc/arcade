//! Command line entry of the parser evaluation. `cargo parser-eval all` runs everything.

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use arcade_workload::{Rng, Workload, generate, scrollback_fill};
use parser_eval::candidate::{CANDIDATES, EMULATORS, make};
use parser_eval::cases;
use parser_eval::procstat::memory_bytes;
use serde_json::{Value, json};

/// Counts heap allocations made through Rust's allocator, to check the "no allocation per byte
/// on the hot path" rule. libghostty-vt allocates through its own allocator and is not counted.
struct CountingAllocator;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call is forwarded unchanged to the system allocator; only a counter is added.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

const MB: f64 = 1_000_000.0;
const SCROLLBACK: usize = 50_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["child", "random", candidate, kind, megabytes] => {
            child_random(candidate, kind, megabytes.parse().expect("megabytes"))
        }
        ["child", "memory", candidate, kind] => child_memory(candidate, kind),
        ["correctness"] => {
            correctness();
        }
        ["throughput"] => {
            throughput();
        }
        ["random"] => {
            random();
        }
        ["memory"] => {
            memory();
        }
        ["all"] => {
            let result = json!({
                "correctness": correctness(),
                "throughput": throughput(),
                "random": random(),
                "memory": memory(),
            });
            std::fs::create_dir_all("bench-results").expect("create bench-results");
            let path = "bench-results/parser-eval.json";
            std::fs::write(path, serde_json::to_string_pretty(&result).unwrap())
                .expect("write json");
            println!("\nMachine-readable results written to {path}");
        }
        _ => {
            eprintln!("usage: parser-eval <all|correctness|throughput|random|memory>");
            std::process::exit(2);
        }
    }
}

fn correctness() -> Value {
    println!("\n== Correctness: every case fed whole and split at 1, 7 and 1024 bytes ==\n");
    let all = cases::all();
    let mut by_candidate = serde_json::Map::new();
    let mut failures: Vec<(String, &str, String)> = Vec::new();
    print!("{:<36}", "case");
    for name in EMULATORS {
        print!("{name:>20}");
    }
    println!();
    let mut table: Vec<Vec<bool>> = Vec::new();
    for case in &all {
        print!("{:<36}", format!("{}/{}", case.group.name(), case.id));
        let mut row = Vec::new();
        for name in EMULATORS {
            let outcome = cases::run_case(name, case);
            print!(
                "{:>20}",
                if outcome.failure.is_none() {
                    "pass"
                } else {
                    "FAIL"
                }
            );
            row.push(outcome.failure.is_none());
            let entry = by_candidate.entry(name).or_insert_with(|| json!({}));
            entry[case.id] = match &outcome.failure {
                None => json!("pass"),
                Some(message) => json!(message),
            };
            if let Some(message) = outcome.failure {
                failures.push((name.to_string(), case.id, message));
            }
        }
        table.push(row);
        println!();
    }
    println!();
    let mut summary = serde_json::Map::new();
    for (index, name) in EMULATORS.iter().enumerate() {
        let mut parts = Vec::new();
        let mut groups = serde_json::Map::new();
        for group in [
            cases::Group::Strings,
            cases::Group::Unicode,
            cases::Group::Screen,
            cases::Group::Replies,
        ] {
            let total = all.iter().filter(|c| c.group == group).count();
            let passed = all
                .iter()
                .zip(&table)
                .filter(|(c, row)| c.group == group && row[index])
                .count();
            parts.push(format!("{} {passed}/{total}", group.name()));
            groups.insert(
                group.name().into(),
                json!({"passed": passed, "total": total}),
            );
        }
        println!("{name:<20} {}", parts.join(", "));
        summary.insert((*name).into(), Value::Object(groups));
    }
    println!("\nFailures:");
    for (name, id, message) in &failures {
        println!("  {name} / {id}: {message}");
    }
    json!({"summary": summary, "cases": by_candidate})
}

fn throughput() -> Value {
    println!(
        "\n== Throughput: megabytes per second on one core, best of 3 runs, 120x40, 50,000 lines of scrollback =="
    );
    let mut result = serde_json::Map::new();
    for chunk in [65_536usize, 1024] {
        println!("\nInput delivered in writes of {chunk} bytes:\n");
        print!("{:<14}", "workload");
        for name in CANDIDATES {
            print!("{name:>20}");
        }
        println!();
        let mut chunk_result = serde_json::Map::new();
        for workload in Workload::ALL {
            let data = generate(workload, 32 * 1024 * 1024, 1);
            print!("{:<14}", workload.name());
            let mut row = serde_json::Map::new();
            for name in CANDIDATES {
                let mut best = f64::MAX;
                let mut allocations = 0;
                for _ in 0..3 {
                    let mut candidate = make(
                        name,
                        arcade_workload::COLS,
                        arcade_workload::ROWS,
                        SCROLLBACK,
                    );
                    let before = ALLOCATIONS.load(Ordering::Relaxed);
                    let start = Instant::now();
                    for piece in data.chunks(chunk) {
                        candidate.feed(piece);
                    }
                    let elapsed = start.elapsed().as_secs_f64();
                    allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
                    best = best.min(elapsed);
                }
                let rate = data.len() as f64 / MB / best;
                let per_mb = allocations as f64 / (data.len() as f64 / MB);
                print!("{rate:>20.0}");
                row.insert(
                    (*name).into(),
                    json!({"mb_per_s": rate, "rust_allocations_per_mb": per_mb}),
                );
            }
            println!();
            chunk_result.insert(workload.name().into(), Value::Object(row));
        }
        result.insert(format!("chunk_{chunk}"), Value::Object(chunk_result));
    }
    println!(
        "\nRust heap allocations per megabyte of input (64 KiB writes; libghostty-vt uses its own allocator and is not counted):\n"
    );
    print!("{:<14}", "workload");
    for name in CANDIDATES {
        print!("{name:>20}");
    }
    println!();
    for workload in Workload::ALL {
        print!("{:<14}", workload.name());
        for name in CANDIDATES {
            let value = &result["chunk_65536"][workload.name()][name]["rust_allocations_per_mb"];
            if name == "libghostty-vt" {
                print!("{:>20}", "n/a");
            } else {
                print!("{:>20.0}", value.as_f64().unwrap_or(0.0));
            }
        }
        println!();
    }
    Value::Object(result)
}

fn run_child(args: &[&str]) -> (bool, String, Value) {
    let exe = std::env::current_exe().expect("current exe");
    let output = Command::new(exe)
        .arg("child")
        .args(args)
        .output()
        .expect("spawn child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value = stdout
        .lines()
        .last()
        .and_then(|line| serde_json::from_str(line).ok())
        .unwrap_or(Value::Null);
    let status = if output.status.success() {
        "ok".to_string()
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string();
        format!(
            "{} {}",
            output.status,
            last.chars().take(100).collect::<String>()
        )
    };
    (output.status.success(), status, value)
}

const RANDOM_KINDS: [(&str, usize, &str); 4] = [
    ("noise", 64, "uniformly random bytes"),
    (
        "escape-noise",
        64,
        "random mix of escape sequence fragments and random bytes",
    ),
    (
        "mutated",
        64,
        "the six workloads with 1% of bytes replaced at random",
    ),
    ("endless-osc", 256, "one OSC string that never ends"),
];

fn random() -> Value {
    println!(
        "\n== Bad data: each run in its own process, input cut into writes of random size ==\n"
    );
    for (kind, megabytes, what) in RANDOM_KINDS {
        println!("  {kind}: {megabytes} MB of {what}");
    }
    println!();
    println!(
        "{:<20}{:<14}{:>8}{:>10}{:>18}",
        "candidate", "input", "result", "seconds", "memory growth MB"
    );
    let mut result = serde_json::Map::new();
    for name in CANDIDATES {
        let mut per = serde_json::Map::new();
        for (kind, megabytes, _) in RANDOM_KINDS {
            let (ok, status, value) = run_child(&["random", name, kind, &megabytes.to_string()]);
            let seconds = value["seconds"].as_f64().unwrap_or(f64::NAN);
            let growth = value["memory_growth_mb"].as_f64().unwrap_or(f64::NAN);
            println!(
                "{name:<20}{kind:<14}{:>8}{seconds:>10.2}{growth:>18.1}{}",
                if ok { "ok" } else { "CRASH" },
                if ok {
                    String::new()
                } else {
                    format!("  {status}")
                }
            );
            per.insert(
                kind.into(),
                json!({"ok": ok, "status": status, "seconds": seconds, "memory_growth_mb": growth}),
            );
        }
        result.insert((*name).into(), Value::Object(per));
    }
    Value::Object(result)
}

fn escape_noise(rng: &mut Rng, len: usize) -> Vec<u8> {
    const FRAGMENTS: &[&[u8]] = &[
        b"\x1b",
        b"\x1b[",
        b"\x1b]",
        b"\x1bP",
        b"\x1b_",
        b"\x1b^",
        b"\x1bX",
        b"\x1b\\",
        b"\x07",
        b"\x18",
        b"\x1a",
        b"\x9b",
        b"\x9d",
        b"\x9c",
        b"\x90",
        b";",
        b":",
        b"?",
        b">",
        b"<",
        b"=",
        b"$",
        b" ",
        b"!",
        b"h",
        b"l",
        b"m",
        b"H",
        b"J",
        b"K",
        b"r",
        b"u",
        b"q",
        b"c",
        b"n",
        b"p",
        b"L",
        b"M",
        b"S",
        b"T",
        b"@",
        b"P",
        b"X",
        b"52;c;",
        b"8;;",
        b"0;",
        b"9;",
        b"777;notify;",
        b"1049",
        b"2026",
        b"2004",
        b"38;2;",
        b"48:2::",
        b"65535",
        b"99999999999999999999",
        b"0",
        b"1",
        b"\r",
        b"\n",
        b"\t",
        b"\x08",
        b"\x0e",
        b"\x0f",
        b"\xe6\xb1\x89",
        b"e\xcc\x82",
        b"\xf0\x9f\x91\xa8\xe2\x80\x8d",
        b"\xff",
        b"\xc3",
        b"\xf0\x9f",
        b"text ",
    ];
    let mut out = Vec::with_capacity(len + 32);
    while out.len() < len {
        if rng.below(10) == 0 {
            let mut noise = [0u8; 8];
            rng.fill(&mut noise);
            out.extend_from_slice(&noise[..rng.range(1, 8)]);
        } else {
            out.extend_from_slice(FRAGMENTS[rng.below(FRAGMENTS.len())]);
        }
    }
    out
}

fn child_random(name: &str, kind: &str, megabytes: usize) {
    let total = megabytes * 1024 * 1024;
    let mut rng = Rng::new(0xBAD_DA7A);
    // Build the input in 1 MiB pieces so the harness itself stays small.
    let piece_len = 1024 * 1024;
    let mut candidate = make(
        name,
        arcade_workload::COLS,
        arcade_workload::ROWS,
        SCROLLBACK,
    );
    let before = memory_bytes();
    let start = Instant::now();
    let mut fed = 0usize;
    let mut round = 0u64;
    if kind == "endless-osc" {
        candidate.feed(b"\x1b]0;");
    }
    while fed < total {
        let piece: Vec<u8> = match kind {
            "noise" => {
                let mut buf = vec![0u8; piece_len];
                rng.fill(&mut buf);
                buf
            }
            "escape-noise" => escape_noise(&mut rng, piece_len),
            "mutated" => {
                let workload = Workload::ALL[round as usize % Workload::ALL.len()];
                let mut buf = generate(workload, piece_len, round);
                for _ in 0..buf.len() / 100 {
                    let at = rng.below(buf.len());
                    buf[at] = rng.next_u64() as u8;
                }
                buf
            }
            "endless-osc" => vec![b'x'; piece_len],
            other => panic!("unknown kind {other}"),
        };
        let mut offset = 0;
        while offset < piece.len() {
            let size = rng.range(1, 4096).min(piece.len() - offset);
            candidate.feed(&piece[offset..offset + size]);
            offset += size;
        }
        candidate.drain_events();
        fed += piece.len();
        round += 1;
    }
    let seconds = start.elapsed().as_secs_f64();
    // Read something back, so a corrupted screen has a chance to show up as a crash.
    let _ = candidate.row_text(0);
    let _ = candidate.cursor();
    let growth = memory_bytes().saturating_sub(before) as f64 / (1024.0 * 1024.0);
    println!(
        "{}",
        json!({"seconds": seconds, "memory_growth_mb": growth})
    );
}

const MEMORY_KINDS: [(&str, &str); 4] = [
    ("plain-50k", "50,000 full lines of 120 plain characters"),
    ("colored-50k", "50,000 lines where every word changes color"),
    ("short-50k", "50,000 lines of at most 5 characters"),
    (
        "empty-x20",
        "20 panes showing only a prompt (limit in the spec: 30 for all of them)",
    ),
];

fn first_lines(mut data: Vec<u8>, lines: usize) -> Vec<u8> {
    let mut seen = 0;
    for index in 0..data.len() {
        if data[index] == b'\n' {
            seen += 1;
            if seen == lines {
                data.truncate(index + 1);
                return data;
            }
        }
    }
    panic!("workload has only {seen} lines, need {lines}");
}

fn child_memory(name: &str, kind: &str) {
    if kind == "empty-x20" {
        // Twenty panes that only show a prompt: the "20 empty panes" line of the spec.
        let before = memory_bytes();
        let mut panes: Vec<_> = (0..20).map(|_| make(name, 120, 40, SCROLLBACK)).collect();
        for pane in &mut panes {
            pane.feed(b"$ ");
        }
        let megabytes = memory_bytes().saturating_sub(before) as f64 / (1024.0 * 1024.0);
        println!(
            "{}",
            json!({"memory_mb": megabytes, "lines_kept": 50_000, "compressed_mb": null})
        );
        return;
    }
    let lines = 50_000;
    let data = match kind {
        "plain-50k" => scrollback_fill(lines, 120, 1),
        "colored-50k" => first_lines(generate(Workload::Colored, 16 * 1024 * 1024, 1), lines),
        "short-50k" => first_lines(generate(Workload::Scroll, 1024 * 1024, 1), lines),
        other => panic!("unknown kind {other}"),
    };
    let before = memory_bytes();
    let mut candidate = make(name, 120, 40, SCROLLBACK);
    for piece in data.chunks(65_536) {
        candidate.feed(piece);
    }
    let megabytes = |now: u64| now.saturating_sub(before) as f64 / (1024.0 * 1024.0);
    let after = megabytes(memory_bytes());
    let lines_kept = candidate.scrollback_lines();
    let compressed = if candidate.compress_scrollback() {
        Some(megabytes(memory_bytes()))
    } else {
        None
    };
    let _ = candidate.row_text(0);
    println!(
        "{}",
        json!({"memory_mb": after, "lines_kept": lines_kept, "compressed_mb": compressed})
    );
}

fn memory() -> Value {
    println!(
        "\n== Memory of one 120x40 pane after 50,000 lines, megabytes (limit in the spec: 30) =="
    );
    println!(
        "A second number in brackets is the size after the library compressed its idle scrollback.\n"
    );
    for (kind, what) in MEMORY_KINDS {
        println!("  {kind}: {what}");
    }
    println!();
    print!("{:<14}", "content");
    for name in EMULATORS {
        print!("{name:>20}");
    }
    println!();
    let mut result = serde_json::Map::new();
    for (kind, _) in MEMORY_KINDS {
        print!("{kind:<14}");
        let mut row = serde_json::Map::new();
        for name in EMULATORS {
            let (_, _, value) = run_child(&["memory", name, kind]);
            let megabytes = value["memory_mb"].as_f64().unwrap_or(f64::NAN);
            let cell = match value["compressed_mb"].as_f64() {
                Some(compressed) => format!("{megabytes:.1} ({compressed:.1})"),
                None => format!("{megabytes:.1}"),
            };
            print!("{cell:>20}");
            assert!(
                value["lines_kept"].as_u64().unwrap_or(0) >= 49_000,
                "{name} kept too few lines: {value}"
            );
            row.insert(name.into(), value);
        }
        println!();
        result.insert(kind.into(), Value::Object(row));
    }
    Value::Object(result)
}
