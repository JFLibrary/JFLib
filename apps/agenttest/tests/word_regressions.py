#!/usr/bin/env python3
"""Agent regression cases for jfWord history, clipboard failure, modal and revision rules."""
import argparse
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    args = parser.parse_args()
    process = subprocess.Popen([str(args.exe.resolve(strict=True)), "--agent-json", "--editor"],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               creationflags=0x08000000 if os.name == "nt" else 0)
    count = 0

    def rpc(method, params=None):
        nonlocal count
        count += 1
        process.stdin.write((json.dumps({"jsonrpc": "2.0", "id": count, "method": method, "params": params or {}}) + "\n").encode())
        process.stdin.flush()
        response = json.loads(process.stdout.readline())
        assert response["id"] == count, response
        return response

    def action(name, input=None, revision=0):
        return rpc("actions/execute", {"action": name, "input": input or {}, "expectedRevision": revision})

    def error(response, code):
        assert response.get("error", {}).get("data", {}).get("code") == code, response

    checks = 0
    try:
        initial = rpc("state/inspect", {"domain": "document"})["result"]
        revision = initial["revision"]
        paragraph = initial["paragraphs"][0]["id"]
        controls = rpc("state/inspect", {})["result"]
        assert all(not command["enabled"] for command in controls["commands"] if command["id"] in (5, 6))
        checks += 1
        error(action("document.undo", revision=revision), "unavailable")
        error(action("document.redo", revision=revision), "unavailable")
        checks += 2
        # A collapsed selection must not read/change the user's native clipboard.
        assert "result" in action("document.select", {"paragraph": paragraph, "start": 0, "end": 0}, revision)
        error(action("clipboard.cut", revision=revision), "unavailable")
        checks += 1
        edit = {"paragraph": paragraph, "start": 0, "end": 0, "text": "Revision prüfen"}
        changed = action("document.text.replace", edit, revision)["result"]
        assert changed["revision"] > revision
        error(action("document.text.replace", edit, revision), "revision_conflict")
        revision = changed["revision"]
        checks += 1
        batch = rpc("actions/batch", {"atomic": True, "calls": [
            {"action": "document.text.replace", "input": edit, "expectedRevision": revision},
        ]})
        error(batch, "unsupported")
        assert rpc("state/inspect", {"domain": "document"})["result"]["revision"] == revision
        checks += 1
        assert "result" in action("document.undo", revision=revision)
        current = rpc("state/inspect", {"domain": "document"})["result"]
        assert current["paragraphs"][0]["text"] == ""
        revision = current["revision"]
        checks += 1
        assert "result" in action("ui.command", {"id": 2}, revision)
        assert rpc("state/inspect", {})["result"]["modal"] != 0
        error(action("document.text.replace", edit, revision), "unavailable")
        assert rpc("state/inspect", {"domain": "document"})["result"]["revision"] == revision
        checks += 1
        print(f"word_regressions: {checks} checks, 0 failures")
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
        if process.returncode:
            raise AssertionError(process.stderr.read().decode(errors="replace"))


if __name__ == "__main__":
    main()
