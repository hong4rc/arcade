# Arcade

Arcade is a terminal multiplexer, in the spirit of tmux: sessions that keep running when you
close the terminal, windows, split panes, a status bar, copy mode, and notifications that tell
you when a program in a pane needs you.

## Install

On macOS (Apple silicon) and Linux:

```sh
brew tap hong4rc/tap
brew trust --tap hong4rc/tap
brew install hong4rc/tap/arcade
```

The `brew trust` line is needed on recent Homebrew versions, which refuse to load a formula from
a tap outside the official ones until you trust the tap.

After an upgrade, run `arcade upgrade`: the background process continues as the new version and
keeps your sessions and the programs in them. A daemon from before that command existed still needs
`arcade upgrade --restart` once, which restarts it keeping the session names, window counts and directories (the programs in the panes end).

## First run

```sh
arcade            # attach to the session used last, or create one
arcade new -s work
arcade ls
```

New here? The [user guide](docs/user-guide.md) starts with a ten-minute tour that tries each
feature in a throwaway copy; after it come the key tables and every setting.

## Build from source

You need a Rust toolchain (the version is pinned in `rust-toolchain.toml`), a C compiler and
`curl`. The terminal emulation library is written in Zig and needs exactly one Zig version, which
the script below downloads into `.tools/zig` inside the repository:

```sh
./scripts/fetch-zig.sh
cargo build --release -p arcade
./target/release/arcade --version
```

To build and install the binary in one step, run `./scripts/install-local.sh`. It puts `arcade` in
`~/.local/bin` (set `ARCADE_INSTALL_DIR` to choose another folder) and does not touch a daemon that
is already running; run `arcade upgrade` when you want the new code in it (it keeps the sessions;
`arcade kill-server` is needed only for a daemon older than that command). To go back, delete that
file.

To check your changes: `cargo fmt --all --check`, `cargo lint` and `cargo test --workspace`.

## License

MIT, see [LICENSE](LICENSE).
