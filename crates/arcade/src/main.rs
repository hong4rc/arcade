//! The `arcade` command.

mod client;
mod connection;
mod host;
mod keytool;
mod notify;
mod wait;

use std::path::PathBuf;
use std::process::ExitCode;

use arcade_proto::{AttachRequest, ClientMessage, CreateMode, ServerMessage};
use connection::{ConnectError, Handshake, connect, handshake, mismatch_message, start_daemon};

const USAGE: &str = "\
usage: arcade [--socket <path>] [command]

commands:
  (none)                          attach to the session used last, or create one
  new [-d] [-s <name>] [<command>...]
                                  create a session and attach to it (-d: do not attach)
  attach [-t <session>]           attach to a session
                                  (inside a pane: switch this screen to it)
  ls                              list sessions
  kill-session [-t <session>]     close a session
  kill-server                     stop the daemon and every session
  run <command> [<argument>...]   run a command, for scripts (for example session.detach)
  config check [--host kitty|legacy] [<file>]
                                  check the configuration file: list its mistakes and the
                                  bindings that cannot work
  keys list [--host kitty|legacy] [<file>]
                                  print every key binding in force, with its source
  keys test [<file>]              show the name of each key Arcade receives (Ctrl+C quits)
  notify [--class <info|input|approval|error>] [--title <text>] [--message <text>]
         [--target <pane>] [--source <name>] [--strict]
                                  tell Arcade that a pane needs attention (for hooks: it
                                  stays quiet and exits 0 unless --strict is given)
  notify clear [--target <pane> | --all] [--strict]
                                  take a notification away
  notify check [--target <pane>]  show which pane and socket would be used, and send a test
  notify log                      show what became of the latest signals
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
    if let Err(message) = arcade_daemon::check_socket_path_length(&options.socket) {
        eprintln!("arcade: {message}");
        return 1;
    }
    let Some(command) = args.first().cloned() else {
        return attach_command(&options, None, CreateMode::IfNone, None, Vec::new());
    };
    let rest = &args[1..];
    match command.as_str() {
        "__daemon" => daemon_command(&options),
        "new" => {
            let mut name = None;
            let mut detached = false;
            let mut index = 0;
            while index < rest.len() {
                match rest[index].as_str() {
                    "-d" => {
                        detached = true;
                        index += 1;
                    }
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
            let command = rest[index..].to_vec();
            if detached || in_own_pane(&options) {
                let created = match create_detached(&options, name, command) {
                    Ok(created) => created,
                    Err(code) => return code,
                };
                if detached {
                    return 0;
                }
                // Inside a pane of this daemon: the screen goes to the new session.
                return switch_from_pane(&options, &created);
            }
            attach_command(&options, None, CreateMode::Always, name, command)
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
            if in_own_pane(&options) {
                let Some(target) = target else {
                    return usage_error("inside a pane, attach needs -t <session>");
                };
                return switch_from_pane(&options, &target);
            }
            attach_command(&options, target, CreateMode::Never, None, Vec::new())
        }
        "ls" => {
            if !rest.is_empty() {
                return usage_error("ls takes no arguments");
            }
            list_command(&options)
        }
        "config" => match rest.split_first() {
            Some((word, args)) if word == "check" => {
                let mut args = args.to_vec();
                let host = match keytool::take_host_flag(&mut args) {
                    Ok(host) => host,
                    Err(e) => return usage_error(&e),
                };
                match args.as_slice() {
                    [] => config_check(None, host),
                    [file] => config_check(Some(PathBuf::from(file)), host),
                    _ => usage_error("usage: config check [--host kitty|legacy] [<file>]"),
                }
            }
            _ => usage_error("usage: config check [--host kitty|legacy] [<file>]"),
        },
        "keys" => keytool::run(rest, default_config_path()),
        "notify" => notify::run(&options.socket, rest),
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

/// Where the configuration file is: `ARCADE_CONFIG`, or `~/.config/arcade/config.toml`.
fn default_config_path() -> Option<PathBuf> {
    arcade_daemon::config::default_path(
        std::env::var("ARCADE_CONFIG").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// `arcade config check`: read the file and say what is wrong with it, with the lines.
fn config_check(file: Option<PathBuf>, host: Option<arcade_daemon::keytools::Host>) -> u8 {
    let Some(path) = file.or_else(default_config_path) else {
        eprintln!("arcade: no configuration file path (set HOME or ARCADE_CONFIG)");
        return 1;
    };
    if !path.exists() {
        println!("no file at {}: the defaults are used", path.display());
        keytool::print_warnings(&arcade_daemon::config::Config::default(), host);
        return 0;
    }
    match arcade_daemon::config::load(&path) {
        Ok(config) => {
            println!("ok: {}", path.display());
            keytool::print_warnings(&config, host);
            0
        }
        Err(errors) => {
            for e in errors {
                eprintln!("{}: {e}", path.display());
            }
            1
        }
    }
}

fn daemon_command(options: &Options) -> u8 {
    let mut config = arcade_daemon::DaemonConfig::new(&options.socket);
    config.log_path = Some(options.socket.with_extension("log"));
    config.config_path = default_config_path();
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

/// Running in a pane of the very daemon this command talks to: attaching here would draw an
/// Arcade inside Arcade.
fn in_own_pane(options: &Options) -> bool {
    std::env::var_os("ARCADE_PANE").is_some()
        && std::env::var_os("ARCADE").is_some_and(|v| v == options.socket.as_os_str())
}

/// Move the screen that shows this pane to another session.
fn switch_from_pane(options: &Options, session: &str) -> u8 {
    let pane = std::env::var("ARCADE_PANE")
        .ok()
        .and_then(|p| p.parse::<u64>().ok());
    simple_command(
        options,
        "session.switch",
        &[session.to_string()],
        pane,
        false,
    )
}

/// Create a session without showing it: attach, wait for the answer, and let go. Needs no
/// terminal. Returns the name of the new session.
fn create_detached(
    options: &Options,
    new_name: Option<String>,
    command: Vec<String>,
) -> Result<String, u8> {
    let mut connection = match open_connection(options, true) {
        Ok(Handshake::Ok(connection)) => connection,
        Ok(Handshake::Mismatch { daemon, client, .. }) => {
            eprintln!("{}", mismatch_message(daemon, client));
            return Err(1);
        }
        Err(code) => return Err(code),
    };
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let env = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    let request = AttachRequest {
        session: None,
        create: CreateMode::Always,
        new_name,
        command,
        cwd,
        env,
        cols: 80,
        rows: 24,
    };
    if let Err(e) = connection.send(&ClientMessage::Attach(request)) {
        eprintln!("arcade: {e}");
        return Err(1);
    }
    loop {
        match connection.receive() {
            Ok(Some(ServerMessage::Attached { session })) => return Ok(session),
            Ok(Some(ServerMessage::CommandResult { ok: false, output })) => {
                eprintln!("arcade: {output}");
                return Err(1);
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!("arcade: the server closed the connection");
                return Err(1);
            }
            Err(e) => {
                eprintln!("arcade: {e}");
                return Err(1);
            }
        }
    }
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
