//! A signal that was ignored where the daemon was started (a script's `&` job, `nohup`) must not
//! stay ignored inside the panes: ^C has to stop the program in front, as it does in any terminal.

use std::time::Duration;

use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT};

#[test]
fn ctrl_c_stops_a_program_in_a_pane_even_when_the_daemon_was_started_ignoring_it() {
    // This file is its own test program, so ignoring the signal here touches no other test.
    // SAFETY: setting the disposition of a signal before anything is started.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    }
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("sig"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    // `sleep` ends on ^C, and then the echo runs; if the signal were ignored the echo would only
    // come after the 60 seconds.
    client.type_line("sleep 60; echo stopped-$((1+1))");
    client.wait_until(Duration::from_millis(1000), |_| false);
    client.send(b"\x03");
    client.type_line("echo after-$((2+2))");
    assert!(
        client.wait_until(WAIT, |c| c.screen_text().contains("after-4")),
        "{}",
        client.screen_text()
    );
}
