//! What the client's screen shows after the daemon has drawn it: it must match, cell for cell,
//! a terminal that was given the pane's output directly. Also how often the daemon sends updates,
//! how little it sends for a small change, and what a client that does not read costs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use arcade_term::{Frame, Mode, Term, TermConfig};
use arcade_testkit::{AttachOptions, PtyHost, Screened, TestClient, TestDaemon, WAIT};
use arcade_workload::{Workload, generate};

/// Two cells look the same: a cell with a background color and no text and one holding a space
/// are drawn alike, so they count as equal.
fn same_cell(a: &arcade_term::Cell, b: &arcade_term::Cell) -> bool {
    let text = |c: &arcade_term::Cell| {
        if c.text.is_empty() {
            " ".to_string()
        } else {
            c.text.clone()
        }
    };
    text(a) == text(b) && a.width == b.width && a.style == b.style
}

/// The first difference between two screens, or `None` when they are the same.
fn first_difference(got: &Frame, want: &Frame) -> Option<String> {
    if (got.cols, got.rows_total) != (want.cols, want.rows_total) {
        return Some(format!(
            "size {}x{} instead of {}x{}",
            got.cols, got.rows_total, want.cols, want.rows_total
        ));
    }
    for (g, w) in got.rows.iter().zip(&want.rows) {
        if g.cells.len() != w.cells.len()
            || !g.cells.iter().zip(&w.cells).all(|(a, b)| same_cell(a, b))
        {
            for (col, (gc, wc)) in g.cells.iter().zip(&w.cells).enumerate() {
                if !same_cell(gc, wc) {
                    return Some(format!(
                        "row {} column {col}: got {gc:?}, wanted {wc:?}",
                        g.y
                    ));
                }
            }
            return Some(format!("row {} differs in length", g.y));
        }
    }
    if got.rows.len() != want.rows.len() {
        return Some("different number of rows".to_string());
    }
    if got.cursor != want.cursor {
        return Some(format!("cursor {:?}, wanted {:?}", got.cursor, want.cursor));
    }
    None
}

/// What the panes show on the client's screen: all of it but the status bar, the last row.
fn client_frame(client: &mut TestClient) -> Frame {
    let mut frame = client.screen.snapshot(true).expect("snapshot");
    frame.rows.pop();
    frame.rows_total -= 1;
    frame
}

fn direct_term(cols: u16, rows: u16, data: &[u8]) -> Term {
    let mut term = Term::new(TermConfig::new(cols, rows)).unwrap();
    for chunk in data.chunks(64 * 1024) {
        term.feed(chunk);
        let _ = term.drain_events();
    }
    term
}

/// Wait until the client's screen is the same as `want`, and fail with the first difference.
fn expect_same_screen(client: &mut TestClient, want: &Frame, timeout: Duration) {
    let mut last = None;
    let ok = client.wait_until(timeout, |c| {
        last = first_difference(&client_frame(c), want);
        last.is_none()
    });
    assert!(
        ok,
        "the client's screen differs from the pane's: {}\nscreen:\n{}",
        last.unwrap_or_default(),
        client.screen_text()
    );
}

/// A shell command that prints a file untouched, then marks the top left corner, then waits.
fn cat_then_mark(file: &Path) -> Vec<String> {
    vec![
        "/bin/sh".into(),
        "-c".into(),
        format!(
            "stty raw -echo; cat '{}'; printf '\\033[HDONE-MARK'; sleep 120",
            file.display()
        ),
    ]
}

#[test]
fn every_kind_of_load_draws_the_same_screen_as_feeding_it_directly() {
    for workload in Workload::ALL {
        let data = generate(workload, 3 * 1024 * 1024, 11);
        let daemon = TestDaemon::start();
        let file = daemon.dir().join("load.bin");
        std::fs::write(&file, &data).unwrap();

        let mut options =
            AttachOptions::new().size(arcade_workload::COLS, arcade_workload::ROWS + 1);
        options.command = cat_then_mark(&file);
        let mut client = daemon.attach(options).unwrap();

        let mut expected_bytes = data.clone();
        expected_bytes.extend_from_slice(b"\x1b[HDONE-MARK");
        let mut direct = direct_term(
            arcade_workload::COLS,
            arcade_workload::ROWS,
            &expected_bytes,
        );
        let want = direct.snapshot(true).unwrap();

        let name = workload.name();
        client
            .wait_for_text_within("DONE-MARK", Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut last = None;
        let ok = client.wait_until(Duration::from_secs(20), |c| {
            last = first_difference(&client_frame(c), &want);
            last.is_none()
        });
        assert!(
            ok,
            "{name}: the client's screen differs from a terminal fed directly: {}\n{}",
            last.unwrap_or_default(),
            client.screen_text()
        );
    }
}

#[test]
fn the_screen_is_redrawn_whole_when_the_client_resizes() {
    let daemon = TestDaemon::start();
    let data = generate(Workload::Colored, 200 * 1024, 5);
    let file = daemon.dir().join("load.bin");
    std::fs::write(&file, &data).unwrap();
    // One row more than the pane: the status bar.
    let mut options = AttachOptions::new().size(100, 31);
    options.command = cat_then_mark(&file);
    let mut client = daemon.attach(options).unwrap();
    client
        .wait_for_text_within("DONE-MARK", Duration::from_secs(30))
        .unwrap();

    let mut expected_bytes = data.clone();
    expected_bytes.extend_from_slice(b"\x1b[HDONE-MARK");
    let mut direct = direct_term(100, 30, &expected_bytes);
    expect_same_screen(&mut client, &direct.snapshot(true).unwrap(), WAIT);

    for (cols, rows) in [(70, 20), (140, 45), (100, 30)] {
        client.resize(cols, rows + 1);
        direct.resize(cols, rows).unwrap();
        let want = direct.snapshot(true).unwrap();
        expect_same_screen(&mut client, &want, WAIT);
    }
}

#[test]
fn a_new_client_gets_the_whole_screen() {
    let daemon = TestDaemon::start();
    let data = generate(Workload::Colored, 100 * 1024, 9);
    let file = daemon.dir().join("load.bin");
    std::fs::write(&file, &data).unwrap();
    let mut options = AttachOptions::new().size(100, 31);
    options.command = cat_then_mark(&file);
    let mut first = daemon.attach(options).unwrap();
    first
        .wait_for_text_within("DONE-MARK", Duration::from_secs(30))
        .unwrap();
    let want = client_frame(&mut first);
    // Same session, new client, same size: everything must be drawn for it.
    let mut second = daemon
        .attach(AttachOptions::new().existing(None).size(100, 31))
        .unwrap();
    expect_same_screen(&mut second, &want, WAIT);
}

#[test]
fn a_flooding_pane_costs_the_client_at_most_sixty_updates_a_second() {
    let daemon = TestDaemon::start();
    let mut options = AttachOptions::new();
    options.command = vec![
        "/usr/bin/yes".into(),
        "a line of output that never stops".into(),
    ];
    let mut client = daemon.attach(options).unwrap();
    // Let it get going, then count for a while.
    client.wait_for_text("a line of output").unwrap();
    let warmup = Instant::now() + Duration::from_millis(500);
    while Instant::now() < warmup {
        client.pump(warmup);
    }
    let before = client.output_messages;
    let start = Instant::now();
    let end = start + Duration::from_secs(3);
    while Instant::now() < end {
        client.pump(end);
    }
    let seconds = start.elapsed().as_secs_f64();
    let updates = (client.output_messages - before) as f64;
    let rate = updates / seconds;
    eprintln!("flood: {updates} updates in {seconds:.2} s = {rate:.1} a second");
    assert!(
        rate <= 60.0,
        "{updates} updates in {seconds:.2} s = {rate:.1} a second"
    );
    // And it keeps drawing: the flood is not starved.
    assert!(rate >= 30.0, "only {rate:.1} updates a second");
}

#[test]
fn changing_one_character_sends_only_a_few_bytes() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    // Fill the screen so that a full redraw would be large.
    client.type_line("seq 1 40");
    client.wait_for_row("40").unwrap();
    client.wait_for_prompt().unwrap();
    // Let anything still on its way arrive.
    let settle = Instant::now() + Duration::from_millis(300);
    while Instant::now() < settle {
        client.pump(settle);
    }
    let before = client.output_bytes;
    client.send(b"x");
    client.wait_for_row("$ x").unwrap();
    let sent = client.output_bytes - before;
    assert!(sent < 300, "one typed character cost {sent} bytes");
    assert!(sent > 0);
}

#[test]
fn the_pane_s_key_modes_are_shared_with_the_host_terminal() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("printf '\\033[?1h\\033[?2004h'; printf 'modes-on\\n'");
    client.wait_for_row("modes-on").unwrap();
    assert!(client.wait_until(WAIT, |c| {
        c.screen.mode(Mode::AppCursorKeys) == Some(true)
            && c.screen.mode(Mode::BracketedPaste) == Some(true)
    }));
    client.type_line("printf '\\033[?1l\\033[?2004l'; printf 'modes-off\\n'");
    client.wait_for_row("modes-off").unwrap();
    assert!(client.wait_until(WAIT, |c| {
        c.screen.mode(Mode::AppCursorKeys) == Some(false)
            && c.screen.mode(Mode::BracketedPaste) == Some(false)
    }));
}

#[test]
fn cursor_shape_and_visibility_follow_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("printf '\\033[6 q\\033[?25l'; printf 'cursor-set\\n'");
    client.wait_for_row("cursor-set").unwrap();
    let ok = client.wait_until(WAIT, |c| {
        let f = client_frame(c);
        f.cursor.shape == arcade_term::CursorShape::Bar && !f.cursor.visible
    });
    assert!(ok, "cursor {:?}", client_frame(&mut client).cursor);
}

// ---- A client that does not read ----

fn resident_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("rss")
}

fn wait_for_file(path: &Path, timeout: Duration) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < timeout,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_client_that_stops_reading_does_not_make_the_daemon_grow() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    let script = format!(
        "cd '{d}'; seq -f %.0f 1 3000000; touch phase1; seq -f %.0f 1 12000000; touch phase2; sleep 120",
        d = dir.display()
    );
    let mut options = AttachOptions::new().size(100, 30);
    options.command = vec!["/bin/sh".into(), "-c".into(), script];
    let mut client = daemon.attach(options).unwrap();
    let started = Instant::now();
    // From here on the client reads nothing for at least five seconds.
    wait_for_file(&dir.join("phase1"), Duration::from_secs(60));
    let after_first = resident_kib();
    wait_for_file(&dir.join("phase2"), Duration::from_secs(120));
    while started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let after_second = resident_kib();
    // The second phase wrote about four times more than the first. A daemon that kept what the
    // client did not take would have grown by tens of megabytes.
    let growth = after_second.saturating_sub(after_first);
    eprintln!(
        "slow client: process memory {after_first} KiB after the first phase, {after_second} KiB \
         after the second (about 100 MB more output); growth {growth} KiB"
    );
    assert!(
        growth < 16 * 1024,
        "grew by {growth} KiB while the client did not read"
    );

    // Reading again gives the latest state, not the history of updates.
    client.wait_for_row("12000000").unwrap();
    eprintln!(
        "slow client: {} bytes of updates received once it read again",
        client.output_bytes
    );
    // The updates that built up in the socket are few: it is the state that was kept.
    assert!(
        client.output_bytes < 2 * 1024 * 1024,
        "{} bytes of updates were waiting",
        client.output_bytes
    );
}

// ---- A real full-screen program ----

fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .or_else(|| {
            ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
                .iter()
                .map(|d| Path::new(d).join(name))
                .find(|p| p.is_file())
        })
}

const SAMPLE: &str = "// A sample file for the editor.\nuse std::collections::HashMap;\n\n#[derive(Debug, Clone)]\nstruct Point {\n    x: f64,\n    y: f64,\n}\n\nfn main() {\n    let mut seen: HashMap<String, Point> = HashMap::new();\n    for i in 0..10 {\n        let p = Point { x: i as f64, y: (i * i) as f64 };\n        seen.insert(format!(\"point-{i}\"), p);\n    }\n    println!(\"{} points, first is {:?}\", seen.len(), seen.get(\"point-0\"));\n}\n\n// Vietnamese: Tiếng Việt, and wide: 汉字\n";

/// Core scenario 4, as far as it can be automated: Neovim in a pane draws the same screen as
/// Neovim run straight in a terminal of the same size, at start-up and after some editing.
#[test]
fn neovim_in_a_pane_looks_the_same_as_neovim_alone() {
    let Some(nvim) = find_program("nvim") else {
        eprintln!("neovim is not installed; skipping");
        return;
    };
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    std::fs::write(dir.join("sample.rs"), SAMPLE).unwrap();
    // Neovim keeps files here; with the folders in place neither run prints a complaint.
    for sub in ["cfg", "data", "state/nvim", "cache"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    let env: Vec<String> = vec![
        "PATH=/usr/bin:/bin:/usr/sbin:/sbin".into(),
        format!("HOME={}", dir.display()),
        format!("XDG_CONFIG_HOME={}/cfg", dir.display()),
        format!("XDG_DATA_HOME={}/data", dir.display()),
        format!("XDG_STATE_HOME={}/state", dir.display()),
        format!("XDG_CACHE_HOME={}/cache", dir.display()),
        "LANG=en_US.UTF-8".into(),
        "TERM=xterm-256color".into(),
    ];
    let args: Vec<String> = [
        "-u",
        "NONE",
        "-i",
        "NONE",
        "-n",
        "--noplugin",
        "-c",
        "syntax on",
        "-c",
        "set filetype=rust",
        "-c",
        "set number",
        "-c",
        "set nowrap",
        "sample.rs",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let mut argv = vec![nvim.display().to_string()];
    argv.extend(args.iter().cloned());
    let mut options = AttachOptions::new().size(100, 31);
    options.command = argv;
    options.env = Some(env.clone());
    let mut client = daemon.attach(options).unwrap();

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut alone = PtyHost::spawn(&nvim, &arg_refs, env, &dir, 100, 30);
    // Neovim asks the terminal questions as it starts and gives up quickly when nobody answers.
    // The pane's questions are answered by the daemon at once; this stand-in for a terminal has
    // to be reading to do the same.
    alone.wait_until(Duration::from_millis(1500), |_| false);

    // Take the editing in steps: each time both must settle on the same screen.
    let steps: [(&str, &[u8]); 5] = [
        ("start-up", b""),
        ("insert text", b"ggOinserted line\x1b"),
        ("scroll down", b"\x04\x04"),
        ("command line", b":set list\r"),
        ("go to the end", b"G$"),
    ];
    for (name, keys) in steps {
        if !keys.is_empty() {
            client.send(keys);
            alone.send(keys);
        }
        // Wait for both to show the same thing, within a time limit.
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            client.wait_until(Duration::from_millis(300), |_| false);
            alone.wait_until(Duration::from_millis(300), |_| false);
            let got = client_frame(&mut client);
            let want = alone.screen.snapshot(true).unwrap();
            let last = match first_difference(&got, &want) {
                None => break,
                Some(d) => d,
            };
            assert!(
                Instant::now() < deadline,
                "{name}: the pane differs from neovim alone: {last}\n--- in the pane ---\n{}\n--- alone ---\n{}",
                client.screen_text(),
                alone.screen_text()
            );
        }
        assert!(
            client
                .screen_rows()
                .iter()
                .any(|r| r.contains("fn main") || r.contains("inserted line")),
            "{name}: neovim does not seem to have started:\n{}",
            client.screen_text()
        );
    }
}

#[test]
fn an_echoed_key_is_drawn_without_waiting_for_the_frame_interval() {
    let daemon = TestDaemon::start();
    let mut options = AttachOptions::new();
    options.command = vec!["/bin/cat".into()];
    let mut client = daemon.attach(options).unwrap();
    let settle = Instant::now() + Duration::from_millis(300);
    while Instant::now() < settle {
        client.pump(settle);
    }
    // Keys 5 ms apart, faster than the 60-a-second limit would allow if it applied to them.
    let mut waits = Vec::new();
    for _ in 0..100 {
        let before = client.output_messages;
        let sent = Instant::now();
        client.send(b"a");
        let deadline = sent + Duration::from_secs(2);
        while client.output_messages == before && Instant::now() < deadline {
            client.pump(deadline);
        }
        assert!(
            client.output_messages > before,
            "an echoed key was never drawn"
        );
        waits.push(sent.elapsed());
        let next = Instant::now() + Duration::from_millis(5);
        while Instant::now() < next {
            client.pump(next);
        }
    }
    waits.sort();
    let median = waits[waits.len() / 2];
    let p99 = waits[waits.len() - 2];
    eprintln!("echo drawn after: median {median:?}, p99 {p99:?}");
    assert!(median < Duration::from_millis(3), "median {median:?}");
    assert!(p99 < Duration::from_millis(15), "p99 {p99:?}");
}
