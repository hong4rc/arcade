//! No timer runs for a drag that is not at an edge of its pane, and none when the drag is over:
//! the daemon costs no processor time. This file holds one test, so no other test shares the
//! process (the daemon runs on a thread of it).

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, cpu_time};

fn idle_for(client: &mut TestClient, wait: Duration) -> Duration {
    let before = cpu_time();
    let end = Instant::now() + wait;
    while Instant::now() < end {
        client.read_more(end);
    }
    cpu_time() - before
}

fn mouse(client: &mut TestClient, code: u16, x: u16, y: u16, up: bool) {
    let end = if up { 'm' } else { 'M' };
    client.send(format!("\x1b[<{code};{};{}{end}", x + 1, y + 1).as_bytes());
}

#[test]
fn a_drag_inside_the_pane_and_a_finished_drag_cost_no_processor_time() {
    let daemon = TestDaemon::start();
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("idle"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line(
        "i=1; while [ $i -le 100 ]; do echo line-$i; i=$((i+1)); done; echo filled-$((1+1))",
    );
    client.wait_for_text("filled-2").unwrap();
    let quiet = idle_for(&mut client, Duration::from_secs(3));
    // A selection held in the middle of the pane.
    mouse(&mut client, 0, 3, 15, false);
    mouse(&mut client, 32, 8, 12, false);
    let inside = idle_for(&mut client, Duration::from_secs(3));
    // At the edge until the top of the history, the timer stops there.
    mouse(&mut client, 32, 8, 0, false);
    let _ = idle_for(&mut client, Duration::from_secs(4));
    let at_top = idle_for(&mut client, Duration::from_secs(3));
    mouse(&mut client, 0, 8, 0, true);
    let after = idle_for(&mut client, Duration::from_secs(3));
    eprintln!(
        "idle over 3 s: quiet {quiet:?}, drag inside {inside:?}, at the top {at_top:?}, after {after:?}"
    );
    for (what, t) in [("inside", inside), ("at the top", at_top), ("after", after)] {
        assert!(t < Duration::from_millis(100), "{what}: {t:?}");
        assert!(
            t <= quiet + Duration::from_millis(50),
            "{what}: {t:?} against {quiet:?}"
        );
    }
}
