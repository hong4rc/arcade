//! `arcade keys list`, `arcade keys test` and the warnings of `arcade config check`: the real
//! command, with a configuration file of its own. (Key spec, section 9; scenarios 11, 13, 14.)

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use arcade_testkit::{PtyHost, Screened, WAIT};

const ARCADE: &str = env!("CARGO_BIN_EXE_arcade");

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A private directory, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let dir = PathBuf::from(format!(
            "/tmp/arc-keys-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }

    fn env(&self) -> Vec<String> {
        vec![
            "PATH=/usr/bin:/bin".to_string(),
            format!("HOME={}", self.0.display()),
            "TERM=xterm-256color".to_string(),
        ]
    }

    /// Run `arcade` with no terminal and collect what it prints.
    fn plain(&self, args: &[&str]) -> Output {
        let mut command = Command::new(ARCADE);
        command.args(args).env_clear().stdin(Stdio::null());
        for entry in self.env() {
            let (k, v) = entry.split_once('=').unwrap();
            command.env(k, v);
        }
        command.output().expect("run arcade")
    }

    fn host(&self, args: &[&str]) -> PtyHost {
        PtyHost::spawn(Path::new(ARCADE), args, self.env(), &self.0, 100, 30)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

const CHANGED: &str =
    "[keys]\nnav = \"hnei\"\n[keys.prefix]\n\"b\" = \"pane.zoom\"\n\"x\" = false\n";

#[test]
fn keys_list_prints_every_table_with_where_each_binding_comes_from() {
    let dir = Dir::new();
    let file = dir.file("c.toml", CHANGED);
    let o = dir.plain(&["keys", "list", &file]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let text = out(&o);
    for table in ["[direct]", "[prefix]", "[copy]", "[list]"] {
        assert!(text.lines().any(|l| l == table), "{table}: {text}");
    }
    let line = |needle: &str| {
        text.lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {text}"))
            .to_string()
    };
    assert!(line("pane.zoom").trim_end().ends_with("config"), "{text}");
    assert!(
        line("pane.focus-left").trim_end().ends_with("default"),
        "{text}"
    );
    // The movement keys the file changed come from it.
    let moved = text
        .lines()
        .find(|l| l.trim_start().starts_with("ctrl+alt+n ") && l.contains("pane.focus-down"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(moved.trim_end().ends_with("config"), "{moved}");
    // A default taken away with `false` is no longer in force.
    assert!(
        !text.lines().any(|l| l.contains("pane.close")),
        "pane.close is listed: {text}"
    );
}

#[test]
fn keys_list_gives_the_same_text_every_time() {
    let dir = Dir::new();
    let file = dir.file("c.toml", CHANGED);
    let first = out(&dir.plain(&["keys", "list", &file, "--host", "kitty"]));
    assert!(first.lines().count() > 50, "{first}");
    for _ in 0..10 {
        let again = out(&dir.plain(&["keys", "list", &file, "--host", "kitty"]));
        assert_eq!(again, first);
    }
}

#[test]
fn keys_list_reads_the_file_in_the_home_directory_without_a_daemon() {
    let dir = Dir::new();
    std::fs::create_dir_all(dir.0.join(".config/arcade")).unwrap();
    std::fs::write(
        dir.0.join(".config/arcade/config.toml"),
        "[keys.prefix]\n\"b\" = \"pane.zoom\"\n",
    )
    .unwrap();
    let o = dir.plain(&["keys", "list"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        out(&o)
            .lines()
            .any(|l| l.contains("pane.zoom") && l.trim_end().ends_with("config"))
    );
}

#[test]
fn keys_list_says_what_is_wrong_with_a_bad_file_and_prints_no_table() {
    let dir = Dir::new();
    let file = dir.file("bad.toml", "[keys.prefix]\n\"z\" = \"pane.fly\"\n");
    let o = dir.plain(&["keys", "list", &file]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("line 2"), "{}", err(&o));
    assert!(out(&o).is_empty(), "{}", out(&o));
}

#[test]
fn a_binding_a_terminal_without_the_protocol_cannot_send_is_warned_about() {
    // Scenario 11.
    let dir = Dir::new();
    let file = dir.file("c.toml", "[keys.prefix]\n\"ctrl+1\" = \"pane.zoom\"\n");
    let legacy = dir.plain(&["config", "check", "--host", "legacy", &file]);
    assert_eq!(legacy.status.code(), Some(0), "{}", err(&legacy));
    let text = out(&legacy);
    assert!(text.starts_with("ok:"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("warning: [keys.prefix] ctrl+1:") && l.contains("digit")),
        "{text}"
    );
    let kitty = dir.plain(&["config", "check", "--host", "kitty", &file]);
    assert!(!out(&kitty).contains("warning:"), "{}", out(&kitty));
    // With no terminal to ask and no word about it, nothing is claimed.
    let unknown = dir.plain(&["config", "check", &file]);
    assert!(!out(&unknown).contains("warning:"), "{}", out(&unknown));
    assert!(out(&unknown).contains("not known"), "{}", out(&unknown));
}

#[test]
fn the_default_prefix_is_named_when_the_terminal_has_old_key_codes_only() {
    let dir = Dir::new();
    let o = dir.plain(&["config", "check", "--host", "legacy"]);
    let text = out(&o);
    assert!(text.contains("no file"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.starts_with("warning: [keys] ctrl+m: ") && l.contains("Enter")),
        "{text}"
    );
}

#[test]
fn keys_list_ends_with_the_same_warnings() {
    let dir = Dir::new();
    let file = dir.file(
        "c.toml",
        "[keys]\nprefix = \"ctrl+g\"\n[keys.after-prefix]\n\"ctrl+2\" = \"pane.zoom\"\n",
    );
    let o = dir.plain(&["keys", "list", "--host", "legacy", &file]);
    let text = out(&o);
    assert!(
        text.lines()
            .any(|l| l.starts_with("warning: [keys.prefix] ctrl+2:")),
        "{text}"
    );
    assert!(!text.contains("warning: [keys] "), "{text}");
}

#[test]
fn a_bad_host_word_is_a_usage_error() {
    let dir = Dir::new();
    let o = dir.plain(&["keys", "list", "--host", "weird"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("kitty or legacy"), "{}", err(&o));
}

/// Ctrl+Alt+`h` arrives as the name of the key and what it does; the same letter with Alt alone
/// is another key, the one the system would have taken (scenario 14).
#[test]
fn keys_test_shows_the_name_of_each_key_and_what_it_does() {
    let dir = Dir::new();
    let file = dir.file("c.toml", "[keys]\nprefix = \"ctrl+g\"\n");
    let mut host = dir.host(&["keys", "test", &file]);
    host.wait_for_text("Ctrl+C quits").unwrap();
    host.wait_for_text("host terminal:").unwrap();
    host.send(b"\x1b[104;7u");
    host.wait_for_text("ctrl+alt+h").unwrap();
    host.wait_for_text("direct: pane.focus-left").unwrap();
    host.send(b"\x1bx");
    host.wait_for_text("alt+x").unwrap();
    host.wait_for_text("no binding").unwrap();
    // The prefix key says so, and so does a key of the table after it.
    host.send(b"\x07");
    host.wait_for_text("the prefix key").unwrap();
    host.send(b"z");
    host.wait_for_text("after the prefix: pane.zoom").unwrap();
    host.send(b"\x03");
    assert_eq!(host.wait_exit(WAIT), Some(0), "{}", host.screen_text());
}

#[test]
fn keys_test_gives_the_terminal_back_by_every_way_out() {
    let dir = Dir::new();
    for way in ["ctrl-c", "TERM", "HUP", "INT"] {
        // The program starts after the terminal's first settings were read, however fast it is.
        let mut host = PtyHost::spawn(
            Path::new("/bin/sh"),
            &["-c", "sleep 0.3; exec \"$0\" keys test", ARCADE],
            dir.env(),
            &dir.0,
            100,
            30,
        );
        host.wait_for_text("Ctrl+C quits").unwrap();
        assert_ne!(&host.settings(), host.initial_settings(), "{way}");
        match way {
            "ctrl-c" => host.send(b"\x03"),
            "TERM" => host.signal(libc::SIGTERM),
            "HUP" => host.signal(libc::SIGHUP),
            _ => host.signal(libc::SIGINT),
        }
        assert_eq!(
            host.wait_exit(WAIT),
            Some(0),
            "{way}: {}",
            host.screen_text()
        );
        assert_eq!(&host.settings(), host.initial_settings(), "{way}");
        // The keyboard protocol is turned off again.
        assert!(
            host.raw.windows(4).any(|w| w == b"\x1b[<u"),
            "{way}: the keyboard protocol was left on"
        );
    }
}

#[test]
fn keys_test_without_a_terminal_says_so() {
    let dir = Dir::new();
    let o = dir.plain(&["keys", "test"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("needs a terminal"), "{}", err(&o));
}
