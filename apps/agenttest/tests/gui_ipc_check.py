#!/usr/bin/env python3
"""Exercise opt-in native IPC in real Gallery/Kritzel event loops without user files."""

import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("jfgui_agent", ROOT / "tools/jfgui-agent.py")
agent = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agent)


def launch(exe, enabled):
    options = dict(stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT)
    if os.name == "nt":
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        options.update(startupinfo=startup, creationflags=0x08000000)
    args = [str(exe), "--auto-close", "20000" if enabled else "2500", "--size", "800x600"]
    if enabled:
        args.append("--agent")
    return subprocess.Popen(args, **options)


def endpoints(process):
    pattern = re.compile(rf"-{process.pid}-[0-9]+(?:\.sock)?$")
    return [address for address in agent.addresses() if pattern.search(address)]


def wait_endpoint(process):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        found = endpoints(process)
        if found:
            assert len(found) == 1
            return found[0]
        if process.poll() is not None:
            raise AssertionError(process.stderr.read().decode(errors="replace"))
        time.sleep(0.03)
    raise AssertionError("Enabled native app did not create an endpoint")


def call(client, action, input=None, revision=None):
    params = {"action": action, "input": input or {}}
    if revision is not None:
        params["expectedRevision"] = revision
    result = client.request("actions/execute", params)
    assert "result" in result, result
    return result["result"]


def close_cleanly(process):
    process.wait(timeout=25)
    assert process.returncode == 0, process.stderr.read().decode(errors="replace")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    suffix = ".exe" if os.name == "nt" else ""
    exes = [(args.bin_dir / (name + suffix)).resolve(strict=True) for name in ("gallery", "Kritzel")]
    children = []
    checks = 0
    try:
        # Inspect the OS endpoint inventory repeatedly while both normal windows run.
        disabled = [launch(exe, False) for exe in exes]
        children.extend(disabled)
        while any(child.poll() is None for child in disabled):
            for child in disabled:
                assert not endpoints(child), "A normal app opened agent IPC without activation"
            time.sleep(0.025)
        for child in disabled:
            close_cleanly(child)
            checks += 1

        enabled = [launch(exe, True) for exe in exes]
        children.extend(enabled)
        addresses = [wait_endpoint(child) for child in enabled]
        assert addresses[0] != addresses[1]
        checks += 1
        listing = subprocess.run([sys.executable, str(ROOT / "tools/jfgui-agent.py"), "list"],
                                 text=True, encoding="utf-8", capture_output=True, timeout=10)
        assert listing.returncode == 0, listing.stderr
        discovered = {item["endpoint"] for item in json.loads(listing.stdout)["instances"]}
        assert all(address in discovered for address in addresses)
        checks += 1
        for address in addresses:
            with agent.Connection(address) as client:
                info = client.request("agent/info", {})["result"]
                assert info["controlEnabled"] is True
                state = client.request("state/inspect", {"limit": 5})["result"]
                assert state["total"] > 0 and 0 < len(state["widgets"]) <= 5
                assert state["control"]["enabled"] is True
                checks += 1
                actions = client.request("actions/list", {"query": "ui.", "limit": 20})["result"]["actions"]
                assert {item["name"] for item in actions} >= {"ui.state", "ui.render", "ui.click", "ui.key"}
                checks += 1
                rendered = call(client, "ui.render")
                resource = rendered["resources"][0]
                assert resource["mimeType"] == "image/png"
                data = client.request("resources/read", {"uri": resource["uri"], "offset": 0, "limit": 8})["result"]
                assert base64.b64decode(data["data"]) == b"\x89PNG\r\n\x1a\n"
                checks += 1
        with agent.Connection(addresses[1]) as kritzel:
            initial = call(kritzel, "image.inspect")
            original = initial["value"]
            resized = call(kritzel, "image.resize", {"width": 8, "height": 6}, initial["revision"])
            assert (resized["value"]["width"], resized["value"]["height"]) == (8, 6)
            checks += 1
            filled = call(kritzel, "image.layer.fill", {"argb": 0xFF123456}, resized["revision"])
            assert call(kritzel, "image.pixel", {"x": 2, "y": 3})["value"]["argb"] == 0xFF123456
            checks += 1
            undone = call(kritzel, "image.undo", revision=filled["revision"])
            restored = call(kritzel, "image.undo", revision=undone["revision"])["value"]
            assert (restored["width"], restored["height"]) == (original["width"], original["height"])
            checks += 1
        for child, address in zip(enabled, addresses):
            close_cleanly(child)
            assert address not in endpoints(child)
            checks += 1
        print(f"gui_ipc_check: {checks} checks, 0 failures")
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=5)


if __name__ == "__main__":
    main()
