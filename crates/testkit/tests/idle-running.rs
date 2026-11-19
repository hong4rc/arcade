//! One running tab and nothing else, with a client attached: the processor time of the daemon
//! while the test only waits. A running tab takes turns between its symbol and its number (one
//! phase per `alternate-interval`), so it costs a redraw of the bar once a phase; `replace` is
//! the look before the turns, and is measured beside it for comparison.

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT, cpu_time};

fn idle_for(client: &mut TestClient, wait: Duration) -> Duration {
    let before = cpu_time();
    let end = Instant::now() + wait;
    while Instant::now() < end {
        client.read_more(end);
    }
    cpu_time() - before
}

fn one_running_tab(alternate: bool) -> Duration {
    let daemon = TestDaemon::start_with(move |c| c.config.state_look.alternate = alternate);
    let mut client = daemon
        .attach(AttachOptions::new().size(100, 30).named("work"))
        .unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line("printf '\\033]9;4;3;\\007'");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains('▸')));
    idle_for(&mut client, Duration::from_secs(2));
    idle_for(&mut client, Duration::from_secs(5))
}

#[test]
fn one_running_tab_costs_a_redraw_a_phase_and_no_more() {
    let with = one_running_tab(true);
    let replace = one_running_tab(false);
    eprintln!(
        "idle processor time over 5 s, one running tab: alternate {with:?}, replace {replace:?}"
    );
    assert!(with < Duration::from_millis(50), "{with:?}");
}
