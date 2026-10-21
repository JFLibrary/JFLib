"""Verify real browser DOCX artifacts in both native jfWord executables.

Run after web_apps.mjs. The outputs are suitable for the same browser file picker.
No Microsoft Office installation or private files are used.
"""
import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("jfword_agent_tests", ROOT / "apps/jfWord/tests/agent.py")
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("build", nargs="?", default="build/office-universal")
    args = parser.parse_args()
    build = (ROOT / args.build).resolve()
    artifacts = build / "browser-tests"
    results = []
    for target in ("win", "linux"):
        for engine in ("chromium", "firefox"):
            path = artifacts / f"browser-{engine}.docx"
            original = path.read_bytes()
            app = agent.Session(target, build)
            try:
                native_path = str(path) if target == "win" else agent.build_rs.wsl_path(path)
                app.invoke("file.open", {"path": native_path}, app.state()["revision"])
                state = app.state()
                text = state["paragraphs"][0]["text"]
                assert "Browserprüfung über DOM 研究 Zwischenablage" in text, state
                assert state["sources"][0]["title"] == "Web und native Quellen", state
                assert any(node["format"].get("bold") for node in state["paragraphs"][0]["inlines"]), state
                before = state["revision"]
                exported = app.invoke("document.export")
                content = app.resource(exported["resources"][0]["uri"])
                assert app.state()["revision"] == before
                assert content == original, f"{target}/{engine}: native DOCX export differs from browser package"
                output = artifacts / f"native-{target}-{engine}.docx"
                output.write_bytes(content)
                results.append({"target": target, "browser": engine, "bytes": len(content), "byteIdentical": True})
                print(f"{target} <- {engine} DOCX: exact {len(content)} bytes", flush=True)
            finally:
                app.close()
    (artifacts / "native-transfer.json").write_text(json.dumps(results, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
