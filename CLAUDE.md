# Arcade

Arcade is a tmux-style terminal multiplexer written in Rust. The spec lives in `docs/`, starting from `docs/README.md`. Technical decisions with their measurements are in `docs/decisions/`, and measurement results are in `docs/benchmarks/`. These working files are kept on disk but are not tracked by git (see "Constraints").

## The user and how things are checked

- The author is the only user. They neither read nor write Rust. All code is written by AI, and the author does not review code by hand.
- What tells whether code is right or wrong is automated tests and measurements. A piece of work with no test or measurement attached counts as not done.
- When reporting, speak in plain words: what now works, what was measured and how much, what is uncertain. Do not assume the reader knows Rust jargon.

## Commands

Run from the repository root. The first time on a new machine, run `./scripts/fetch-zig.sh` first (see "Build tools").

| Task | Command |
|---|---|
| Tests | `cargo test --workspace` |
| Format | `cargo fmt --all` (check without changing: `cargo fmt --all --check`) |
| Lint | `cargo lint` |
| Compare the parsers (decision 001) | `cargo parser-eval all` |
| Benchmark (direct, tmux, Arcade) | `cargo bench-run` (add `--quick` to run fast; `--subjects` and `--tests` run a part) |
| Tests on Linux in a container | `./scripts/test-linux.sh` (needs Docker running) |

A change counts as done only when the test, format and lint commands all run clean.

## Build tools

- The Rust version is pinned in `rust-toolchain.toml`.
- The terminal emulation library libghostty-vt is written in Zig and needs exactly Zig 0.17. `./scripts/fetch-zig.sh` downloads Zig into `.tools/zig` inside the repository (nothing is installed on the machine); `.cargo/config.toml` points the `ZIG` variable there.
- `.cargo/config.toml` sets `LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseFast`. Do not remove this line: without it, the Zig part is compiled in debug mode and becomes thousands of times slower (measured, see decision 001).

## Constraints

- **The spec is the source of truth, and it is not changed silently.** When the spec is wrong, missing or contradicts itself, write it in a file under `docs/decisions/`, state the assumption used to go on, and tell the author in the final report. Stop and ask only when the spot blocks the work completely.
- `docs/spec-core.md` was rebuilt because sections 1 to 6 of the original spec are not in the repository. If the author adds the original spec, the original wins.
- **Arcade has no support specific to any coding agent.** Do not add anything that recognises Claude Code or another agent.
- **Add no feature outside the "In Phase 1" column** of `docs/spec-phase-1.md`.
- **Every benchmark runs on an optimised build** (`--release`), never on a debug build.
- **Never touch the author's default tmux server.** The author works inside it. Do not stop it, restart it, or change its configuration. Every measurement or test that uses tmux must run on a server of its own with its own socket (`tmux -L <own-name> -f /dev/null`) and clean that server up when done.
- **Every process the session starts** (a private tmux server, a program run in the background, a trial daemon) must be stopped before the session ends.
- **Commit in logical steps**, using conventional commits, with no mention of AI in the commit message. Push nowhere.
- **What git tracks.** Only the code, the configuration, the scripts, this `CLAUDE.md`, and `docs/user-guide.md` are committed. The spec, the decisions, the measurement results, the prompts, and every temporary file are working files that stay on disk and are never committed.
- **Commit author and hashes.** The commit author is `hong4rc` (the repository's git configuration already says so; do not pass `--author` or change it). After every commit, run `git vanity -q -p <preset>` so the hash of that commit starts with a word (prefer `coffee`; other quick presets are `cafe`, `code`, `food`, `beef`, `feed`, `face`, `fade`). It rewrites exactly the HEAD commit, so run it before the next commit and only when no other git command is running. When a commit hash is written in a document or a report, take it after the vanity step.

## Language

- Code, identifiers, comments in code and commit messages are written in English.
- Documents that are committed (this file and `docs/user-guide.md`) are written in English. Working documents under `docs/` and reports to the author are written in Vietnamese.

## Repository layout

- `docs/`: the spec, decisions and measurement results (working files, not tracked), and `user-guide.md` (tracked).
- `prompts/`: the prompts that hand out work for each session, numbered in order (working files, not tracked).
- `crates/workload/`: the standard test-data generator (the six kinds of load of the performance spec), shared by every measurement.
- `crates/parser-eval/`: the program that compares the parsers, and the right-and-wrong test cases. These cases later become tests of the terminal emulation part in the daemon.
- `crates/term/`, `crates/proto/`, `crates/daemon/`, `crates/arcade/`: terminal emulation (the only place that calls libghostty-vt), the protocol, the daemon, the `arcade` command.
- In `crates/daemon/src/`: `server.rs` and `server/` (the loop, session, window, pane, commands, frame building), `layout.rs` (the split tree), `compose.rs` and `render.rs` (composing and sending frames), `keys.rs` (reading keys from the host terminal), `keymap.rs` (the four key tables), `commands.rs` (the list of named commands), `config.rs` (the configuration file), `status.rs`, `picker.rs`, `tree.rs`, `copy.rs`, `clipboard.rs`, `theme.rs` (the only place allowed to write colours and symbols; a test scans the other files).
- `crates/testkit/`: a fake client, a fake terminal, and the tests that run a real daemon.
- `crates/bench/`: the benchmark suite (decision 003). It uses its own socket and its own tmux server, cleaned up when done.
- `scripts/`: downloading Zig, running the tests on Linux.
- `docker/`: the Linux environment for `scripts/test-linux.sh`.
- `bench-results/`: machine-readable measurement results, kept out of git.
