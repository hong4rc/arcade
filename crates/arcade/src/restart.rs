//! `arcade upgrade` against a daemon that is older than `arcade upgrade` itself: it cannot hand its
//! sessions over, and what it can tell a client is very little (see `docs/decisions/035`). So the
//! only move left is a restart that keeps what can be known: the sessions by name, how many
//! windows each had, and the directories of the panes as the operating system shows them. This
//! module says what is known and what will end, asks, and then does it.

use std::io::{BufRead, IsTerminal, Write};
use std::process::Command;
use std::time::{Duration, Instant};

use arcade_proto::{AttachRequest, ClientMessage, CreateMode, ServerMessage};

use crate::connection::{Connection, Handshake, connect, handshake};

/// One session of the old daemon, as `session.list` tells it.
#[derive(Debug, PartialEq, Eq)]
pub struct OldSession {
    pub name: String,
    pub windows: usize,
    pub panes: usize,
}

/// One pane of the old daemon as the operating system shows its first process.
#[derive(Debug, PartialEq, Eq)]
pub struct OldPane {
    pub pid: i32,
    pub cwd: Option<String>,
    /// What runs in the foreground of the pane's terminal, when it is not the shell itself.
    pub running: Option<String>,
}

pub fn parse_sessions(list: &str) -> Vec<OldSession> {
    list.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let name = f.next()?.to_string();
            let _id = f.next()?;
            let _attached = f.next()?;
            Some(OldSession {
                name,
                windows: f.next()?.parse().ok()?,
                panes: f.next()?.parse().ok()?,
            })
        })
        .collect()
}

struct Process {
    pid: i32,
    ppid: i32,
    pgid: i32,
    tpgid: i32,
    tty: String,
    command: String,
}

fn processes() -> Vec<Process> {
    let Ok(out) = Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,tpgid=,tty=,command="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let pgid = f.next()?.parse().ok()?;
            let tpgid = f.next()?.parse().ok()?;
            let tty = f.next()?.to_string();
            let command = f.collect::<Vec<_>>().join(" ");
            Some(Process {
                pid,
                ppid,
                pgid,
                tpgid,
                tty,
                command,
            })
        })
        .collect()
}

fn cwd_of(pid: i32) -> Option<String> {
    if let Ok(p) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
        return Some(p.display().to_string());
    }
    let out = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(str::to_string))
}

fn program_name(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or(command);
    first.rsplit('/').next().unwrap_or(first).to_string()
}

/// The old daemon's panes in the order they were made (their process numbers rise), found from
/// the daemon's own process: the one started as `__daemon` on this socket.
pub fn old_panes(socket: &std::path::Path) -> Vec<OldPane> {
    let all = processes();
    let socket = socket.display().to_string();
    let Some(daemon) = all
        .iter()
        .find(|p| p.command.contains("__daemon") && p.command.contains(&socket))
    else {
        return Vec::new();
    };
    let mut children: Vec<&Process> = all
        .iter()
        .filter(|p| {
            p.ppid == daemon.pid
                && !p.command.contains("caffeinate")
                && !p.command.contains("systemd-inhibit")
        })
        .collect();
    children.sort_by_key(|p| p.pid);
    children
        .into_iter()
        .map(|shell| {
            // The foreground program of the pane's terminal; when nothing else runs there, the shell.
            let foreground = (shell.tpgid > 0 && shell.tpgid != shell.pgid && shell.tty != "?")
                .then(|| {
                    all.iter()
                        .find(|p| p.tty == shell.tty && p.pid == shell.tpgid)
                        .map(|p| program_name(&p.command))
                })
                .flatten();
            let running = foreground
                .or_else(|| {
                    Some(
                        program_name(&shell.command)
                            .trim_start_matches('-')
                            .to_string(),
                    )
                })
                .filter(|n| !n.is_empty());
            OldPane {
                pid: shell.pid,
                cwd: cwd_of(shell.pid),
                running,
            }
        })
        .collect()
}

/// What will be made again: for each session its name and, when there is exactly one session, the
/// directory of each of its windows (taken from the panes in the order they were made). Also the
/// directories that are left over.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub sessions: Vec<(String, Vec<Option<String>>)>,
    pub left_over: Vec<String>,
}

pub fn plan(sessions: &[OldSession], panes: &[OldPane]) -> Plan {
    let dirs: Vec<String> = panes
        .iter()
        .filter_map(|p| p.cwd.clone())
        .filter(|d| std::path::Path::new(d).is_dir())
        .collect();
    if let [only] = sessions {
        let used = only.windows.min(dirs.len());
        let mut windows: Vec<Option<String>> = dirs.iter().take(used).cloned().map(Some).collect();
        windows.resize(only.windows.max(1), None);
        return Plan {
            sessions: vec![(only.name.clone(), windows)],
            left_over: dirs.into_iter().skip(used).collect(),
        };
    }
    Plan {
        sessions: sessions
            .iter()
            .map(|s| (s.name.clone(), vec![None; s.windows.max(1)]))
            .collect(),
        left_over: dirs,
    }
}

fn show(sessions: &[OldSession], panes: &[OldPane], plan: &Plan) -> String {
    let mut text = String::from(
        "The running daemon is older than `arcade upgrade`, so it cannot hand its sessions over. A restart can keep this:\n",
    );
    for s in sessions {
        text += &format!(
            "  session {}: {} window(s), {} pane(s)\n",
            s.name, s.windows, s.panes
        );
    }
    text +=
        "\nThe daemon's panes, as the system shows them (these programs END, they are not kept):\n";
    for (i, p) in panes.iter().enumerate() {
        text += &format!(
            "  pane {} (process {}): {}{}\n",
            i + 1,
            p.pid,
            p.cwd.as_deref().unwrap_or("directory unknown"),
            p.running
                .as_deref()
                .map_or(String::new(), |r| format!(", program {r}")),
        );
    }
    text += "\nAfter the restart: each session again by name with the same number of windows, each with a shell";
    if sessions.len() == 1 {
        text += ", in the directory of the pane of the same position where it is known.\n";
    } else {
        text += " in your current directory (with several sessions the system cannot say which pane belongs to which).\n";
    }
    text += "Not kept: the programs running in the panes, the history and screen text, the split layout and sizes, window names, which pane had the focus.\n";
    if !plan.left_over.is_empty() {
        text += "Directories not used (go there yourself):\n";
        for d in &plan.left_over {
            text += &format!("  {d}\n");
        }
    }
    text
}

fn wait_for_result(connection: &mut Connection) -> Result<(), String> {
    loop {
        match connection.receive()? {
            Some(ServerMessage::CommandResult { ok: true, .. }) => return Ok(()),
            Some(ServerMessage::CommandResult { ok: false, output }) => return Err(output),
            Some(_) => {}
            None => return Err("the daemon closed the connection".to_string()),
        }
    }
}

/// Make one session, with its first window in `dir` and one more window for each further entry.
fn make_session(
    program: &std::path::Path,
    socket: &std::path::Path,
    name: &str,
    windows: &[Option<String>],
) -> Result<(), String> {
    let stream = match connect(socket) {
        Ok(s) => s,
        Err(_) => {
            crate::connection::start_daemon_as(program, socket)?;
            connect(socket).map_err(|_| "cannot connect to the daemon just started".to_string())?
        }
    };
    let Handshake::Ok(mut connection) = handshake(stream)? else {
        return Err("the new daemon speaks another protocol".to_string());
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let first = windows
        .first()
        .cloned()
        .flatten()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or(home);
    connection.send(&ClientMessage::Attach(AttachRequest {
        session: None,
        create: CreateMode::Always,
        new_name: Some(name.to_string()),
        command: Vec::new(),
        cwd: first,
        env: std::env::vars().map(|(k, v)| format!("{k}={v}")).collect(),
        cols: 80,
        rows: 24,
    }))?;
    loop {
        match connection.receive()? {
            Some(ServerMessage::Attached { .. }) => break,
            Some(ServerMessage::CommandResult { ok: false, output }) => return Err(output),
            Some(_) => {}
            None => return Err("the daemon closed the connection".to_string()),
        }
    }
    for dir in windows.iter().skip(1) {
        // The shell of the new window starts in its own directory (and the window is named after
        // it); a window whose old directory is not known starts where the last one is.
        connection.send(&ClientMessage::Command {
            name: "window.new".to_string(),
            args: dir.iter().cloned().collect(),
        })?;
        wait_for_result(&mut connection)?;
    }
    connection.send(&ClientMessage::Detach)?;
    Ok(())
}

/// Offer the restart (or do it, with `--restart`). Returns the exit code.
pub fn offer(
    program: &std::path::Path,
    socket: &std::path::Path,
    list_output: &str,
    restart: bool,
    announce: &dyn Fn(),
) -> u8 {
    let sessions = parse_sessions(list_output);
    let panes = old_panes(socket);
    let plan = plan(&sessions, &panes);
    print!("{}", show(&sessions, &panes, &plan));
    if std::env::var_os("ARCADE_PANE").is_some() {
        eprintln!(
            "arcade: this command is running inside an Arcade pane, which the restart would end; run it in a plain terminal window"
        );
        return 1;
    }
    if !restart {
        if !std::io::stdin().is_terminal() {
            eprintln!("arcade: nothing was changed; add --restart to do it without being asked");
            return 1;
        }
        print!("\nRestart now? Type yes to go on: ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if answer.trim() != "yes" {
            println!("Nothing was changed.");
            return 1;
        }
    }
    // The restart is going ahead: now it is said which program it is to.
    announce();
    // Stop the old daemon (its panes get the hangup signal, then are killed).
    if let Ok(stream) = connect(socket)
        && let Ok(
            Handshake::Ok(mut c)
            | Handshake::Mismatch {
                connection: mut c, ..
            },
        ) = handshake(stream)
    {
        let _ = c.send(&ClientMessage::Command {
            name: "server.kill".to_string(),
            args: Vec::new(),
        });
        let _ = wait_for_result(&mut c);
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while connect(socket).is_ok() {
        if Instant::now() >= deadline {
            eprintln!("arcade: the old daemon did not stop");
            return 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for (name, windows) in &plan.sessions {
        if let Err(e) = make_session(program, socket, name, windows) {
            eprintln!("arcade: could not make session {name} again: {e}");
            return 1;
        }
        println!(
            "session {name}: made again with {} window(s)",
            windows.len()
        );
    }
    println!("Restarted with {}.", env!("CARGO_PKG_VERSION"));
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_list_of_an_old_daemon_is_read() {
        let got = parse_sessions("work\t1\t1\t3\t5\nlog\t2\t0\t1\t1\n");
        assert_eq!(
            got,
            vec![
                OldSession {
                    name: "work".into(),
                    windows: 3,
                    panes: 5
                },
                OldSession {
                    name: "log".into(),
                    windows: 1,
                    panes: 1
                }
            ]
        );
    }

    #[test]
    fn directories_are_used_only_with_one_session_and_the_rest_are_listed() {
        let pane = |cwd: &str| OldPane {
            pid: 1,
            cwd: Some(cwd.to_string()),
            running: None,
        };
        let one = [OldSession {
            name: "a".into(),
            windows: 2,
            panes: 3,
        }];
        let panes = [pane("/tmp"), pane("/"), pane("/usr")];
        let p = plan(&one, &panes);
        assert_eq!(
            p.sessions,
            vec![("a".to_string(), vec![Some("/tmp".into()), Some("/".into())])]
        );
        assert_eq!(p.left_over, vec!["/usr".to_string()]);
        let two = [
            OldSession {
                name: "a".into(),
                windows: 1,
                panes: 1,
            },
            OldSession {
                name: "b".into(),
                windows: 2,
                panes: 2,
            },
        ];
        let p = plan(&two, &panes);
        assert_eq!(p.sessions[1], ("b".to_string(), vec![None, None]));
        assert_eq!(p.left_over.len(), 3);
    }
}
