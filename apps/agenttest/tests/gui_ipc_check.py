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


def launch(exe, enabled, size="1000x800"):
    options = dict(stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT)
    if os.name == "nt":
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        options.update(startupinfo=startup, creationflags=0x08000000)
    args = [str(exe), "--auto-close", "30000" if enabled else "2500", "--size", size]
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


def click_with_last_revision(client, gallery_cli):
    """A real window draws for every request: the revision read last must still hold."""
    checks = 0
    state = lambda **query: client.request("state/inspect", query)["result"]
    first = state(limit=500)
    time.sleep(0.2)
    assert [state(limit=1)["revision"] for _ in range(5)] == [first["revision"]] * 5
    checks += 1
    assert state(sinceUiRevision=first["uiRevision"]).get("unchanged") is True
    checks += 1
    box = next(w for w in first["widgets"] if w["label"] == "Option A")
    value = lambda: next(w for w in state(limit=500)["widgets"] if w["id"] == box["id"])["state"] & 8
    for _ in range(3):
        # read, then act: each round on a new revision, with the window's frames in between
        before, checked = state(limit=1)["revision"], value()
        time.sleep(0.05)
        after = call(client, "ui.click", {"id": box["id"]}, before)["revision"]
        assert after != before and value() != checked
        assert state(limit=1)["revision"] == after
        checks += 1
    # the revision of a reply is the one to use next, also through the CLI (one connection per command)
    revision = state(limit=1)["revision"]
    for _ in range(3):
        reply = gallery_cli("call", "ui.click", "--input-json", json.dumps({"id": box["id"]}), "--expected-revision", str(revision))
        revision = reply["result"]["revision"]
        checks += 1
    revision = gallery_cli("inspect", "--input-json", json.dumps({"limit": 1}))["result"]["revision"]
    assert gallery_cli("call", "ui.click", "--input-json", json.dumps({"id": box["id"]}), "--expected-revision", str(revision))["result"]
    checks += 1
    # a stale revision changes nothing
    stale, checked = revision, value()
    conflict = client.request("actions/execute", {"action": "ui.click", "input": {"id": box["id"]}, "expectedRevision": stale})
    assert conflict["error"]["data"]["code"] == "revision_conflict" and value() == checked
    checks += 1
    # typing: focus by a click, text and a key with the revision of the reply before
    text = next(w for w in first["widgets"] if w["kind"] == 8 and not w["state"] & 16 and w["rect"][2] > 0)
    label = lambda: next(w for w in state(limit=500)["widgets"] if w["id"] == text["id"])["label"]
    revision = call(client, "ui.click", {"id": text["id"]}, state(limit=1)["revision"])["revision"]
    revision = call(client, "ui.type", {"text": "Grüße "}, revision)["revision"]
    time.sleep(0.05)
    revision = call(client, "ui.type", {"text": "研究"}, revision)["revision"]
    assert label() == "Grüße 研究"
    call(client, "ui.key", {"key": 8}, revision)
    assert label() == "Grüße 研"
    checks += 1
    return checks


def click_reaches(exe):
    """A window too small for the page, a second window, an open popup: ui.click only reports a hit."""
    checks = 0
    process = launch(exe, True, "800x540")
    try:
        with agent.Connection(wait_endpoint(process)) as client:
            state = lambda: client.request("state/inspect", {"limit": 500})["result"]
            widget = lambda id: next(w for w in state()["widgets"] if w["id"] == id)
            # the middle of the text box is below the view: the page scrolls, the click and the text arrive
            revision = state()["revision"]
            before = widget(700)["rect"]
            revision = call(client, "ui.click", {"id": 700}, revision)["revision"]
            assert state()["focus"] == 700 and widget(700)["rect"][1] < before[1], (state()["focus"], before, widget(700))
            call(client, "ui.type", {"text": "Hallo"}, revision)
            assert widget(700)["label"] == "Hallo"
            checks += 1
            # an open popup would take the press: refused, it stays open, then its own row works
            revision = call(client, "ui.click", {"id": next(w["id"] for w in state()["widgets"] if w["label"] == "Stärke")}, state()["revision"])["revision"]
            base = next(w["id"] for w in state()["widgets"] if w["label"] == "Standard")
            refused = client.request("actions/execute", {"action": "ui.click", "input": {"id": base}, "expectedRevision": revision})
            assert refused["error"]["data"]["code"] == "unavailable" and refused["error"]["data"]["details"]["reason"] == "popup", refused
            row = next(w["id"] for w in state()["widgets"] if w["label"] == "Schließen")
            call(client, "ui.click", {"id": row}, revision)
            checks += 1
            # a second window: the press goes to its platform window
            tool = next(w["id"] for w in state()["widgets"] if w["label"] == "Werkzeugfenster")
            call(client, "ui.click", {"id": tool}, state()["revision"])
            deadline = time.monotonic() + 5
            while not any(w["id"] == 7601 for w in state()["widgets"]) and time.monotonic() < deadline:
                time.sleep(0.05)
            slider = widget(7601)
            assert slider["window"] != 0
            call(client, "ui.click", {"id": 7601, "window": slider["window"]}, state()["revision"])
            assert widget(7601)["value"] != slider["value"], (slider, widget(7601))
            checks += 1
    finally:
        process.terminate()
        process.wait(timeout=5)
    return checks


def close_cleanly(process):
    process.wait(timeout=35)
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
        def gallery_cli(*command):
            done = subprocess.run([sys.executable, str(ROOT / "tools/jfgui-agent.py"), command[0], addresses[0], *command[1:]],
                                  text=True, encoding="utf-8", capture_output=True, timeout=20)
            return json.loads(done.stdout)
        with agent.Connection(addresses[0]) as gallery:
            checks += click_with_last_revision(gallery, gallery_cli)
        checks += click_reaches(exes[0])
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
            # an app with action_revision: the window's frames leave it, a stale one is refused
            state = kritzel.request("state/inspect", {"limit": 500})["result"]
            time.sleep(0.1)
            assert kritzel.request("state/inspect", {"limit": 1})["result"]["revision"] == state["revision"]
            stale = kritzel.request("actions/execute", {"action": "image.layer.fill", "input": {"argb": 0xFF000000}, "expectedRevision": state["revision"] + 1})
            assert stale["error"]["data"]["code"] == "revision_conflict"
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
