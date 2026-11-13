#!/usr/bin/env python3
"""Prototype of the Arcade benchmark suite, used to measure tmux as the baseline.

It verifies the measuring method of docs/decisions/003-benchmark-design.md before the suite is
written in Rust. Every tmux command here runs on a private server (its own socket name, no
configuration file); the default tmux server is never touched. The private servers are killed
on exit.

Run:  python3 scripts/bench-prototype.py [--quick]
"""

import atexit
import fcntl
import json
import os
import pty
import re
import select
import signal
import statistics
import struct
import subprocess
import sys
import termios
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(ROOT, "bench-results")
DATA_DIR = os.path.join(OUT_DIR, "workloads")
SOCKET = f"arcade-bench-{os.getpid()}"
COLS, ROWS = 120, 40
QUICK = "--quick" in sys.argv
ENV = {k: v for k, v in os.environ.items() if k not in ("TMUX", "TMUX_PANE")}
ENV["TERM"] = "xterm-256color"
WORKLOADS = ["plain", "colored", "scroll", "fullscreen", "wide", "long-escapes"]
ANSI = re.compile(rb"\x1b\[[0-9;?<=>:]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-B]|\x1b[=>78MDEHc]|[\r\n\x0f\x0e]")
sockets = [SOCKET]


def now():
    return time.perf_counter_ns()


def tmux(*args, socket=None, check=True):
    cmd = ["tmux", "-L", socket or SOCKET, "-f", "/dev/null", *args]
    return subprocess.run(cmd, env=ENV, check=check, capture_output=True, text=True).stdout.strip()


def cleanup():
    """Stop every private server this run started and remove its socket file."""
    tmpdir = os.environ.get("TMUX_TMPDIR", "/tmp")
    for name in sockets:
        subprocess.run(["tmux", "-L", name, "kill-server"], env=ENV, capture_output=True)
        try:
            os.remove(os.path.join(tmpdir, f"tmux-{os.getuid()}", name))
        except OSError:
            pass


def stop_server():
    # The server may already be gone: it exits by itself when its last pane closes.
    tmux("kill-server", check=False)


atexit.register(cleanup)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(1))


def spawn_in_pty(argv):
    """Run a program in a new terminal of COLS x ROWS; return (pid, master descriptor)."""
    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        os.execvpe(argv[0], argv, ENV)
    return pid, fd


def kill(pid, fd):
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
    os.close(fd)


class Drain(threading.Thread):
    """Reads everything a terminal client writes, like a host terminal would, and counts it."""

    def __init__(self, fd):
        super().__init__(daemon=True)
        self.fd, self.bytes, self.reads, self.stop = fd, 0, 0, False

    def run(self):
        while not self.stop:
            ready, _, _ = select.select([self.fd], [], [], 0.2)
            if ready:
                try:
                    data = os.read(self.fd, 1 << 20)
                except OSError:
                    return
                if not data:
                    return
                self.bytes += len(data)
                self.reads += 1


def read_until(fd, marker, timeout=5.0, buffer=b""):
    """Read until `marker` shows up in the text with escape sequences removed."""
    deadline = time.monotonic() + timeout
    while True:
        if marker in ANSI.sub(b"", buffer):
            return now(), buffer
        left = deadline - time.monotonic()
        if left <= 0:
            return None, buffer
        ready, _, _ = select.select([fd], [], [], left)
        if ready:
            buffer += os.read(fd, 1 << 16)


def start_server(command="cat"):
    tmux("new-session", "-d", "-s", "bench", "-x", str(COLS), "-y", str(ROWS), command)
    tmux("set-option", "-g", "history-limit", "50000")
    return int(tmux("display-message", "-p", "#{pid}"))


def attach(session="bench"):
    return spawn_in_pty(["tmux", "-L", SOCKET, "-f", "/dev/null", "attach-session", "-t", session])


def ensure_workloads():
    os.makedirs(DATA_DIR, exist_ok=True)
    binary = os.path.join(ROOT, "target", "release", "arcade-workload")
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "arcade-workload"], cwd=ROOT, check=True)
    sizes = {}
    for name in WORKLOADS:
        megabytes = 32 if QUICK else (1000 if name == "plain" else 256)
        path = os.path.join(DATA_DIR, f"{name}-{megabytes}.bin")
        if not os.path.exists(path):
            subprocess.run([binary, name, str(megabytes), path], check=True)
        sizes[name] = (path, os.path.getsize(path))
    fill = os.path.join(DATA_DIR, "scrollback-fill-50000.bin")
    if not os.path.exists(fill):
        subprocess.run([binary, "scrollback-fill", "50000", fill], check=True)
    return sizes, fill


def percentile(values, p):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(p / 100 * (len(ordered) - 1))))]


def summary_us(values_ns):
    return {
        "p50_us": round(percentile(values_ns, 50) / 1000, 1),
        "p99_us": round(percentile(values_ns, 99) / 1000, 1),
        "max_us": round(max(values_ns) / 1000, 1),
        "samples": len(values_ns),
    }


# ---------------------------------------------------------------- throughput


def throughput_direct(path, size):
    """The same command in a bare terminal with nothing in between: the ceiling."""
    pid, fd = spawn_in_pty(["sh", "-c", f"stty -opost; cat '{path}'"])
    start, total = now(), 0
    while True:
        try:
            data = os.read(fd, 1 << 20)
        except OSError:
            break
        if not data:
            break
        total += len(data)
    seconds = (now() - start) / 1e9
    os.waitpid(pid, 0)
    os.close(fd)
    return {"seconds": round(seconds, 3), "mb_per_s": round(size / 1e6 / seconds, 1)}


def throughput_tmux(sizes):
    start_server()
    pid, fd = attach()
    drain = Drain(fd)
    drain.start()
    time.sleep(0.5)
    results = {}
    for index, (name, (path, size)) in enumerate(sizes.items()):
        before = drain.bytes
        signal_name = f"done-{index}"
        script = f"stty -opost; cat '{path}'; tmux -L {SOCKET} wait-for -S {signal_name}"
        start = now()
        tmux("new-window", "-t", "bench", script)
        tmux("wait-for", signal_name)
        seconds = (now() - start) / 1e9
        time.sleep(0.3)
        results[name] = {
            "megabytes": round(size / 1e6),
            "seconds": round(seconds, 3),
            "mb_per_s": round(size / 1e6 / seconds, 1),
            "client_received_mb": round((drain.bytes - before) / 1e6, 1),
        }
        print(f"  tmux   {name:<14}{results[name]['mb_per_s']:>8} MB/s  ({seconds:.1f} s)", flush=True)
    rss_after_flood = server_rss_mb()
    drain.stop = True
    drain.join()
    kill(pid, fd)
    stop_server()
    return results, rss_after_flood


# ---------------------------------------------------------------- key latency

PROBE = r"""
import os, sys, termios, time, tty
log = sys.argv[1]
tty.setraw(0)
records = []
n = 0
while True:
    b = os.read(0, 1)
    t1 = time.perf_counter_ns()
    if not b or b == b'Q':
        break
    token = b'<%05d>' % n
    t2 = time.perf_counter_ns()
    os.write(1, token)
    records.append((n, t1, t2))
    n += 1
    if n % 12 == 0:
        os.write(1, b'\r\n')
with open(log, 'w') as f:
    for r in records:
        f.write('%d %d %d\n' % r)
"""


def latency_run(fd, log, samples):
    """Send keys one at a time through `fd`; the probe in the pane stamps arrival and echo."""
    sent, seen, buffer = [], [], b""
    stamp, buffer = read_until(fd, b"PROBE-READY", 10)
    assert stamp, "probe did not start"
    time.sleep(0.3)
    buffer = b""
    for n in range(samples):
        t0 = now()
        os.write(fd, b"a")
        t3, buffer = read_until(fd, b"<%05d>" % n, 5, buffer)
        if t3 is None:
            raise RuntimeError(f"echo {n} never reached the client")
        sent.append(t0)
        seen.append(t3)
        buffer = buffer[-64:]
        time.sleep(0.004 + (n % 7) * 0.001)
    os.write(fd, b"Q")
    for _ in range(100):
        if os.path.exists(log) and os.path.getsize(log) > 0:
            break
        time.sleep(0.05)
    time.sleep(0.1)
    rows = [tuple(map(int, line.split())) for line in open(log)]
    key_in = [t1 - sent[n] for n, t1, _ in rows[:samples]]
    echo_out = [seen[n] - t2 for n, _, t2 in rows[:samples]]
    round_trip = [seen[n] - sent[n] for n in range(samples)]
    return {"key_to_pane": summary_us(key_in), "pane_to_client": summary_us(echo_out), "round_trip": summary_us(round_trip)}


def probe_command(log):
    probe_path = os.path.join(OUT_DIR, "probe.py")
    with open(probe_path, "w") as f:
        f.write(PROBE)
    if os.path.exists(log):
        os.remove(log)
    return f"printf PROBE-READY; exec python3 '{probe_path}' '{log}'"


def latency(sizes):
    samples = 200 if QUICK else 1000
    results = {}
    log = os.path.join(OUT_DIR, "probe.log")

    pid, fd = spawn_in_pty(["sh", "-c", probe_command(log)])
    results["direct"] = latency_run(fd, log, samples)
    kill(pid, fd)

    start_server(probe_command(log))
    pid, fd = attach()
    results["tmux_idle"] = latency_run(fd, log, samples)
    kill(pid, fd)
    stop_server()

    # Same measurement while a second, visible pane floods output without pause.
    start_server(probe_command(log))
    path = sizes["plain"][0]
    tmux("split-window", "-d", "-t", "bench", f"stty -opost; while :; do cat '{path}'; done")
    pid, fd = attach()
    results["tmux_other_pane_flooding"] = latency_run(fd, log, samples)
    kill(pid, fd)
    stop_server()
    return results


# ---------------------------------------------------------------- resources


def cpu_seconds(pid):
    text = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    minutes, seconds = text.rsplit(":", 1)
    hours = 0
    if ":" in minutes:
        hours, minutes = minutes.split(":")
    return int(hours) * 3600 + int(minutes) * 60 + float(seconds)


def rss_mb(pid):
    text = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return round(int(text) / 1024, 1)


def server_rss_mb():
    return rss_mb(int(tmux("display-message", "-p", "#{pid}")))


def idle_cpu():
    seconds = 10 if QUICK else 60
    server = start_server()
    pid, fd = attach()
    drain = Drain(fd)
    drain.start()
    time.sleep(2)
    before_server, before_client, before_bytes = cpu_seconds(server), cpu_seconds(pid), drain.bytes
    time.sleep(seconds)
    result = {
        "window_seconds": seconds,
        "server_cpu_seconds": round(cpu_seconds(server) - before_server, 2),
        "client_cpu_seconds": round(cpu_seconds(pid) - before_client, 2),
        "bytes_sent_to_host_terminal": drain.bytes - before_bytes,
        "redraws_seen": drain.reads,
    }
    drain.stop = True
    drain.join()
    kill(pid, fd)
    stop_server()
    return result


def memory(fill):
    server = start_server()
    time.sleep(0.5)
    result = {"server_one_idle_pane_mb": rss_mb(server)}
    tmux("new-window", "-t", "bench", f"stty -opost; cat '{fill}'; tmux -L {SOCKET} wait-for -S filled; cat")
    tmux("wait-for", "filled")
    time.sleep(0.5)
    result["history_lines_kept"] = int(tmux("display-message", "-p", "-t", "bench", "#{history_size}"))
    result["server_after_50000_lines_of_120_columns_mb"] = rss_mb(server)
    result["one_pane_50000_lines_mb"] = round(
        result["server_after_50000_lines_of_120_columns_mb"] - result["server_one_idle_pane_mb"], 1
    )
    stop_server()

    server = start_server()
    for _ in range(19):
        tmux("new-window", "-d", "-t", "bench", "cat")
    time.sleep(1)
    result["server_20_idle_panes_mb"] = rss_mb(server)
    stop_server()
    return result


# ---------------------------------------------------------------- startup


def startup():
    rounds = 10 if QUICK else 40
    result = {}

    start_server("printf READY-MARK; cat")
    time.sleep(0.5)
    times = []
    for _ in range(rounds):
        t0 = now()
        pid, fd = attach()
        t1, _ = read_until(fd, b"READY-MARK", 5)
        assert t1, "attach showed no frame"
        times.append(t1 - t0)
        kill(pid, fd)
        time.sleep(0.05)
    result["attach_to_first_frame"] = summary_us(times)

    times = []
    for _ in range(rounds):
        t0 = now()
        subprocess.run(["tmux", "-L", SOCKET, "-f", "/dev/null", "display-message", "-p", "x"], env=ENV, capture_output=True)
        times.append(now() - t0)
    result["command_line_round_trip"] = summary_us(times)

    # Switching window: time from the key to the other window's content reaching the client.
    tmux("new-window", "-d", "-t", "bench", "printf SECOND-WINDOW; cat")
    pid, fd = attach()
    read_until(fd, b"READY-MARK", 5)
    times = []
    for index in range(rounds):
        marker = b"SECOND-WINDOW" if index % 2 == 0 else b"READY-MARK"
        time.sleep(0.05)
        while select.select([fd], [], [], 0.05)[0]:
            os.read(fd, 1 << 16)
        t0 = now()
        os.write(fd, b"\x02n")
        t1, _ = read_until(fd, marker, 5)
        assert t1, "window switch showed no frame"
        times.append(t1 - t0)
    result["switch_window_to_frame"] = summary_us(times)
    kill(pid, fd)
    stop_server()

    times = []
    for index in range(rounds):
        name = f"{SOCKET}-fresh-{index}"
        sockets.append(name)
        t0 = now()
        pid, fd = spawn_in_pty(["tmux", "-L", name, "-f", "/dev/null", "new-session", "printf READY-MARK; cat"])
        t1, _ = read_until(fd, b"READY-MARK", 5)
        assert t1, "new server showed no frame"
        times.append(t1 - t0)
        kill(pid, fd)
        tmux("kill-server", socket=name, check=False)
    result["start_server_and_first_session_to_first_frame"] = summary_us(times)
    return result


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    sizes, fill = ensure_workloads()
    result = {
        "tmux_version": subprocess.run(["tmux", "-V"], capture_output=True, text=True).stdout.strip(),
        "quick": QUICK,
        "terminal_size": f"{COLS}x{ROWS}",
    }
    print("Throughput (command prints a file inside the pane, a client is attached):", flush=True)
    result["throughput_direct"] = {}
    for name, (path, size) in sizes.items():
        result["throughput_direct"][name] = throughput_direct(path, size)
        print(f"  direct {name:<14}{result['throughput_direct'][name]['mb_per_s']:>8} MB/s", flush=True)
    result["throughput_tmux"], result["server_rss_after_flood_mb"] = throughput_tmux(sizes)
    print("Key latency:", flush=True)
    result["latency"] = latency(sizes)
    for name, value in result["latency"].items():
        print(f"  {name}: {json.dumps(value)}", flush=True)
    print("Idle CPU:", flush=True)
    result["idle"] = idle_cpu()
    print(f"  {json.dumps(result['idle'])}", flush=True)
    print("Memory:", flush=True)
    result["memory"] = memory(fill)
    print(f"  {json.dumps(result['memory'])}", flush=True)
    print("Startup:", flush=True)
    result["startup"] = startup()
    for name, value in result["startup"].items():
        print(f"  {name}: {json.dumps(value)}", flush=True)
    path = os.path.join(OUT_DIR, "tmux-baseline.json")
    with open(path, "w") as f:
        json.dump(result, f, indent=2)
    print(f"\nMachine-readable results written to {path}")


if __name__ == "__main__":
    main()
