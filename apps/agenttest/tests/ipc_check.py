#!/usr/bin/env python3
"""Cross-process native IPC/CLI checks; run from RustLib with --exe build/.../agenttest."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import queue
import stat
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("jfgui_agent", ROOT / "tools/jfgui-agent.py")
agent = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agent)


def start(exe, extra=()):
    process = subprocess.Popen([str(exe), "--ipc-server", *extra], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               creationflags=0x08000000 if os.name == "nt" else 0)
    incoming = queue.Queue()
    threading.Thread(target=lambda: incoming.put(process.stdout.readline()), daemon=True).start()
    try:
        line = incoming.get(timeout=10)
        endpoint = json.loads(line)["endpoint"]
        return process, endpoint
    except Exception:
        process.terminate()
        _, error = process.communicate(timeout=5)
        raise RuntimeError(f"IPC server failed to start: {error.decode(errors='replace')}") from None


def request(connection, method, params=None):
    return connection.request(method, params or {})


def windows_private_acl(connection):
    """Independently inspect the pipe's DACL: one allow ACE, for the caller's exact SID."""
    import ctypes as c
    from ctypes import wintypes as w
    adv = c.WinDLL("advapi32", use_last_error=True)
    kernel = c.WinDLL("kernel32", use_last_error=True)
    adv.GetSecurityInfo.argtypes = [w.HANDLE, w.DWORD, w.DWORD, c.c_void_p, c.c_void_p,
                                   c.POINTER(c.c_void_p), c.c_void_p, c.POINTER(c.c_void_p)]
    adv.GetSecurityInfo.restype = w.DWORD
    adv.GetAce.argtypes = [c.c_void_p, w.DWORD, c.POINTER(c.c_void_p)]
    adv.EqualSid.argtypes = [c.c_void_p, c.c_void_p]
    adv.OpenProcessToken.argtypes = [w.HANDLE, w.DWORD, c.POINTER(w.HANDLE)]
    adv.GetTokenInformation.argtypes = [w.HANDLE, w.DWORD, c.c_void_p, w.DWORD, c.POINTER(w.DWORD)]
    kernel.GetCurrentProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.LocalFree.argtypes = [c.c_void_p]
    acl, descriptor = c.c_void_p(), c.c_void_p()
    error = adv.GetSecurityInfo(connection.pipe, 6, 4, None, None, c.byref(acl), None, c.byref(descriptor))
    assert error == 0 and acl.value
    token = w.HANDLE()
    try:
        assert c.c_uint16.from_address(acl.value + 4).value == 1
        ace = c.c_void_p()
        assert adv.GetAce(acl, 0, c.byref(ace))
        assert c.c_ubyte.from_address(ace.value).value == 0  # ACCESS_ALLOWED_ACE_TYPE
        assert adv.OpenProcessToken(kernel.GetCurrentProcess(), 8, c.byref(token))
        length = w.DWORD()
        adv.GetTokenInformation(token, 1, None, 0, c.byref(length))
        buffer = c.create_string_buffer(length.value)
        assert adv.GetTokenInformation(token, 1, buffer, len(buffer), c.byref(length))
        sid = c.c_void_p.from_buffer(buffer).value
        assert adv.EqualSid(ace.value + 8, sid)
    finally:
        if token.value:
            kernel.CloseHandle(token)
        kernel.LocalFree(descriptor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    args = parser.parse_args()
    exe = args.exe.resolve(strict=True)
    first, a = start(exe)
    second, b = start(exe, ["--long-title"])
    checks = 0
    try:
        assert a != b
        checks += 1
        with agent.Connection(a) as client, agent.Connection(b) as other:
            if os.name == "nt":
                windows_private_acl(client)
            else:
                assert stat.S_IMODE(Path(a).parent.stat().st_mode) == 0o700
                assert stat.S_IMODE(Path(a).stat().st_mode) == 0o600
            checks += 1
            assert request(client, "agent/info")["result"]["controlEnabled"] is True
            checks += 1
            assert request(other, "agent/info")["result"]["serverInfo"]["name"] == "Unbenannt – große Grafik / Paint! mit einem langen Namen"
            checks += 1
            names = request(client, "actions/list")["result"]["actions"]
            assert any(action["name"] == "counter.add" for action in names)
            checks += 1
            assert "inputSchema" in request(client, "actions/describe", {"action": "counter.add"})["result"]
            checks += 1
            result = request(client, "actions/execute", {"action": "counter.add", "input": {"delta": 7}, "expectedRevision": 0})
            assert result["result"]["value"] == 7
            checks += 1
            assert request(client, "actions/execute", {"action": "counter.add", "input": {"delta": 1}, "expectedRevision": 0})["error"]["data"]["code"] == "revision_conflict"
            checks += 1
            assert request(other, "actions/execute", {"action": "counter.get"})["result"]["value"] == 0
            checks += 1
            # JSON connection remains JSON even after a client sends a different protocol envelope.
            client.send(json.dumps({"jsonrpc": "2.0", "id": 44, "method": "server/discover", "params": {"_meta": agent.metadata()}}).encode())
            assert json.loads(client.receive())["error"]["code"] == -32601
            assert request(client, "agent/info")["result"]["protocolVersion"] == "1"
            checks += 1
        # The same endpoint selects MCP only from an explicit first-request version envelope.
        with agent.Connection(a) as mcp:
            mcp.send(json.dumps({"jsonrpc": "2.0", "id": "discover", "method": "server/discover", "params": {"_meta": agent.metadata()}}).encode())
            assert json.loads(mcp.receive())["result"]["supportedVersions"] == [agent.MODERN]
            checks += 1
            mcp.send(json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"_meta": agent.metadata(), "name": "jfgui.actions.execute", "arguments": {"action": "counter.add", "input": {"delta": 2}, "expectedRevision": 1}}}).encode())
            result = json.loads(mcp.receive())
            assert result["result"]["structuredContent"]["data"]["value"] == 9
            checks += 1
            mcp.send(b'{"jsonrpc":"2.0","id":3,"method":"agent/info"}')
            assert "error" in json.loads(mcp.receive())
            checks += 1
        # Legacy initialization negotiates the same application state, independently per peer.
        with agent.Connection(a) as legacy:
            legacy.send(b'{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}')
            assert json.loads(legacy.receive())["result"]["protocolVersion"] == "2025-11-25"
            legacy.send(b'{"jsonrpc":"2.0","method":"notifications/initialized"}')
            legacy.send(b'{"jsonrpc":"2.0","id":2,"method":"tools/list"}')
            assert len(json.loads(legacy.receive())["result"]["tools"]) == 9
            checks += 1
        # A complete MCP stdio gateway uses the shared Rust adapter through the native endpoint.
        gateway_input = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": agent.metadata()}}) + "\n"
        gateway = subprocess.run([sys.executable, str(ROOT / "tools/jfgui-agent.py"), "stdio", a, "--protocol", "mcp"], input=gateway_input, text=True, capture_output=True, timeout=5)
        assert gateway.returncode == 0, gateway.stderr
        assert len(json.loads(gateway.stdout)["result"]["tools"]) == 9
        checks += 1
        listing = subprocess.run([sys.executable, str(ROOT / "tools/jfgui-agent.py"), "list"], capture_output=True, text=True, timeout=10)
        assert listing.returncode == 0, listing.stderr
        found = {item["endpoint"] for item in json.loads(listing.stdout)["instances"]}
        assert a in found and b in found
        checks += 1
        # UTF-8 survives transport chunk boundaries and appears only in protocol output.
        with agent.Connection(a) as raw:
            raw.send(br'{"jsonrpc":"2.0","id":"Gr\u00fc\u00dfe","method":"agent/info"}')
            assert json.loads(raw.receive())["id"] == "Grüße"
            checks += 1
        for child in (first, second):
            child.wait(timeout=25)
            assert child.returncode == 0, child.stderr.read().decode(errors="replace")
        checks += 1
        if os.name != "nt":
            assert not Path(a).exists() and not Path(b).exists()
            checks += 1
        print(f"ipc_check: {checks} checks, 0 failures")
    finally:
        for child in (first, second):
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=5)


if __name__ == "__main__":
    main()
