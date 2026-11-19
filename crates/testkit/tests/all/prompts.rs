//! The shell snippets of the user guide (section 5.15) give the prompt marks that copy mode reads:
//! each shell runs in a pane, a command is typed, and `O` selects exactly its output.

use std::path::PathBuf;
use std::process::Command;

use arcade_testkit::{AttachOptions, Screened, TestClient, TestDaemon, WAIT};

const PREFIX: &[u8] = b"\x1b[109;5u";

/// The text of the first code block of the user guide written in `language`.
fn snippet(language: &str) -> String {
    let guide = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/user-guide.md"),
    )
    .expect("the user guide");
    let open = format!("```{language}\n");
    let start = guide.find(&open).expect("a block for the shell") + open.len();
    let end = guide[start..].find("```").expect("end of the block");
    guide[start..start + end].to_string()
}

fn have(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run the shell in a pane, type two commands, then copy the output of the last with `O`.
fn output_of_last_command(argv: &[&str]) -> Vec<u8> {
    let daemon = TestDaemon::start();
    let mut client: TestClient = daemon
        .attach(AttachOptions::new().size(80, 20).named("sh").command(argv))
        .unwrap();
    client.wait_for_text("READY").unwrap();
    client.type_line("echo first-out");
    client.wait_for_row("first-out").unwrap();
    client.type_line("printf 'second-a\\nsecond-b\\n'");
    client.wait_for_row("second-b").unwrap();
    client.wait_until(WAIT, |c| {
        c.content_rows()
            .iter()
            .rev()
            .find(|r| !r.is_empty())
            .is_some_and(|r| r.starts_with("READY"))
    });
    client.send(PREFIX);
    client.send(b"[");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("COPY")));
    // On the waiting prompt: the output of the command before it.
    client.send(b"O");
    assert!(client.wait_until(WAIT, |c| c.status_row().contains("V-LINE")));
    client.send(b"y");
    assert!(client.wait_until(WAIT, |c| !c.clipboard_writes.is_empty()));
    client.clipboard_writes[0].clone()
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn scratch(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "arc-prompts-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    file
}

#[test]
fn the_zsh_snippet_gives_the_marks_arcade_needs() {
    if have("zsh", &["--version"]).is_none() {
        eprintln!("no zsh here: skipped");
        return;
    }
    let rc = scratch(".zshrc", &format!("PS1='READY> '\n{}", snippet("zsh")));
    let dir = rc.parent().unwrap().to_string_lossy().into_owned();
    let script = format!("ZDOTDIR='{dir}' exec zsh -i");
    let got = output_of_last_command(&["sh", "-c", &script]);
    assert_eq!(got, b"second-a\nsecond-b".to_vec());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_zsh_snippet_works_after_starship_builds_the_prompt() {
    let starship = have("sh", &["-c", "command -v starship"]).map(|p| p.trim().to_string());
    let (true, Some(starship)) = (have("zsh", &["--version"]).is_some(), starship) else {
        eprintln!("no zsh or no starship here: skipped");
        return;
    };
    let config = scratch(
        "starship.toml",
        "format = \"READY> \"\nadd_newline = false\n",
    );
    let bin = std::path::Path::new(&starship).parent().unwrap().display();
    let rc = scratch(
        ".zshrc",
        &format!(
            "export PATH=\"{bin}:$PATH\" STARSHIP_CONFIG='{}'\neval \"$(starship init zsh)\"\n{}",
            config.display(),
            snippet("zsh")
        ),
    );
    let dir = rc.parent().unwrap().to_string_lossy().into_owned();
    let script = format!("ZDOTDIR='{dir}' exec zsh -i");
    let got = output_of_last_command(&["sh", "-c", &script]);
    assert_eq!(got, b"second-a\nsecond-b".to_vec());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_bash_snippet_gives_the_marks_arcade_needs() {
    // The snippet needs PS0, which is in bash 4.4 and newer.
    let version = have(
        "bash",
        &["-c", "echo ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"],
    )
    .unwrap_or_default();
    let (major, minor) = version.trim().split_once('.').unwrap_or(("0", "0"));
    let (major, minor): (u32, u32) = (major.parse().unwrap_or(0), minor.parse().unwrap_or(0));
    if (major, minor) < (4, 4) {
        eprintln!("bash {version}: no PS0, skipped");
        return;
    }
    let rc = scratch("bashrc", &format!("PS1='READY> '\n{}", snippet("bash")));
    let script = format!("exec bash --noprofile --rcfile '{}' -i", rc.display());
    let got = output_of_last_command(&["sh", "-c", &script]);
    assert_eq!(got, b"second-a\nsecond-b".to_vec());
    let _ = std::fs::remove_dir_all(rc.parent().unwrap());
}

#[test]
fn the_snippets_are_in_the_guide() {
    assert!(snippet("zsh").contains("133;C"));
    assert!(snippet("bash").contains("PS0"));
}
