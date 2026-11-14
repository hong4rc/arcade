//! A session ends while the client is in it: the client goes to another session, as the
//! settings say (`[session] on-end`), or leaves as it did before.

use std::path::PathBuf;

use arcade_proto::ServerMessage;
use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

/// A session with a line of its own on the screen and no client.
fn detached_session(daemon: &TestDaemon, name: &str) {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named(name))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line(&format!("echo only-in-{name}"));
    client.wait_for_row(&format!("only-in-{name}")).unwrap();
    client.detach();
    assert!(client.wait_closed(WAIT));
}

fn attached_session(daemon: &TestDaemon, name: &str) -> TestClient {
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named(name))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client
}

fn end_the_session(client: &mut TestClient) {
    client.type_line("exit");
}

#[test]
fn the_client_goes_to_the_other_session_when_its_own_ends() {
    let daemon = TestDaemon::start();
    detached_session(&daemon, "beta");
    let mut client = attached_session(&daemon, "alpha");
    end_the_session(&mut client);
    client.wait_for_row("only-in-beta").unwrap();
    assert!(
        client.wait_until(WAIT, |c| c
            .status_row()
            .contains("session alpha ended; now in beta")),
        "{}",
        client.status_row()
    );
    assert!(!client.is_closed());
    // The client is in beta: what it types goes to beta's shell.
    client.type_line("echo typed-after-the-switch");
    client.wait_for_row("typed-after-the-switch").unwrap();
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert!(list.contains("beta") && !list.contains("alpha"), "{list}");
}

#[test]
fn it_goes_to_the_session_used_last_and_not_to_the_first_by_name() {
    let daemon = TestDaemon::start();
    detached_session(&daemon, "aaa");
    detached_session(&daemon, "bbb");
    let mut client = attached_session(&daemon, "ccc");
    end_the_session(&mut client);
    client.wait_for_row("only-in-bbb").unwrap();
    assert!(
        client.status_row().contains("now in bbb")
            || client.wait_until(WAIT, |c| c.status_row().contains("now in bbb"))
    );
}

#[test]
fn a_session_that_has_a_client_of_its_own_is_not_taken() {
    let daemon = TestDaemon::start();
    let mut other = attached_session(&daemon, "beta");
    let mut client = attached_session(&daemon, "alpha");
    end_the_session(&mut client);
    // Nobody to go to: the client leaves as before, and beta's client stays.
    assert!(
        client
            .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
            .is_ok()
    );
    other.type_line("echo still-here");
    other.wait_for_row("still-here").unwrap();
}

#[test]
fn the_last_session_ending_ends_the_client_as_before() {
    let mut daemon = TestDaemon::start();
    let mut client = attached_session(&daemon, "only");
    end_the_session(&mut client);
    client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    daemon.wait_stopped(WAIT).unwrap();
}

fn config_file(text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("arc-sessionend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{}.toml", text.len()));
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn on_end_detach_keeps_the_old_behaviour() {
    let file = config_file("[session]\non-end = \"detach\"\n");
    let daemon = TestDaemon::start_with(move |c| c.config_path = Some(file));
    detached_session(&daemon, "beta");
    let mut client = attached_session(&daemon, "alpha");
    end_the_session(&mut client);
    client
        .wait_message(|m| matches!(m, ServerMessage::SessionEnded { .. }), WAIT)
        .unwrap();
    let (_, list) = daemon.command("session.list", &[]).unwrap();
    assert!(list.contains("beta"), "{list}");
}

#[test]
fn closing_the_session_from_outside_follows_the_same_rule() {
    let daemon = TestDaemon::start();
    detached_session(&daemon, "beta");
    let mut client = attached_session(&daemon, "alpha");
    daemon.command("session.close", &["alpha"]).unwrap();
    client.wait_for_row("only-in-beta").unwrap();
    assert!(!client.is_closed());
}
