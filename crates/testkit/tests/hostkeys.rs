//! The question the daemon asks the host terminal when a client attaches (does it have the kitty
//! keyboard protocol?), and the one line on the status bar about bindings that cannot work there.
//! (Key spec, section 8, scenario 11.)

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// What a terminal with old key codes answers: its attributes, and nothing for the keyboard.
const LEGACY_ANSWER: &[u8] = b"\x1b[?62;22c";
/// What a terminal with the protocol answers: the keyboard flags, then its attributes.
const KITTY_ANSWER: &[u8] = b"\x1b[?1u\x1b[?62;22c";

fn bar(client: &TestClient) -> String {
    client.status_row()
}

fn config_file(text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-hostkeys-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("config.toml");
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn a_terminal_with_old_key_codes_gets_one_line_about_the_bindings_it_cannot_send() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(LEGACY_ANSWER);
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains("key bindings cannot work here")),
        "{}",
        bar(&client)
    );
    let line = bar(&client);
    assert!(line.contains("(first: "), "{line}");
    assert!(line.contains("arcade keys list"), "{line}");
}

#[test]
fn a_terminal_with_the_protocol_gets_no_line() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(KITTY_ANSWER);
    // The answer was taken, so the keyboard is working with the next thing typed.
    client.type_line("echo marker-$((1+2))");
    client.wait_for_text("marker-3").unwrap();
    assert!(!bar(&client).contains("cannot work"), "{}", bar(&client));
}

#[test]
fn the_answers_do_not_reach_the_program_in_the_pane() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.type_line("cat -v");
    client.send(KITTY_ANSWER);
    client.send(b"seen\r");
    client.wait_for_text("seen").unwrap();
    assert!(
        !client.screen_text().contains("^[[?"),
        "{}",
        client.screen_text()
    );
}

#[test]
fn a_sequence_like_an_answer_is_the_programs_when_nothing_is_being_asked() {
    let daemon = TestDaemon::start();
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(KITTY_ANSWER);
    client.type_line("cat -v");
    // The question was answered: another such sequence is for the pane.
    client.send(LEGACY_ANSWER);
    client.send(b"\r");
    client.wait_for_text("^[[?62;22c").unwrap();
}

#[test]
fn reloading_the_configuration_names_the_bindings_that_cannot_work() {
    let file = config_file(
        "[keys]\nprefix = \"ctrl+g\"\ndirect-modifier = \"none\"\n[keys.after-prefix]\n\"ctrl+2\" = \"pane.zoom\"\n",
    );
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.send(LEGACY_ANSWER);
    assert!(
        client.wait_until(WAIT, |c| bar(c)
            .contains("1 key binding cannot work here (first: ctrl+2)")),
        "{}",
        bar(&client)
    );
    // The file is mended: the reload says so without the line.
    std::fs::write(
        &file,
        "[keys]\nprefix = \"ctrl+g\"\ndirect-modifier = \"none\"\n",
    )
    .unwrap();
    client.send(b"\x1b[103;5ur");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains("configuration reloaded")),
        "{}",
        bar(&client)
    );
    assert!(!bar(&client).contains("cannot work"), "{}", bar(&client));
    // And broken again: the reload carries the line.
    std::fs::write(
        &file,
        "[keys]\nprefix = \"ctrl+g\"\ndirect-modifier = \"none\"\n[keys.after-prefix]\n\"ctrl+3\" = \"pane.zoom\"\n",
    )
    .unwrap();
    client.send(b"x");
    client.send(b"\x1b[103;5ur");
    assert!(
        client.wait_until(WAIT, |c| bar(c).contains(
            "configuration reloaded; 1 key binding cannot work here (first: ctrl+3)"
        )),
        "{}",
        bar(&client)
    );
}

#[test]
fn a_terminal_that_has_not_answered_is_not_blamed() {
    let file = config_file(
        "[keys]\nprefix = \"ctrl+g\"\ndirect-modifier = \"none\"\n[keys.after-prefix]\n\"ctrl+2\" = \"pane.zoom\"\n",
    );
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(path));
    let mut client = daemon.attach_shell();
    client.wait_for_prompt().unwrap();
    client.send(b"\x1b[103;5ur");
    assert!(client.wait_until(WAIT, |c| bar(c).contains("configuration reloaded")));
    assert!(!bar(&client).contains("cannot work"), "{}", bar(&client));
}
