"""Run the pre-pull-request checks from the repository root."""

import argparse
import os
from pathlib import Path
import shlex
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true", help="print commands without running them")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--fast", action="store_true", help="run only the Python checks")
    mode.add_argument("--test", metavar="FILTER", help="also run the named Rust library tests")
    args = parser.parse_args()
    if Path.cwd() != Path(__file__).resolve().parents[1]:
        parser.error("run tools/check.py from the repository root")

    windows = sys.platform == "win32"
    clippy = ["cargo", "clippy", "--workspace", "--locked", "--all-targets"]
    asr = ["--features", "asr-runtime"]
    commands = []
    if not args.fast:
        commands += [
            ["cargo", "fmt", "--all", "--", "--check"],
            [*clippy, "--", "-D", "warnings"],
            [*clippy, *asr, "--", "-D", "warnings"],
        ]
        if windows:
            check = ["cargo", "check", "--workspace", "--locked", "--tests"]
            commands += [check, [*check, *asr]]

    python = "python" if windows else "python3"
    commands += [
        [python, "-I", "tools/prove_linux_boundaries.py", ".", "--self-test"],
        [python, "-I", "tools/check_links.py", "--self-test"],
        [python, "-I", "tools/check_links.py"],
        [python, "-I", "tools/icon/check_icon.py"],
        [python, "-I", ".github/ci_changes.py", "--self-test"],
    ]
    if args.test is not None:
        commands.append(["cargo", "test", "--locked", "-p", "sidenote", "--lib", args.test])

    if not args.list:
        # Split the old channel name so this check does not match itself.
        obsolete = subprocess.run([
            "git", "grep", "-ilIF", "-e", "night" "ly", "--",
            ":!tools/linux-boundary-inventory.json", ":(icase,exclude,glob)**/CHANGELOG*",
        ])
        if obsolete.returncode == 0:
            sys.exit("check: use Dev for the developer channel in the files above")
        if obsolete.returncode != 1:
            return obsolete.returncode

    cargo_env = None
    if sys.platform == "linux" and not args.fast and not args.list:
        cargo_env = dict(os.environ, RUSTUP_TOOLCHAIN="stable",
                         CARGO_BUILD_TARGET="x86_64-unknown-linux-gnu")

    for command in commands:
        display = subprocess.list2cmdline(command) if windows else shlex.join(command)
        print(display, flush=True)
        if args.list:
            continue
        executable = command[0]
        try:
            if executable == python:
                result = subprocess.run([sys.executable, *command[1:]])
            else:
                result = subprocess.run(command, env=cargo_env)
        except FileNotFoundError:
            sys.exit(f"check: could not find {executable}")
        if result.returncode:
            return result.returncode if result.returncode > 0 else 128 - result.returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
