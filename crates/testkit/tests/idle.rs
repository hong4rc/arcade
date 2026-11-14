//! A daemon with a client attached and notifications standing must cost nothing when nothing
//! happens: no timer that keeps waking it, no work for a bell that already rang. Its processor
//! time is the processor time of this process while the test only waits (the daemon runs on a
//! thread of it). This file holds one test, so no other test shares the process.

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT, cpu_time};

const PREFIX: &[u8] = b"\x1b[109;5u";

fn idle_for(client: &mut arcade_testkit::TestClient, wait: Duration) -> Duration {
    let before = cpu_time();
    let end = Instant::now() + wait;
    while Instant::now() < end {
        client.read_more(end);
    }
    cpu_time() - before
}

#[test]
fn standing_notifications_and_rung_bells_cost_no_processor_time_when_idle() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    let mut panes = vec![client.focused_pane_id()];
    for _ in 0..3 {
        client.send(PREFIX);
        client.send(b"c");
        client.wait_for_prompt().unwrap();
        panes.push(client.focused_pane_id());
    }
    let quiet = idle_for(&mut client, Duration::from_secs(1));
    let without = idle_for(&mut client, Duration::from_secs(5));

    // The terminal is somewhere else; every class stands, bells rang and waited for their
    // interval, and the interval has ended.
    client.send(b"\x1b[O");
    for (pane, class) in panes.iter().zip(["approval", "input", "error", "info"]) {
        let target = format!("target={pane}");
        let class = format!("class={class}");
        for n in 0..3 {
            let message = format!("message=signal {n}");
            let (ok, why) = daemon
                .command(
                    "notify.send",
                    &["version=1", &target, &class, "source=agent", &message],
                )
                .unwrap();
            assert!(ok, "{why}");
        }
    }
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("[!]1")));
    let settling = idle_for(&mut client, Duration::from_secs(3));
    let with = idle_for(&mut client, Duration::from_secs(5));
    eprintln!(
        "idle processor time over 5 s: {without:?} without notifications, {with:?} with four standing (a second of quiet: {quiet:?}, while the bells settle: {settling:?})"
    );
    assert!(with < Duration::from_millis(100), "{with:?}");
    assert!(
        with <= without + Duration::from_millis(50),
        "{with:?} against {without:?}"
    );
}
