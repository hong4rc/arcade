# Linux build and test environment for Arcade. Run it with scripts/test-linux.sh.
# The source is mounted when the container runs, not copied in, and Zig goes to /opt/tools so the
# Zig compiler for macOS in the mounted .tools directory is left alone.
FROM rust:1.98-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl xz-utils neovim procps \
 && rm -rf /var/lib/apt/lists/*

# The toolchain version and components come from rust-toolchain.toml; install them up front so a
# test run does not need the network for them.
RUN rustup toolchain install 1.98.0 --profile minimal --component rustfmt,clippy

COPY scripts/fetch-zig.sh /opt/fetch-zig/scripts/fetch-zig.sh
RUN ARCADE_TOOLS_DIR=/opt/tools sh /opt/fetch-zig/scripts/fetch-zig.sh

# Cargo's environment settings in .cargo/config.toml point ZIG at the mounted .tools directory;
# a variable that is already set wins.
ENV ZIG=/opt/tools/zig/zig \
    CARGO_TARGET_DIR=/cache/target \
    TERM=xterm-256color

WORKDIR /arcade
