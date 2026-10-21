#!/usr/bin/env python3
"""Control explicitly enabled jfguiLib instances over private local IPC.

Only Python's standard library is used. Applications execute all semantic actions through
the shared Rust dispatcher; this program implements discovery, framing and CLI arguments.
It never evaluates shell text. Run `python tools/jfgui-agent.py --help` from JFLib.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import secrets
import socket
import stat
import subprocess
import sys
import time

LIMIT = 1_048_576
MODERN = "2026-07-28"
PIPE_PREFIX = "\\\\.\\pipe\\jfgui-agent-"
TOOLS = {
    "actions/list": "jfgui.actions.search",
    "actions/describe": "jfgui.actions.describe",
    "actions/execute": "jfgui.actions.execute",
    "actions/batch": "jfgui.actions.batch",
    "state/inspect": "jfgui.state.inspect",
    "tasks/get": "jfgui.tasks.status",
    "tasks/cancel": "jfgui.tasks.cancel",
    "resources/read": "jfgui.resources.read",
    "view/render": "jfgui.view.render",
}


def metadata() -> dict:
    return {
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "jfgui-agent", "version": "1"},
    }


def protocol(address: str) -> str:
    return "mcp" if re.search(r"-mcp-\d+-\d+(?:\.sock)?$", address) else "json"


def validate_address(address: str) -> None:
    if os.name == "nt":
        if not address.startswith(PIPE_PREFIX) or not re.fullmatch(r"[A-Za-z0-9_-]+", address[len(PIPE_PREFIX):]):
            raise ValueError("Expected a local jfguiLib named-pipe address")
    else:
        directory = Path(f"/tmp/jfgui-agent-{os.geteuid()}")
        path = Path(address)
        if path.parent != directory or not re.fullmatch(r"[A-Za-z0-9_-]+\.sock", path.name):
            raise ValueError("Expected an jfguiLib socket in the current user's private directory")
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
            raise PermissionError("The agent directory is not private to the current user")
        info = path.lstat()
        if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.geteuid():
            raise PermissionError("The endpoint is not a socket owned by the current user")


class Connection:
    """Bounded line I/O over either a current-user Unix socket or a local named pipe."""

    def __init__(self, address: str, timeout: float = 5):
        validate_address(address)
        self.address = address
        self.timeout = timeout
        self.pending = bytearray()
        self.pipe = None
        self.sock = None
        if os.name != "nt":
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.sock.settimeout(timeout)
            self.sock.connect(address)
            if hasattr(socket, "SO_PEERCRED"):
                import struct
                _, uid, _ = struct.unpack("3i", self.sock.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
                if uid != os.geteuid():
                    self.close()
                    raise PermissionError("Server peer has a different user identity")
            return
        from ctypes import wintypes as w
        self.k = ctypes.WinDLL("kernel32", use_last_error=True)
        self.k.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID, w.DWORD, w.DWORD, w.HANDLE]
        self.k.CreateFileW.restype = w.HANDLE
        self.k.CloseHandle.argtypes = [w.HANDLE]
        self.k.SetNamedPipeHandleState.argtypes = [w.HANDLE, ctypes.POINTER(w.DWORD), w.LPVOID, w.LPVOID]
        self.k.ReadFile.argtypes = [w.HANDLE, w.LPVOID, w.DWORD, ctypes.POINTER(w.DWORD), w.LPVOID]
        self.k.WriteFile.argtypes = [w.HANDLE, w.LPCVOID, w.DWORD, ctypes.POINTER(w.DWORD), w.LPVOID]
        deadline = time.monotonic() + timeout
        while True:
            pipe = self.k.CreateFileW(address, 0xC0000000, 0, None, 3, 0, None)
            if pipe != ctypes.c_void_p(-1).value:
                self.pipe = pipe
                break
            error = ctypes.get_last_error()
            if error not in (2, 231) or time.monotonic() >= deadline:
                raise OSError(error, "Cannot connect to the enabled instance", address)
            time.sleep(0.01)
        mode = w.DWORD(1)  # PIPE_NOWAIT, byte mode
        if not self.k.SetNamedPipeHandleState(self.pipe, ctypes.byref(mode), None, None):
            error = ctypes.get_last_error()
            self.close()
            raise OSError(error, "Cannot enable bounded nonblocking pipe I/O")

    def close(self):
        if self.sock is not None:
            self.sock.close()
            self.sock = None
        if self.pipe is not None:
            self.k.CloseHandle(self.pipe)
            self.pipe = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def send(self, line: bytes):
        if len(line) > LIMIT + 1 or b"\n" in line.rstrip(b"\r\n"):
            raise ValueError("One request must be one JSON line at most 1048576 bytes")
        if not line.endswith(b"\n"):
            line += b"\n"
        if self.sock is not None:
            self.sock.sendall(line)
            return
        from ctypes import wintypes as w
        offset = 0
        deadline = time.monotonic() + self.timeout
        while offset < len(line):
            count = w.DWORD()
            part = line[offset:offset + 65536]
            ok = self.k.WriteFile(self.pipe, part, len(part), ctypes.byref(count), None)
            if not ok and ctypes.get_last_error() not in (232, 536):
                raise OSError(ctypes.get_last_error(), "Named-pipe write failed")
            offset += count.value
            if offset < len(line) and count.value == 0:
                if time.monotonic() >= deadline:
                    raise TimeoutError("Agent write timed out")
                time.sleep(0.002)

    def receive(self) -> bytes:
        deadline = time.monotonic() + self.timeout
        while b"\n" not in self.pending:
            if len(self.pending) > LIMIT:
                raise ValueError("Response exceeds control protocol limit")
            if self.sock is not None:
                data = self.sock.recv(16384)
                if not data:
                    raise EOFError("Instance closed the control connection")
            else:
                from ctypes import wintypes as w
                buffer = ctypes.create_string_buffer(16384)
                count = w.DWORD()
                ok = self.k.ReadFile(self.pipe, buffer, len(buffer), ctypes.byref(count), None)
                if not ok:
                    error = ctypes.get_last_error()
                    if error in (109, 233):
                        raise EOFError("Instance closed the control connection")
                    if error not in (232, 536):
                        raise OSError(error, "Named-pipe read failed")
                data = buffer.raw[:count.value]
            self.pending.extend(data)
            if not data:
                if time.monotonic() >= deadline:
                    raise TimeoutError("Agent response timed out")
                time.sleep(0.002)
        line, _, remainder = self.pending.partition(b"\n")
        if len(line) > LIMIT:
            raise ValueError("Response exceeds control protocol limit")
        self.pending = bytearray(remainder)
        return bytes(line)

    def request(self, method: str, params: dict, *, request_id=1):
        wire_method = method
        wire_params = params
        if protocol(self.address) == "mcp":
            if method == "agent/info":
                wire_method, wire_params = "server/discover", {"_meta": metadata()}
            else:
                wire_method, wire_params = "tools/call", {"name": TOOLS[method], "arguments": params, "_meta": metadata()}
        payload = {"jsonrpc": "2.0", "id": request_id, "method": wire_method, "params": wire_params}
        self.send(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))
        response = json.loads(self.receive())
        if response.get("id") != request_id:
            raise ValueError("Mismatched response identifier")
        if wire_method == "tools/call" and "result" in response:
            result = response["result"].get("structuredContent", {})
            if result.get("ok"):
                return {"jsonrpc": "2.0", "id": request_id, "result": result["data"]}
            return {"jsonrpc": "2.0", "id": request_id, "error": result.get("error", {"message": "MCP tool failed"})}
        return response


def addresses():
    if os.name == "nt":
        return sorted("\\\\.\\pipe\\" + name for name in os.listdir("\\\\.\\pipe\\") if name.startswith("jfgui-agent-"))
    directory = Path(f"/tmp/jfgui-agent-{os.geteuid()}")
    if not directory.exists():
        return []
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise PermissionError("Agent discovery directory is not private")
    return sorted(str(path) for path in directory.glob("*.sock") if path.is_socket())


def emit(value):
    sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def json_input(args, default=None):
    if getattr(args, "input_file", None):
        value = json.loads(Path(args.input_file).read_text(encoding="utf-8"))
    elif getattr(args, "input_json", None):
        value = json.loads(args.input_json)
    else:
        value = {} if default is None else default
    return value


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", newline="\n")
        sys.stderr.reconfigure(encoding="utf-8", newline="\n")
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("list", help="List only reachable, explicitly activated instances")
    for name in ("actions", "describe", "call", "inspect", "batch", "task", "resource", "render", "stdio"):
        item = sub.add_parser(name)
        item.add_argument("endpoint", help="Exact endpoint returned by list or shown by the app")
        item.add_argument("--timeout", type=float, default=5)
        if name in ("describe", "call"):
            item.add_argument("action")
        if name == "actions":
            item.add_argument("--query", default="")
            item.add_argument("--cursor")
            item.add_argument("--limit", type=int, default=30)
        if name in ("call", "inspect", "batch", "render"):
            group = item.add_mutually_exclusive_group()
            group.add_argument("--input-json")
            group.add_argument("--input-file")
        if name == "call":
            item.add_argument("--expected-revision", type=int)
        if name == "batch":
            item.add_argument("--atomic", action="store_true")
            item.add_argument("--continue-on-error", action="store_true")
        if name == "task":
            item.add_argument("id", type=int)
            item.add_argument("--cancel", action="store_true")
        if name == "resource":
            item.add_argument("uri")
            item.add_argument("--offset", type=int, default=0)
            item.add_argument("--limit", type=int, default=32768)
        if name == "stdio":
            item.add_argument("--protocol", choices=("json", "mcp"), default="mcp", help="The native endpoint selects MCP from the first explicit version envelope")
    headless = sub.add_parser("headless", help="Start an jfguiLib app's explicit stdio control mode")
    headless.add_argument("app", type=Path)
    headless.add_argument("--protocol", choices=("json", "mcp"), default="json")
    bridge = sub.add_parser("browser-bridge", help="Explicitly start a loopback bridge for one chosen browser origin")
    bridge.add_argument("--origin", required=True)
    bridge.add_argument("--port", type=int, default=0)
    for name in ("browser-stdio", "browser-call"):
        item = sub.add_parser(name)
        item.add_argument("--url", required=True)
        item.add_argument("--token", required=True)
        item.add_argument("--session", default=None)
        if name == "browser-call":
            item.add_argument("method")
            item.add_argument("--params-json", default="{}")
    args, extra_args = parser.parse_known_args()
    if extra_args and (args.command != "headless" or extra_args[0] != "--"):
        parser.error("Extra application arguments must follow -- in headless mode")
    if args.command == "browser-bridge":
        from agent_bridge import BridgeServer
        if not 0 <= args.port <= 65535:
            raise ValueError("Port must be 0 to 65535; zero allocates an available loopback port")
        server = BridgeServer(args.origin, args.port)
        print(json.dumps(server.config(), separators=(",", ":")), file=sys.stderr, flush=True)
        try:
            server.serve_forever(poll_interval=0.2)
        except KeyboardInterrupt:
            pass
        finally:
            server.server_close()
        return 0
    if args.command in ("browser-stdio", "browser-call"):
        from agent_bridge import remote_rpc, remote_close, strict_json
        if args.command == "browser-call":
            request = {"jsonrpc": "2.0", "id": 1, "method": args.method, "params": strict_json(args.params_json)}
            session = args.session or ("cli-mcp" if "_meta" in request["params"] or args.method == "initialize" else "cli-json")
            response = remote_rpc(args.url, args.token, session, request)
            emit(response)
            return 1 if "error" in response else 0
        session = args.session or secrets.token_urlsafe(18)
        try:
            while True:
                line = sys.stdin.buffer.readline(LIMIT + 2)
                if not line:
                    return 0
                if len(line) > LIMIT + 1:
                    raise ValueError("Request exceeds bridge limit")
                if not line.strip():
                    continue
                request = strict_json(line)
                response = remote_rpc(args.url, args.token, session, request)
                if "id" in request:
                    emit(response)
        finally:
            try:
                remote_close(args.url, args.token, session)
            except (OSError, ValueError, EOFError):
                pass  # The browser may already have revoked control or disconnected.
    if args.command == "list":
        instances = []
        for address in addresses():
            try:
                with Connection(address, timeout=0.5) as connection:
                    response = connection.request("agent/info", {})
                if "result" in response:
                    instances.append({"endpoint": address, "protocol": protocol(address), "info": response["result"]})
            except (OSError, ValueError, EOFError, KeyError):
                pass  # Stale or inaccessible endpoints are never advertised as active.
        emit({"instances": instances})
        return 0
    if args.command == "headless":
        app = args.app.resolve(strict=True)
        if not app.is_file():
            raise ValueError("The app path must name an executable file")
        extra = extra_args[1:]
        # Argument arrays only; no shell/interpreter command string is evaluated.
        # Explicitly inherit all three handles. With CREATE_NO_WINDOW on Windows,
        # implicit inheritance loses redirected standard streams at this second hop.
        return subprocess.call([str(app), "--agent-" + args.protocol, *extra],
                               stdin=sys.stdin.buffer, stdout=sys.stdout.buffer,
                               stderr=sys.stderr.buffer,
                               creationflags=0x08000000 if os.name == "nt" else 0)
    with Connection(args.endpoint, args.timeout) as connection:
        if args.command == "stdio":
            # Protocol-preserving bridge; the first explicit MCP version envelope selects
            # the native Rust MCP adapter for this connection, including -json- endpoints.
            # Notifications have no response. Requests are sequential and retain their IDs.
            while True:
                line = sys.stdin.buffer.readline(LIMIT + 2)
                if not line:
                    return 0
                if not line.strip():
                    continue
                payload = json.loads(line)
                if args.protocol == "mcp" and payload.get("method") == "agent/info":
                    raise ValueError("The bridge expects MCP requests; select --protocol json for JSON control")
                connection.send(line)
                if "id" in payload:
                    sys.stdout.buffer.write(connection.receive() + b"\n")
                    sys.stdout.buffer.flush()
        if args.command == "actions":
            method, params = "actions/list", {"query": args.query, "limit": args.limit}
            if args.cursor is not None:
                params["cursor"] = args.cursor
        elif args.command == "describe":
            method, params = "actions/describe", {"action": args.action}
        elif args.command == "call":
            method, params = "actions/execute", {"action": args.action, "input": json_input(args)}
            if args.expected_revision is not None:
                params["expectedRevision"] = args.expected_revision
        elif args.command == "inspect":
            method, params = "state/inspect", json_input(args)
        elif args.command == "batch":
            method, params = "actions/batch", {"calls": json_input(args, []), "atomic": args.atomic, "stopOnError": not args.continue_on_error}
        elif args.command == "task":
            method, params = "tasks/cancel" if args.cancel else "tasks/get", {"id": args.id}
        elif args.command == "resource":
            method, params = "resources/read", {"uri": args.uri, "offset": args.offset, "limit": args.limit}
        else:
            method, params = "view/render", json_input(args)
        response = connection.request(method, params)
        emit(response)
        return 1 if "error" in response else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, EOFError, KeyError) as exc:
        print(f"jfgui-agent: {exc}", file=sys.stderr)
        raise SystemExit(2)
