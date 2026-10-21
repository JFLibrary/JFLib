"""Explicit loopback browser bridge for the shared Rust dispatcher (standard library only).

The browser owns the app and its actions. This host only queues bounded JSON-RPC envelopes;
it provides no filesystem, shell or application mutation operation of its own.
"""

from __future__ import annotations

from collections import OrderedDict
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import secrets
import threading
import time
from urllib.parse import parse_qs, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

LIMIT = 1_048_576
MAX_PENDING = 64


def strict_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("Duplicate JSON member")
            result[key] = value
        return result
    def invalid_constant(_):
        raise ValueError("Non-finite JSON number")
    return json.loads(data, object_pairs_hook=pairs, parse_constant=invalid_constant)


def origin_value(origin):
    parts = urlsplit(origin)
    if parts.scheme not in ("http", "https") or not parts.hostname or parts.username or parts.password or parts.path not in ("", "/") or parts.query or parts.fragment:
        raise ValueError("Choose one explicit http(s) browser origin without a path")
    return f"{parts.scheme}://{parts.netloc}"


def rpc_error(request, message, code=-31001):
    return {"jsonrpc": "2.0", "id": request.get("id"), "error": {"code": code, "message": message}}


@dataclass
class Pending:
    identity: str
    session: str
    request: dict
    close_session: bool = False
    delivered: bool = False
    response: object = None
    done: threading.Event = field(default_factory=threading.Event)


class State:
    """A single explicitly attached browser, with independent downstream agent session IDs."""

    def __init__(self, origin, token=None):
        self.origin = origin_value(origin)
        self.token = token or secrets.token_urlsafe(32)
        self.client = None
        self.app = None
        self.pending = OrderedDict()
        self.changed = threading.Condition()
        self.closed = False
        self.last_seen = 0.0

    def connect(self, app):
        if not isinstance(app, str) or not 1 <= len(app) <= 128:
            raise ValueError("app must be a nonempty name up to 128 characters")
        with self.changed:
            if self.closed:
                raise ValueError("Bridge is closed")
            self.expire()
            if self.client is not None:
                raise ValueError("A browser is already attached; disconnect it first")
            self.client = secrets.token_urlsafe(24)
            self.app = app
            self.last_seen = time.monotonic()
            return {"client": self.client, "protocolVersion": "1", "maxMessageBytes": LIMIT}

    def expire(self):
        if self.client is not None and time.monotonic() - self.last_seen > 60:
            self.disconnect_locked("Browser connection expired")

    def require_client(self, client):
        if not isinstance(client, str) or self.client is None or not secrets.compare_digest(client, self.client):
            raise PermissionError("Unknown browser connection")
        self.last_seen = time.monotonic()

    def disconnect_locked(self, message):
        self.client = None
        self.app = None
        for pending in self.pending.values():
            pending.response = rpc_error(pending.request, message)
            pending.done.set()
        self.pending.clear()
        self.changed.notify_all()

    def disconnect(self, client):
        with self.changed:
            self.require_client(client)
            self.disconnect_locked("Browser disconnected")

    def close(self):
        with self.changed:
            self.closed = True
            self.disconnect_locked("Browser bridge closed")

    def submit(self, session, request, timeout=30, *, close_session=False):
        if not isinstance(session, str) or not 1 <= len(session) <= 128:
            raise ValueError("session must be a stable nonempty ID up to 128 characters")
        if not isinstance(request, dict) or request.get("jsonrpc") != "2.0" or not isinstance(request.get("method"), str):
            raise ValueError("request must be a JSON-RPC 2.0 request or notification")
        identity = request.get("id")
        if "id" in request and (isinstance(identity, bool) or not isinstance(identity, (str, int, type(None)))):
            raise ValueError("Invalid JSON-RPC request ID")
        pending = Pending(secrets.token_urlsafe(18), session, request, close_session=close_session)
        with self.changed:
            self.expire()
            if self.closed or self.client is None:
                return rpc_error(request, "No activated browser is attached")
            if len(self.pending) >= MAX_PENDING:
                return rpc_error(request, "Browser request queue is full")
            self.pending[pending.identity] = pending
            self.changed.notify_all()
        if not pending.done.wait(timeout):
            with self.changed:
                self.pending.pop(pending.identity, None)
            return rpc_error(request, "Browser request timed out; inspect state before retrying a mutation")
        return pending.response

    def close_session(self, session):
        result = self.submit(session, {"jsonrpc": "2.0", "method": "notifications/sessionClosed"},
                             timeout=5, close_session=True)
        return {"closed": True} if result is None else result

    def poll(self, client, timeout=20):
        deadline = time.monotonic() + timeout
        with self.changed:
            self.require_client(client)
            while True:
                self.require_client(client)
                waiting = [item for item in self.pending.values() if not item.delivered][:8]
                if waiting:
                    for item in waiting:
                        item.delivered = True
                    return {"requests": [dict(id=item.identity, session=item.session,
                                               **({"closeSession": True} if item.close_session else {"request": item.request}))
                                         for item in waiting]}
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return {"requests": []}
                self.changed.wait(remaining)

    def finish(self, client, identity, response):
        with self.changed:
            self.require_client(client)
            pending = self.pending.get(identity)
            if pending is None or not pending.delivered:
                raise ValueError("Unknown or undelivered request ID")
            if "id" in pending.request:
                if not isinstance(response, dict) or response.get("jsonrpc") != "2.0" or response.get("id") != pending.request["id"] or (("result" in response) == ("error" in response)):
                    raise ValueError("Response must match the delivered JSON-RPC request")
            elif response is not None:
                raise ValueError("A notification must be acknowledged with a null response")
            self.pending.pop(identity)
            pending.response = response
            pending.done.set()


class BridgeServer(ThreadingHTTPServer):
    daemon_threads = True
    block_on_close = False

    def __init__(self, origin, port=0):
        self.state = State(origin)
        self.slots = threading.BoundedSemaphore(16)
        super().__init__(("127.0.0.1", port), Handler)

    def process_request(self, request, client_address):
        request.settimeout(30)
        if not self.slots.acquire(blocking=False):
            try:
                request.sendall(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
            finally:
                self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()

    def config(self):
        return {"url": f"http://127.0.0.1:{self.server_port}", "token": self.state.token, "origin": self.state.origin}

    def server_close(self):
        self.state.close()
        super().server_close()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass  # Capability tokens and document data are never HTTP log messages.

    def send_json(self, code, value):
        payload = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
        if len(payload) > LIMIT:
            code, payload = 413, b'{"error":"Message exceeds 1048576 bytes"}'
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        if self.headers.get("Origin") == self.server.state.origin:
            self.send_header("Access-Control-Allow-Origin", self.server.state.origin)
            self.send_header("Vary", "Origin")
        self.end_headers()
        self.wfile.write(payload)
        self.close_connection = True

    def authorized(self, browser=False):
        origin = self.headers.get("Origin")
        if origin is not None and origin != self.server.state.origin or browser and origin is None:
            self.send_json(403, {"error": "Browser origin is not authorized"})
            return False
        authorization = self.headers.get("Authorization", "")
        if not secrets.compare_digest(authorization, "Bearer " + self.server.state.token):
            self.send_json(403, {"error": "Invalid bridge capability token"})
            return False
        return True

    def do_OPTIONS(self):
        if self.headers.get("Origin") != self.server.state.origin:
            self.send_json(403, {"error": "Browser origin is not authorized"})
            return
        requested = {h.strip().lower() for h in self.headers.get("Access-Control-Request-Headers", "").split(",") if h.strip()}
        if not requested <= {"authorization", "content-type"} or self.headers.get("Access-Control-Request-Method") not in ("GET", "POST"):
            self.send_json(403, {"error": "Unsupported preflight request"})
            return
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", self.server.state.origin)
        self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Private-Network", "true")
        self.send_header("Vary", "Origin")
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True

    def do_GET(self):
        if not self.authorized(browser=True):
            return
        parts = urlsplit(self.path)
        query = parse_qs(parts.query)
        if parts.path != "/poll" or set(query) != {"client"} or len(query["client"]) != 1:
            self.send_json(404, {"error": "Unknown endpoint"})
            return
        try:
            self.send_json(200, self.server.state.poll(query["client"][0]))
        except (ValueError, PermissionError) as error:
            self.send_json(400, {"error": str(error)})

    def do_POST(self):
        parts = urlsplit(self.path)
        if parts.query or parts.fragment or parts.path not in ("/connect", "/rpc", "/result", "/disconnect", "/session/close"):
            self.send_json(404, {"error": "Unknown endpoint"})
            return
        if not self.authorized(browser=parts.path not in ("/rpc", "/session/close")):
            return
        try:
            length = int(self.headers.get("Content-Length", "-1"))
            if not 0 <= length <= LIMIT or self.headers.get("Transfer-Encoding"):
                self.send_json(413, {"error": "A bounded Content-Length is required"})
                return
            data = self.rfile.read(length)
            if len(data) != length:
                raise ValueError("Incomplete request body")
            value = strict_json(data)
            if not isinstance(value, dict):
                raise ValueError("Body must be a JSON object")
            state = self.server.state
            if parts.path == "/connect":
                response = state.connect(value.get("app"))
            elif parts.path == "/disconnect":
                state.disconnect(value.get("client"))
                response = {"disconnected": True}
            elif parts.path == "/result":
                if "response" not in value:
                    raise ValueError("response is required; use null for a notification")
                state.finish(value.get("client"), value.get("id"), value["response"])
                response = {"accepted": True}
            elif parts.path == "/session/close":
                response = state.close_session(value.get("session"))
            else:
                response = state.submit(value.get("session"), value.get("request"))
            self.send_json(200, response)
        except (ValueError, PermissionError, UnicodeError, RecursionError, TypeError) as error:
            self.send_json(400, {"error": str(error)})


def remote_request(url, token, path, value, timeout=35):
    parts = urlsplit(url)
    if parts.scheme != "http" or parts.hostname not in ("127.0.0.1", "localhost") or parts.path not in ("", "/") or parts.username or parts.password or parts.query or parts.fragment:
        raise ValueError("Browser bridge URL must be an explicit loopback HTTP address")
    body = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
    if len(body) > LIMIT:
        raise ValueError("Request exceeds bridge limit")
    request = Request(url.rstrip("/") + path, data=body, headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}, method="POST")
    class NoRedirect(HTTPRedirectHandler):
        def redirect_request(self, *_):
            raise ValueError("A local browser bridge must not redirect capability-bearing requests")
    # Environment proxies and redirects must never receive the local capability token.
    opener = build_opener(ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=timeout) as response:
        body = response.read(LIMIT + 1)
    if len(body) > LIMIT:
        raise ValueError("Response exceeds bridge limit")
    return strict_json(body)


def remote_rpc(url, token, session, request, timeout=35):
    return remote_request(url, token, "/rpc", {"session": session, "request": request}, timeout)


def remote_close(url, token, session):
    return remote_request(url, token, "/session/close", {"session": session}, timeout=7)
