"""Exercise the real macOS TUI with an isolated, controllably blocked Keychain."""

from __future__ import annotations

import argparse
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

SENTINEL = "UNTRUSTED_CLAUDE_SENTINEL"
DEADLINE_SECONDS = 5
CSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
OSC = re.compile(rb"\x1b\].*?(?:\x07|\x1b\\)", re.DOTALL)
CASES = ("offline-empty", "online-local", "online-usage-blocked", "offline-untrusted-cache")


def child_tty_setup() -> None:
    """Make the slave the child's controlling terminal before exec."""
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


class Probe:
    def __init__(self, binary: Path, case: str, root: Path) -> None:
        self.case = case
        self.root = root
        self.marker = root / "keychain-called"
        self.release = root / "release-keychain"
        self.output = bytearray()
        config = root / ".config" / "tokscale"
        cache = config / "cache"
        cache.mkdir(parents=True)
        (config / "settings.json").write_text(
            json.dumps({"autoRefreshEnabled": False, "tuiLanguage": "en"})
        )
        now = int(time.time())
        empty = {"timestamp": now, "data": {}}
        for name in ("pricing-litellm.json", "pricing-openrouter.json"):
            (cache / name).write_text(json.dumps(empty))
        (cache / "pricing-models-dev.json").write_text(
            json.dumps(
                {
                    "timestamp": now,
                    "data": {
                        "fixture/unused-model": {
                            "input_cost_per_token": 0.000001,
                            "output_cost_per_token": 0.000002,
                        }
                    },
                }
            )
        )
        if case == "offline-untrusted-cache":
            (cache / "subscription-usage-cache.json").write_text(
                json.dumps(
                    {
                        "timestamp": now,
                        "data": [
                            {"provider": "Claude", "plan": SENTINEL, "email": None, "metrics": []}
                        ],
                    }
                )
            )
        fake_bin = root / "bin"
        fake_bin.mkdir()
        security = fake_bin / "security"
        security.write_text(
            '#!/bin/sh\ncase "$*" in\n'
            '  *"Claude Code-credentials"*)\n'
            '    printf "%s\\n" "$$" >> "$TOKSCALE_TEST_KEYCHAIN_MARKER"\n'
            '    while test ! -f "$TOKSCALE_TEST_KEYCHAIN_RELEASE"; do /bin/sleep 0.05; done\n'
            "    ;;\nesac\nexit 1\n"
        )
        security.chmod(0o700)
        # Preserve only loader settings that Cargo may require for an all-features
        # binary. No provider tokens, config roots, proxy secrets or home leak in.
        env = {
            key: value
            for key, value in os.environ.items()
            if key.startswith(("DYLD_", "LD_LIBRARY_PATH"))
        }
        env.update(
            {
                "HOME": str(root),
                "TOKSCALE_CONFIG_DIR": str(config),
                "TMPDIR": str(root),
                "PATH": f"{fake_bin}:/usr/bin:/bin",
                "TERM": "xterm-256color",
                "LANG": "C",
                "TOKSCALE_TEST_KEYCHAIN_MARKER": str(self.marker),
                "TOKSCALE_TEST_KEYCHAIN_RELEASE": str(self.release),
            }
        )
        master, slave = pty.openpty()
        self.master = master
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        args = [str(binary), "--no-spinner"]
        if case.startswith("offline-"):
            args.append("--offline")
        args.append("tui")
        try:
            self.process = subprocess.Popen(
                args,
                stdin=slave,
                stdout=slave,
                stderr=slave,
                env=env,
                cwd=root,
                # This dedicated fixture process never starts threads.
                preexec_fn=child_tty_setup,  # noqa: PLW1509
            )
        except BaseException:
            os.close(master)
            raise
        finally:
            os.close(slave)
        os.set_blocking(master, False)

    @property
    def text(self) -> str:
        return CSI.sub(b"", OSC.sub(b"", bytes(self.output))).decode(errors="replace")

    def read_until(self, predicate, description: str) -> None:
        end = time.monotonic() + DEADLINE_SECONDS
        while not predicate():
            if time.monotonic() >= end:
                raise AssertionError(
                    f"{description} timed out; keychain_called={self.marker.exists()}; "
                    f"output={self.text[-4000:]!r}"
                )
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    chunk = os.read(self.master, 65536)
                except BlockingIOError:
                    continue
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                if chunk:
                    self.output.extend(chunk)
            if self.process.poll() is not None and not predicate():
                raise AssertionError(f"TUI exited before {description}; output={self.text!r}")

    def run(self) -> None:
        self.read_until(
            lambda: "overview" in self.text.lower() and "usage" in self.text.lower(),
            "visible first frame",
        )
        assert not self.marker.exists(), "startup queried Keychain before Usage was opened"
        if self.case in ("online-usage-blocked", "offline-untrusted-cache"):
            os.write(self.master, b"\t")
            self.read_until(
                lambda: any(
                    content.replace(" ", "") in re.sub(r"\s+", "", self.text)
                    for content in (
                        "Fetching subscription data...",
                        "No subscription data available",
                        "Selected Account",
                    )
                ),
                "Usage content area",
            )
        if self.case == "online-usage-blocked":
            self.read_until(self.marker.exists, "blocked fake Keychain lookup")
            assert not self.release.exists()
        else:
            assert not self.marker.exists(), "local/offline TUI queried Keychain"
        assert SENTINEL not in self.text, "unverified Claude card was rendered"
        os.write(self.master, b"q")
        # Keep draining while waiting: a PTY's output buffer can fill before
        # the application reaches its next keyboard event or terminal restore.
        self.read_until(lambda: self.process.poll() is not None, "q exits with Keychain held")
        status = self.process.wait(timeout=DEADLINE_SECONDS)
        # A heading can arrive in a different PTY read from the card below it.
        # Inspect the complete final frame, including bytes buffered at exit.
        end = time.monotonic() + 1
        while time.monotonic() < end:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if not ready:
                break
            try:
                chunk = os.read(self.master, 65536)
            except BlockingIOError:
                continue
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                break
            if not chunk:
                break
            self.output.extend(chunk)
        assert status == 0, f"TUI exited with status {status}: {self.text!r}"
        assert SENTINEL not in self.text, "unverified Claude card was rendered"
        assert not self.release.exists(), "fixture released Keychain before verifying exit"
        if self.case != "online-usage-blocked":
            assert not self.marker.exists(), "startup/offline navigation queried Keychain"

    def close(self) -> None:
        self.release.touch()
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        except PermissionError:
            if self.process.poll() is None:
                self.process.terminate()
        try:
            self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=2)
        finally:
            os.close(self.master)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--case", choices=CASES, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("the real Keychain startup regression requires macOS")
    with tempfile.TemporaryDirectory(prefix="tokscale-tui-startup-") as directory:
        probe = Probe(args.binary.resolve(strict=True), args.case, Path(directory))
        try:
            probe.run()
        finally:
            probe.close()


if __name__ == "__main__":
    main()
