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

After an upgrade, run `arcade kill-server` once so that the old background process makes way for
the new version. This ends the running sessions.

## First run

```sh
arcade            # attach to the session used last, or create one
arcade new -s work
arcade ls
```

The default prefix key, the key tables and every setting are described in the
[user guide](docs/user-guide.md).

## Build from source

You need a Rust toolchain (the version is pinned in `rust-toolchain.toml`), a C compiler and
`curl`. The terminal emulation library is written in Zig and needs exactly one Zig version, which
the script below downloads into `.tools/zig` inside the repository:

```sh
./scripts/fetch-zig.sh
cargo build --release -p arcade
./target/release/arcade --version
```

To check your changes: `cargo fmt --all --check`, `cargo lint` and `cargo test --workspace`.

## License

MIT, see [LICENSE](LICENSE).
