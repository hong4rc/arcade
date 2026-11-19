//! A short message on the bar ("no notification is waiting", "config reloaded") is painted over
//! the right end of the bar: the tabs, the counter and the panes stay exactly as they are, and
//! the cells under the message come back, as they were, when it goes.

use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT};
use std::time::Duration;

fn windows(daemon: &TestDaemon, count: usize) {
    for _ in 0..count {
        let (ok, why) = daemon.command("window.new", &[]).unwrap();
        assert!(ok, "{why}");
    }
}

#[test]
fn a_message_covers_the_right_end_of_the_bar_and_nothing_is_laid_out_again() {
    for width in [100u16, 60] {
        let daemon = TestDaemon::start();
        let mut client = daemon
            .attach(AttachOptions::new().size(width, 30).named("work"))
            .unwrap();
        client.wait_for_prompt().unwrap();
        windows(&daemon, 8);
        client.wait_for_prompt().unwrap();
        client.type_line("stty size");
        assert!(
            client.wait_until(WAIT, |c| c.screen_text().contains(&format!("29 {width}"))),
            "{}",
            client.screen_text()
        );
        let before = client.status_row();

        // A message that says there is nothing to go to.
        client.send(b"\x1b[109;5u");
        client.send(b"a");
        let said = client.wait_until(Duration::from_secs(2), |c| {
            c.status_row().contains("no notification")
        });
        assert!(said, "{width}: {}", client.status_row());
        let during = client.status_row();

        // Everything left of the message is as it was: the rows agree up to the right half.
        let same = before
            .chars()
            .zip(during.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let half = usize::from(width) / 2;
        assert!(
            same + 8 >= half,
            "{width}: the row changed at column {same}:\n{before}\n{during}"
        );
        // The panes were not resized: the same size is read from the shell, and the pane area
        // is the same on the screen.
        client.type_line("stty size");
        assert!(client.wait_until(WAIT, |c| {
            c.screen_text().matches(&format!("29 {width}")).count() >= 2
        }));
        assert_eq!(client.screen_rows().len(), usize::from(30u16), "{width}");

        // When it goes, the row is what it was before.
        let gone = client.wait_until(Duration::from_secs(12), |c| {
            !c.status_row().contains("no notification")
        });
        assert!(gone, "{width}: {}", client.status_row());
        assert_eq!(client.status_row(), before, "{width}");
    }
}
