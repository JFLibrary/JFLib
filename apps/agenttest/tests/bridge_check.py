#!/usr/bin/env python3
"""Real loopback HTTP tests of the optional browser bridge, without a browser dependency."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import sys
import threading
import time
from urllib.error import HTTPError
from urllib.request import Request, ProxyHandler, build_opener

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
from agent_bridge import BridgeServer, MAX_PENDING, State, remote_rpc, strict_json

ORIGIN = "http://127.0.0.1:8765"


def main():
    checks = 0
    server = BridgeServer(ORIGIN)
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
    thread.start()
    config = server.config()
    assert config["url"].startswith("http://127.0.0.1:") and len(config["token"]) >= 43
    checks += 1
    opener = build_opener(ProxyHandler({}))

    def http(path, value=None, *, method="POST", token=config["token"], origin=ORIGIN, extra=None):
        headers = {"Content-Type": "application/json"}
        if token is not None:
            headers["Authorization"] = "Bearer " + token
        if origin is not None:
            headers["Origin"] = origin
        headers.update(extra or {})
        data = None if method in ("GET", "OPTIONS") else json.dumps(value).encode("utf-8")
        request = Request(config["url"] + path, data=data, headers=headers, method=method)
        try:
            response = opener.open(request, timeout=4)
        except HTTPError as error:
            response = error
        with response:
            body = response.read()
            return response.status, json.loads(body) if body else None, dict(response.headers)

    try:
        assert http("/connect", {"app": "test"}, token="wrong")[0] == 403
        assert http("/connect", {"app": "test"}, token=None)[0] == 403
        checks += 2
        assert http("/connect", {"app": "test"}, origin="http://other.invalid")[0] == 403
        assert http("/connect", {"app": "test"}, origin=None)[0] == 403
        checks += 2
        status, _, headers = http("/connect", method="OPTIONS", token=None, extra={"Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "authorization,content-type"})
        assert status == 204 and headers["Access-Control-Allow-Origin"] == ORIGIN
        assert "*" not in headers["Access-Control-Allow-Origin"]
        checks += 1
        assert http("/connect", method="OPTIONS", origin="http://other.invalid", extra={"Access-Control-Request-Method": "POST"})[0] == 403
        checks += 1
        assert http("/connect", {"app": None})[0] == 400
        assert not server.state.client
        checks += 1
        request = {"jsonrpc": "2.0", "id": 1, "method": "agent/info"}
        assert "No activated browser" in remote_rpc(config["url"], config["token"], "agent-a", request)["error"]["message"]
        checks += 1
        status, body, _ = http("/connect", {"app": "agenttest"})
        assert status == 200 and body["protocolVersion"] == "1"
        client = body["client"]
        assert http("/connect", {"app": "second"})[0] == 400
        checks += 2
        assert http("/poll?client=incorrect", method="GET")[0] == 400
        checks += 1
        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = [pool.submit(remote_rpc, config["url"], config["token"], session, request) for session in ("agent-a", "agent-b")]
            envelopes = []
            while len(envelopes) < 2:
                status, value, _ = http("/poll?client=" + client, method="GET")
                assert status == 200
                envelopes.extend(value["requests"])
            assert {item["session"] for item in envelopes} == {"agent-a", "agent-b"}
            assert envelopes[0]["id"] != envelopes[1]["id"]
            checks += 1
            item = envelopes[0]
            assert http("/result", {"client": client, "id": item["id"], "response": {"jsonrpc": "2.0", "id": 999, "result": {}}})[0] == 400
            checks += 1
            for item in reversed(envelopes):
                assert http("/result", {"client": client, "id": item["id"], "response": {"jsonrpc": "2.0", "id": 1, "result": {"session": item["session"]}}})[0] == 200
            assert [future.result(timeout=2)["result"]["session"] for future in futures] == ["agent-a", "agent-b"]
            checks += 1
        assert http("/result", {"client": client, "id": envelopes[0]["id"], "response": {"jsonrpc": "2.0", "id": 1, "result": {}}})[0] == 400
        checks += 1
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(remote_rpc, config["url"], config["token"], "notify", {"jsonrpc": "2.0", "method": "notifications/initialized"})
            item = http("/poll?client=" + client, method="GET")[1]["requests"][0]
            assert http("/result", {"client": client, "id": item["id"], "response": None})[0] == 200
            assert future.result(timeout=2) is None
            checks += 1
        # Exercise the actual CLI stdio gateway while emulating the browser-side host.
        raw = {"jsonrpc": "2.0", "id": "mcp", "method": "server/discover", "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}}}
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(subprocess.run, [sys.executable, str(ROOT / "tools/jfgui-agent.py"), "browser-stdio", "--url", config["url"], "--token", config["token"]], input=json.dumps(raw) + "\n", text=True, capture_output=True, timeout=5)
            item = http("/poll?client=" + client, method="GET")[1]["requests"][0]
            assert item["request"] == raw
            http("/result", {"client": client, "id": item["id"], "response": {"jsonrpc": "2.0", "id": "mcp", "result": {"resultType": "complete", "supportedVersions": ["2026-07-28"]}}})
            closing = http("/poll?client=" + client, method="GET")[1]["requests"][0]
            assert closing["closeSession"] is True and closing["session"] == item["session"]
            assert http("/result", {"client": client, "id": closing["id"], "response": None})[0] == 200
            checks += 1
            process = future.result(timeout=5)
            assert process.returncode == 0, process.stderr
            assert json.loads(process.stdout)["result"]["supportedVersions"] == ["2026-07-28"]
            checks += 1
        assert "timed out" in server.state.submit("timed", request, timeout=0.01)["error"]["message"]
        assert not server.state.pending
        checks += 1
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(remote_rpc, config["url"], config["token"], "disconnected", request)
            http("/poll?client=" + client, method="GET")
            assert http("/disconnect", {"client": client})[0] == 200
            assert "disconnected" in future.result(timeout=2)["error"]["message"]
            checks += 1
        # The queue cap is independent of HTTP threads and rejects work before execution.
        state = State(ORIGIN)
        fake_client = state.connect("bounded")["client"]
        with ThreadPoolExecutor(max_workers=MAX_PENDING) as pool:
            futures = [pool.submit(state.submit, str(index), request, 5) for index in range(MAX_PENDING)]
            deadline = time.monotonic() + 2
            while len(state.pending) < MAX_PENDING and time.monotonic() < deadline:
                time.sleep(0.005)
            assert "queue is full" in state.submit("overflow", request)["error"]["message"]
            assert len(state.poll(fake_client, timeout=0)["requests"]) == 8
            state.disconnect(fake_client)
            assert all("disconnected" in future.result(timeout=2)["error"]["message"] for future in futures)
            checks += 1
        for malformed in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":Infinity}'):
            try:
                strict_json(malformed)
                raise AssertionError("Malformed JSON accepted")
            except ValueError:
                checks += 1
        try:
            remote_rpc("http://example.invalid", "secret", "x", request)
            raise AssertionError("Non-loopback URL accepted")
        except ValueError:
            checks += 1
        print(f"bridge_check: {checks} checks, 0 failures")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


if __name__ == "__main__":
    main()
