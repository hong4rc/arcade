//! The `arcade` command.

mod client;
mod connection;
mod host;

use std::path::PathBuf;
use std::process::ExitCode;

use arcade_proto::{AttachRequest, ClientMessage, CreateMode, ServerMessage};
use connection::{ConnectError, Handshake, connect, handshake, mismatch_message, start_daemon};

const USAGE: &str = "\
usage: arcade [--socket <path>] [command]

commands:
  (none)                          attach to the session used last, or create one
  new [-s <name>] [<command>...]  create a session and attach to it
  attach [-t <session>]           attach to a session
  ls                              list sessions
  kill-session [-t <session>]     close a session
  kill-server                     stop the daemon and every session
  run <command> [<argument>...]   run a command, for scripts (for example session.detach)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(args))
}

struct Options {
    socket: PathBuf,
}

fn usage_error(message: &str) -> u8 {
    eprintln!("arcade: {message}\n\n{USAGE}");
    2
}

fn run(mut args: Vec<String>) -> u8 {
    let mut socket_arg: Option<PathBuf> = None;
    if args.first().map(String::as_str) == Some("--socket") {
        if args.len() < 2 {
            return usage_error("--socket needs a path");
        }
        socket_arg = Some(PathBuf::from(args.remove(1)));
        args.remove(0);
    }
    let options = Options {
        socket: arcade_daemon::resolve_socket_path(socket_arg.as_deref()),
    };
    let Some(command) = args.first().cloned() else {
        return attach_command(&options, None, CreateMode::IfNone, None, Vec::new());
    };
    let rest = &args[1..];
    match command.as_str() {
        "__daemon" => daemon_command(&options),
        "new" => {
            let mut name = None;
            let mut index = 0;
            while index < rest.len() {
                match rest[index].as_str() {
                    "-s" => {
                        let Some(value) = rest.get(index + 1) else {
                            return usage_error("-s needs a session name");
                        };
                        name = Some(value.clone());
                        index += 2;
                    }
                    "--" => {
                        index += 1;
                        break;
                    }
                    other if other.starts_with('-') => {
                        return usage_error(&format!("unknown option {other}"));
                    }
                    _ => break,
                }
            }
            attach_command(
                &options,
                None,
                CreateMode::Always,
                name,
                rest[index..].to_vec(),
            )
        }
        "attach" => {
            let mut target = None;
            let mut index = 0;
            while index < rest.len() {
                match rest[index].as_str() {
                    "-t" => {
                        let Some(value) = rest.get(index + 1) else {
                            return usage_error("-t needs a session name");
                        };
                        target = Some(value.clone());
                        index += 2;
                    }
                    other => return usage_error(&format!("unexpected argument {other}")),
                }
            }
            attach_command(&options, target, CreateMode::Never, None, Vec::new())
        }
        "ls" => {
            if !rest.is_empty() {
                return usage_error("ls takes no arguments");
            }
            list_command(&options)
        }
        "kill-server" => simple_command(&options, "server.kill", &[], None, true),
        "kill-session" => {
            let mut args_out = Vec::new();
            match rest {
                [] => {}
                [flag, name] if flag == "-t" => args_out.push(name.clone()),
                _ => return usage_error("usage: kill-session [-t <session>]"),
            }
            simple_command(&options, "session.close", &args_out, None, false)
        }
        "run" => {
            let Some((name, extra)) = rest.split_first() else {
                return usage_error("run needs a command name");
            };
            // Run from a pane, the command acts on that pane's session and window.
            let pane = std::env::var("ARCADE_PANE")
                .ok()
                .and_then(|p| p.parse::<u64>().ok());
            simple_command(&options, name, extra, pane, false)
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            0
        }
        other => usage_error(&format!("unknown command {other}")),
    }
}

fn no_server(options: &Options) -> u8 {
    eprintln!("arcade: no server running on {}", options.socket.display());
    1
}

fn daemon_command(options: &Options) -> u8 {
    let mut config = arcade_daemon::DaemonConfig::new(&options.socket);
    config.log_path = Some(options.socket.with_extension("log"));
    match arcade_daemon::run(config) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("arcade: daemon: {e}");
            1
        }
    }
}

/// Connect, starting the daemon first when asked to and none is running.
fn open_connection(options: &Options, autostart: bool) -> Result<Handshake, u8> {
    let stream = match connect(&options.socket) {
        Ok(stream) => stream,
        Err(ConnectError::NoServer) if autostart => {
            if let Err(e) = arcade_daemon::prepare_socket_dir(&options.socket) {
                eprintln!("arcade: {e}");
                return Err(1);
            }
            if let Err(e) = start_daemon(&options.socket) {
                eprintln!("arcade: {e}");
                return Err(1);
            }
            match connect(&options.socket) {
                Ok(stream) => stream,
                Err(_) => {
                    eprintln!("arcade: cannot connect to the daemon just started");
                    return Err(1);
                }
            }
        }
        Err(ConnectError::NoServer) => return Err(no_server(options)),
        Err(ConnectError::Other(message)) => {
            eprintln!("arcade: {message}");
            return Err(1);
        }
    };
    handshake(stream).map_err(|e| {
        eprintln!("arcade: {e}");
        1
    })
}

fn attach_command(
    options: &Options,
    session: Option<String>,
    create: CreateMode,
    new_name: Option<String>,
    command: Vec<String>,
) -> u8 {
    if !host::is_tty(0) || !host::is_tty(1) {
        eprintln!("arcade: the keyboard and screen must be a terminal");
        return 1;
    }
    // Only a command that may create a session starts a daemon that is not running.
    let connection = match open_connection(options, create != CreateMode::Never) {
        Ok(Handshake::Ok(connection)) => connection,
        Ok(Handshake::Mismatch { daemon, client, .. }) => {
            eprintln!("{}", mismatch_message(daemon, client));
            return 1;
        }
        Err(code) => return code,
    };
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let env = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    let request = AttachRequest {
        session,
        create,
        new_name,
        command,
        cwd,
        env,
        cols: 0,
        rows: 0,
    };
    client::attach(connection, request) as u8
}

fn command_result(
    connection: &mut connection::Connection,
    name: &str,
    args: &[String],
    pane: Option<u64>,
) -> Result<(bool, String), String> {
    let message = match pane {
        Some(pane) => ClientMessage::PaneCommand {
            pane,
            name: name.to_string(),
            args: args.to_vec(),
        },
        None => ClientMessage::Command {
            name: name.to_string(),
            args: args.to_vec(),
        },
    };
    connection.send(&message)?;
    loop {
        match connection.receive()? {
            Some(ServerMessage::CommandResult { ok, output }) => return Ok((ok, output)),
            Some(_) => {}
            None => return Err("the server closed the connection".to_string()),
        }
    }
}

/// Run one named command and print its answer. `allow_mismatch` lets a command through to a
/// daemon of another version (only `server.kill` is accepted there).
fn simple_command(
    options: &Options,
    name: &str,
    args: &[String],
    pane: Option<u64>,
    allow_mismatch: bool,
) -> u8 {
    let mut connection = match open_connection(options, false) {
        Ok(Handshake::Ok(connection)) => connection,
        Ok(Handshake::Mismatch {
            daemon,
            client,
            connection,
        }) => {
            if !allow_mismatch {
                eprintln!("{}", mismatch_message(daemon, client));
                return 1;
            }
            connection
        }
        Err(code) => return code,
    };
    match command_result(&mut connection, name, args, pane) {
        Ok((true, output)) => {
            print!("{output}");
            0
        }
        Ok((false, output)) => {
            eprintln!("arcade: {}", output.trim_end());
            1
        }
        // The daemon may close the connection as it stops.
        Err(_) if name == "server.kill" => 0,
        Err(e) => {
            eprintln!("arcade: {e}");
            1
        }
    }
}

fn list_command(options: &Options) -> u8 {
    let mut connection = match open_connection(options, false) {
        Ok(Handshake::Ok(connection)) => connection,
        Ok(Handshake::Mismatch { daemon, client, .. }) => {
            eprintln!("{}", mismatch_message(daemon, client));
            return 1;
        }
        Err(code) => return code,
    };
    match command_result(&mut connection, "session.list", &[], None) {
        Ok((true, output)) => {
            for line in output.lines() {
                let mut fields = line.split('\t');
                let name = fields.next().unwrap_or("");
                let mut number = |skip: usize| {
                    fields
                        .nth(skip)
                        .and_then(|f| f.parse::<usize>().ok())
                        .unwrap_or(0)
                };
                // Fields after the name: id, attached, windows, panes.
                let attached = number(1) == 1;
                let windows = number(0);
                let panes = number(0);
                println!(
                    "{name}: {}, {}{}",
                    count(windows, "window"),
                    count(panes, "pane"),
                    if attached { " (attached)" } else { "" }
                );
            }
            0
        }
        Ok((false, output)) => {
            eprintln!("arcade: {}", output.trim_end());
            1
        }
        Err(e) => {
            eprintln!("arcade: {e}");
            1
        }
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
