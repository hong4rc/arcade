#!/usr/bin/env python3
"""Self-check of bump-formula.py: run `python3 -I scripts/test-bump-formula.py`."""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
BUMP = os.path.join(HERE, "bump-formula.py")
OLD = "a" * 64
NEW_MAC = "b" * 64
NEW_LINUX = "c" * 64
URL = "https://github.com/hong4rc/arcade/releases/download"

FORMULA = f'''class Arcade < Formula
  on_macos do
    on_arm do
      url "{URL}/v0.1.4/arcade-macos-apple-silicon.tar.gz"
      sha256 "{OLD}"
    end
  end
  on_linux do
    on_arm do
      url "{URL}/v0.1.4/arcade-linux-arm64.tar.gz"
      sha256 "{OLD}"
    end
    on_intel do
      url "{URL}/v0.1.4/arcade-linux-amd64.tar.gz"
      sha256 "{OLD}"
    end
  end
end
'''


def run(sums_text):
    with tempfile.TemporaryDirectory() as d:
        formula = os.path.join(d, "arcade.rb")
        sums = os.path.join(d, "SHA256SUMS.txt")
        open(formula, "w").write(FORMULA)
        open(sums, "w").write(sums_text)
        out = subprocess.run(
            [sys.executable, "-I", BUMP, formula, "v0.2.0", sums],
            capture_output=True,
            text=True,
        )
        return out, open(formula).read()


def check(cond, what):
    if not cond:
        sys.exit(f"FAILED: {what}")
    print(f"ok: {what}")


# A checksum file with every target bumps them all.
out, text = run(
    f"{NEW_MAC}  arcade-macos-apple-silicon.tar.gz\n"
    f"{NEW_LINUX}  arcade-linux-arm64.tar.gz\n"
    f"{NEW_LINUX}  arcade-linux-amd64.tar.gz\n"
)
check(out.returncode == 0 and "bumped 3" in out.stdout, "all three targets are bumped")
check(f"{URL}/v0.2.0/arcade-linux-amd64.tar.gz" in text, "the amd64 url moved to the new tag")

# A checksum file without linux-amd64 leaves that url and sha256 untouched.
out, text = run(
    f"{NEW_MAC}  arcade-macos-apple-silicon.tar.gz\n"
    f"{NEW_LINUX}  arcade-linux-arm64.tar.gz\n"
)
check(out.returncode == 0 and "bumped 2" in out.stdout, "two targets are bumped")
check("kept arcade-linux-amd64.tar.gz" in out.stdout, "the kept target is named")
check(
    f'url "{URL}/v0.1.4/arcade-linux-amd64.tar.gz"\n      sha256 "{OLD}"' in text,
    "linux-amd64 keeps its old url and sha256",
)
check(
    f'url "{URL}/v0.2.0/arcade-macos-apple-silicon.tar.gz"\n      sha256 "{NEW_MAC}"' in text,
    "macos is on the new tag with its new sha256",
)
check(text.count(OLD) == 1, "only the amd64 sha256 is still the old one")

# A checksum file with none of the targets is an error and changes nothing.
out, text = run(f"{NEW_MAC}  arcade-other.tar.gz\n")
check(out.returncode != 0 and text == FORMULA.rstrip("\n") + "\n", "no known target is an error")
print("all checks passed")
