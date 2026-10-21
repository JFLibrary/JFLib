"""notify_smoke.py -- e-mail and web hook notifications of bc-server end to end (standard library).

    python apps/BackupCraft/tests/notify_smoke.py [--server build/win/bc-server.exe]
                                                  [--coretest build/win/coretest.exe] [--keep]

A small SMTP server here (STARTTLS or implicit TLS, TLS 1.3 or 1.2, AUTH PLAIN and / or LOGIN,
messages kept) with an Ed25519 certificate of a test CA (coretest --tls-pki), and a web hook
receiver.  Checked: the password is sealed (never shown again), test messages over STARTTLS +
PLAIN and over TLS 1.2 + LOGIN, the UTF-8 subject and the base64 body, a wrong password and an
untrusted certificate refused with a readable reason, no AUTH without encryption, a failed
backup notifies in "failure" mode while a good one does not, "all" mode, the daily summary on
request, the web hook payload, GET /api/notify and the capability."""
import argparse
import base64
import email
import email.header
import json
import os
import shutil
import socket
import socketserver
import ssl
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port, wait_op  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]


class Smtp:
    """an SMTP server for one test setting"""

    def __init__(self, tls_ctx, implicit=False, auth=("PLAIN", "LOGIN"), user="mailer", password="geheim-1234"):
        self.ctx, self.implicit, self.auth, self.user, self.password = tls_ctx, implicit, auth, user, password
        self.messages = []
        self.events = []
        outer = self

        class H(socketserver.BaseRequestHandler):
            def handle(self):
                outer.session(self.request)

        self.srv = socketserver.ThreadingTCPServer(("127.0.0.1", 0), H)
        self.srv.daemon_threads = True
        self.port = self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()

    def session(self, sock):
        try:
            if self.implicit:
                sock = self.ctx.wrap_socket(sock, server_side=True)
            f = {"sock": sock, "buf": b"", "tls": self.implicit}

            def send(line):
                f["sock"].sendall(line.encode() + b"\r\n")

            def line():
                while b"\r\n" not in f["buf"]:
                    d = f["sock"].recv(4096)
                    if not d:
                        raise EOFError
                    f["buf"] += d
                l, f["buf"] = f["buf"].split(b"\r\n", 1)
                return l.decode("utf-8", "replace")

            send("220 smoke.example ESMTP")
            authed, mail_from, rcpts = False, None, []
            while True:
                cmd = line()
                up = cmd.upper()
                self.events.append(cmd if not up.startswith("AUTH PLAIN ") else "AUTH PLAIN ***")
                if up.startswith("EHLO"):
                    caps = ["smoke.example", "SIZE 10485760", "8BITMIME"]
                    if not f["tls"]:
                        caps.append("STARTTLS")
                    if f["tls"]:
                        caps.append("AUTH " + " ".join(self.auth))
                    for c in caps[:-1]:
                        send("250-" + c)
                    send("250 " + caps[-1])
                elif up == "STARTTLS" and not f["tls"]:
                    send("220 go ahead")
                    f["sock"] = self.ctx.wrap_socket(f["sock"], server_side=True)
                    f["tls"], f["buf"] = True, b""
                elif up.startswith("AUTH PLAIN ") and "PLAIN" in self.auth:
                    _, u, p = base64.b64decode(cmd[11:]).split(b"\0")
                    authed = (u.decode(), p.decode()) == (self.user, self.password)
                    send("235 ok" if authed else "535 5.7.8 authentication failed")
                elif up == "AUTH LOGIN" and "LOGIN" in self.auth:
                    send("334 VXNlcm5hbWU6")
                    u = base64.b64decode(line()).decode()
                    send("334 UGFzc3dvcmQ6")
                    p = base64.b64decode(line()).decode()
                    authed = (u, p) == (self.user, self.password)
                    send("235 ok" if authed else "535 5.7.8 authentication failed")
                elif up.startswith("MAIL FROM:"):
                    if self.user and not authed:
                        send("530 5.7.0 authentication required")
                        continue
                    mail_from, rcpts = cmd[10:].strip("<> "), []
                    send("250 ok")
                elif up.startswith("RCPT TO:"):
                    rcpts.append(cmd[8:].strip("<> "))
                    send("250 ok")
                elif up == "DATA":
                    send("354 end with .")
                    data = []
                    while True:
                        l = line()
                        if l == ".":
                            break
                        data.append(l[1:] if l.startswith("..") else l)
                    self.messages.append((mail_from, rcpts, "\r\n".join(data), f["sock"].version() if f["tls"] else "plain"))
                    send("250 queued")
                elif up == "QUIT":
                    send("221 bye")
                    return
                else:
                    send("502 unknown")
        except (EOFError, OSError, ssl.SSLError):
            return


class Hook:
    def __init__(self):
        self.posts = []
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_POST(self):
                n = int(self.headers.get("Content-Length", "0"))
                outer.posts.append(json.loads(self.rfile.read(n)))
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"ok")

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()


def subject_of(raw):
    m = email.message_from_string(raw)
    return str(email.header.make_header(email.header.decode_header(m["Subject"]))), m


def body_of(m):
    for part in m.walk():
        if part.get_content_type() == "text/plain":
            return part.get_payload(decode=True).decode("utf-8")
    return ""


def wait_for(cond, timeout=30):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.1)
    return False


def main():
    ap = argparse.ArgumentParser()
    plat = "win" if os.name == "nt" else "linux"
    ext = ".exe" if os.name == "nt" else ""
    ap.add_argument("--server", default=str(ROOT / "build" / plat / ("bc-server" + ext)))
    ap.add_argument("--coretest", default=str(ROOT / "build" / plat / ("coretest" + ext)))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-notify-"))
    pki = tmp / "pki"
    pki.mkdir()
    subprocess.run([a.coretest, "--tls-pki", str(pki)], check=True)
    ctx13 = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx13.load_cert_chain(pki / "server.pem", pki / "server.key")
    ctx12 = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx12.load_cert_chain(pki / "server.pem", pki / "server.key")
    ctx12.maximum_version = ssl.TLSVersion.TLSv1_2
    ca_pem = (pki / "ca.pem").read_text()
    data = tmp / "data"
    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv = subprocess.Popen([a.server, "--data", str(data), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    hook = Hook()
    smtps = []
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    def settings(**kv):
        api.ok("PUT", "/api/settings", [{"key": k.replace("_", ".", 1), "value": v} for k, v in kv.items()])

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (data / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": "correct horse battery staple", "server_name": "notify-srv"}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": "correct horse battery staple"}, token=None)["token"]
        caps = {c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}
        check(caps.get("notifications") is True, f"notifications available: {caps}")

        # STARTTLS (TLS 1.3) with AUTH PLAIN; the password is sealed and never shown again
        s1 = Smtp(ctx13)
        smtps.append(s1)
        settings(smtp_host=f"127.0.0.1:{s1.port}", smtp_security="starttls", smtp_user="mailer", smtp_password="geheim-1234",
                 smtp_from="backup@example.org", smtp_to="it@example.org, chef@example.org", smtp_ca=ca_pem,
                 notify_webhook=f"http://127.0.0.1:{hook.port}/hook")
        shown = {s["key"]: s["value"] for s in api.ok("GET", "/api/settings")}
        check(shown["smtp.password"] == "********", "password masked")
        check(all("geheim-1234" not in p.read_bytes().decode("latin-1") for p in (data / "db").iterdir() if p.name != "LOCK"),
              "password not in the database in clear")
        r = api.ok("POST", "/api/settings/test-email", {})
        check("E-Mail an it@example.org, chef@example.org gesendet" in r["message"] and "Web-Hook: 200" in r["message"], f"test sent: {r}")
        check(len(s1.messages) == 1, "one message")
        frm, rcpts, raw, ver = s1.messages[0]
        subj, m = subject_of(raw)
        check(frm == "backup@example.org" and rcpts == ["it@example.org", "chef@example.org"], f"envelope: {frm} {rcpts}")
        check(subj == "[BackupCraft] Testnachricht von notify-srv" and ver == "TLSv1.3", f"subject / TLS: {subj} {ver}")
        check("Die Benachrichtigungen von BackupCraft kommen an." in body_of(m), "body")
        check(any(e == "STARTTLS" for e in s1.events) and "AUTH PLAIN ***" in s1.events, f"STARTTLS + PLAIN: {s1.events}")
        check(hook.posts and hook.posts[-1]["event"] == "test" and "Testnachricht" in hook.posts[-1]["text"], f"web hook: {hook.posts}")
        st = api.ok("GET", "/api/notify")
        check(st["last_ok"] is True and st["last_what"] == "Test", f"status: {st}")
        # a saved placeholder keeps the password
        settings(smtp_password="********")
        api.ok("POST", "/api/settings/test-email", {})
        check(len(s1.messages) == 2, "placeholder keeps the password")

        # wrong password, untrusted certificate: refused with the reason
        settings(smtp_password="falsch-falsch")
        st_, e = api.call("POST", "/api/settings/test-email", {})
        check(st_ == 502 and "Anmeldung abgelehnt" in e["message"] and "535" in e["message"], f"wrong password: {e}")
        settings(smtp_password="geheim-1234", smtp_ca="")
        st_, e = api.call("POST", "/api/settings/test-email", {})
        check(st_ == 502 and "TLS" in e["message"] and "not trusted" in e["message"], f"untrusted certificate: {e}")
        check(api.ok("GET", "/api/notify")["last_ok"] is False, "failure recorded")
        settings(smtp_ca=ca_pem)

        # implicit TLS, TLS 1.2, AUTH LOGIN only
        s2 = Smtp(ctx12, implicit=True, auth=("LOGIN",))
        smtps.append(s2)
        settings(smtp_host=f"127.0.0.1:{s2.port}", smtp_security="tls", notify_webhook="")
        api.ok("POST", "/api/settings/test-email", {})
        check(len(s2.messages) == 1 and s2.messages[0][3] == "TLSv1.2" and "AUTH LOGIN" in s2.events, f"TLS 1.2 + LOGIN: {s2.events}")

        # no encryption: no authentication
        s3 = Smtp(None, auth=())
        smtps.append(s3)
        settings(smtp_host=f"127.0.0.1:{s3.port}", smtp_security="none")
        st_, e = api.call("POST", "/api/settings/test-email", {})
        check(st_ == 502 and "verschlüsselte Verbindung" in e["message"], f"no AUTH in clear: {e}")
        settings(smtp_user="")
        s3.user = ""
        api.ok("POST", "/api/settings/test-email", {})
        check(len(s3.messages) == 1 and s3.messages[0][3] == "plain", "relay without login")

        # sessions: a failed backup notifies in "failure" mode, a good one does not
        settings(smtp_host=f"127.0.0.1:{s1.port}", smtp_security="starttls", smtp_user="mailer", smtp_password="geheim-1234",
                 notify_mode="failure", notify_webhook=f"http://127.0.0.1:{hook.port}/hook")
        server_host = [h for h in api.ok("GET", "/api/hosts") if h["kind"] == "server"][0]
        src = tmp / "src"
        src.mkdir()
        (src / "a.txt").write_text("Daten " * 1000)
        gone = tmp / "gone"
        gone.mkdir()
        api.ok("PUT", "/api/hosts/" + server_host["id"], {"name": "", "roots": [str(tmp)], "restore_roots": []})
        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        base = {"name": "Gut", "description": "", "kind": "file", "enabled": True, "host_id": server_host["id"], "sources": [str(src)],
                "excludes": [], "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
                "retention": {"points": 3, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False, "passphrase": "",
                "key_hint": "", "compression": "zstd", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": ""}
        good = api.ok("POST", "/api/jobs", base, 201)
        bad = api.ok("POST", "/api/jobs", dict(base, name="Schlecht", sources=[str(gone)]), 201)
        gone.rmdir()
        before = len(s1.messages)
        wait_op(api, api.ok("POST", f"/api/jobs/{good['id']}/run", {"full": False}, 202)["operation_id"])
        wait_op(api, api.ok("POST", f"/api/jobs/{bad['id']}/run", {"full": False}, 202)["operation_id"], success=False)
        check(wait_for(lambda: len(s1.messages) > before), "failure notified")
        time.sleep(1.5)
        check(len(s1.messages) == before + 1, "success not notified in failure mode")
        subj, m = subject_of(s1.messages[-1][2])
        check(subj == "[BackupCraft] Fehlgeschlagen: Sicherung „Schlecht“", f"failure subject: {subj}")
        body = body_of(m)
        check("Keine der Quellen war lesbar" in body and "Warnungen und Fehler:" in body, f"failure body: {body[:400]}")
        check(any(p.get("event") == "session" and p.get("status") == "failed" and p.get("job") == "Schlecht" for p in hook.posts), "session web hook")
        html = [p for p in m.walk() if p.get_content_type() == "text/html"]
        check(html and "Fehlgeschlagen" in html[0].get_payload(decode=True).decode("utf-8"), "HTML part")
        # "all": the good one too
        settings(notify_mode="all")
        n = len(s1.messages)
        wait_op(api, api.ok("POST", f"/api/jobs/{good['id']}/run", {"full": False}, 202)["operation_id"])
        check(wait_for(lambda: len(s1.messages) > n), "success notified in all mode")
        check(subject_of(s1.messages[-1][2])[0] == "[BackupCraft] Erfolgreich: Sicherung „Gut“", "success subject")
        # the daily summary on request
        r = api.ok("POST", "/api/notify/summary", {})
        subj, m = subject_of(s1.messages[-1][2])
        check(subj.startswith("[BackupCraft] Zusammenfassung: 2 erfolgreich, 0 Warnungen, 1 Fehler"), f"summary: {subj}")
        check("Sicherung „Schlecht“" in body_of(m), "summary lists the failure")
        # settings checks
        st_, e = api.call("PUT", "/api/settings", [{"key": "notify.daily", "value": "25:00"}])
        check(st_ == 400 and e["field"] == "notify.daily", "daily time checked")
        st_, e = api.call("PUT", "/api/settings", [{"key": "smtp.ca", "value": "kein Zertifikat"}])
        check(st_ == 400 and e["field"] == "smtp.ca", "CA checked")
        audit = {x["action"] for x in api.ok("GET", "/api/audit?limit=200")}
        check("notify.test" in audit and "notify.summary" in audit, "audited")
        print(f"notify_smoke: {checks} checks passed")
        return 0
    except AssertionError as e:
        print("FAIL", e)
        log.flush()
        print((tmp / "server.log").read_text()[-3000:])
        return 1
    finally:
        srv.terminate()
        try:
            srv.wait(10)
        except subprocess.TimeoutExpired:
            srv.kill()
        log.close()
        for s in smtps:
            s.close()
        if a.keep:
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


def _up(api):
    try:
        api.ok("GET", "/api")
        return True
    except OSError:
        return False


if __name__ == "__main__":
    sys.exit(main())
