"""vm_smoke.py -- QEMU/KVM in bc-server end to end (Linux with qemu-system-x86_64, qemu-img,
nasm; /dev/kvm when there, else slower without).

    python3 apps/BackupCraft/tests/vm_smoke.py [--server build/linux/bc-server] [--keep]
    (from Windows: wsl.exe -d Ubuntu-24.04 -- python3 apps/BackupCraft/tests/vm_smoke.py)

The guest is tests/guest/boot.asm: a boot sector that counts its boots on its disk and prints
"BACKUPCRAFT-GUEST-OK boot=NNNN".  Two VMs: vm1 (qcow2, running) and vm2 (raw, stopped after it
was seen).  Checked: the hypervisor and its VMs, a VM job (vm1 by QEMU with a persistent dirty
bitmap, vm2 from its file), the restored disks byte for byte, an incremental run after a
reboot of vm1 (one changed block), an instant start from the point (it boots and counts on,
the point stays as it was), merging a running instant VM into a folder, boot checks (passes,
and fails on a text the guest never prints), the rules."""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from http_smoke import Api, free_port, wait_op  # noqa: E402
from notify_smoke import _up, wait_for  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
PW = "correct horse battery staple"
MB = 1 << 20


def qmp(path, *cmds):
    """run QMP commands on a socket, close"""
    s = socket.socket(socket.AF_UNIX)
    s.connect(path)
    f = s.makefile("rwb")
    f.readline()
    out = []
    for c in (("qmp_capabilities", {}),) + cmds:
        f.write(json.dumps({"execute": c[0], "arguments": c[1]}).encode() + b"\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if "event" not in m:
                out.append(m)
                break
    s.close()
    return out[1:]


def main():
    if os.name == "nt":
        print("vm_smoke runs on Linux (wsl.exe -d Ubuntu-24.04 -- python3 ...)")
        return 0
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default=str(ROOT / "build" / "linux" / "bc-server"))
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    tmp = Path(tempfile.mkdtemp(prefix="bc-vm-"))
    qdir = tmp / "qmp"
    qdir.mkdir()
    subprocess.run(["nasm", "-f", "bin", "-o", str(tmp / "boot.bin"), str(Path(__file__).parent / "guest" / "boot.asm")], check=True)
    raw = bytearray(64 * MB)
    raw[:512] = (tmp / "boot.bin").read_bytes()
    (tmp / "vm1.raw").write_bytes(raw)
    subprocess.run(["qemu-img", "convert", "-q", "-f", "raw", "-O", "qcow2", str(tmp / "vm1.raw"), str(tmp / "vm1.qcow2")], check=True)
    (tmp / "vm1.raw").unlink()
    raw[4096:4096 + 9] = b"VM2-DATEN"
    (tmp / "vm2.raw").write_bytes(raw[:32 * MB])
    kvm = os.access("/dev/kvm", os.W_OK)
    vms = {}

    def start_vm(name, image, fmt):
        args = ["qemu-system-x86_64", "-name", name, "-m", "64", "-display", "none", "-nic", "none",
                "-blockdev", f"driver={fmt},node-name=disk0,file.driver=file,file.filename={image}", "-device", "ide-hd,drive=disk0",
                "-qmp", f"unix:{qdir}/{name}.qmp,server=on,wait=off", "-serial", f"file:{tmp}/{name}.serial"]
        if kvm:
            args.insert(1, "-enable-kvm")
        vms[name] = subprocess.Popen(args)
        if not wait_for(lambda: (qdir / f"{name}.qmp").exists(), 15):
            raise AssertionError(f"{name} did not start")

    def serial(name):
        p = tmp / f"{name}.serial"
        return p.read_text(errors="replace") if p.exists() else ""

    def disk_now(image, fmt):
        out = tmp / "now.raw"
        subprocess.run(["qemu-img", "convert", "-q", "-U", "-f", fmt, "-O", "raw", str(image), str(out)], check=True)
        b = out.read_bytes()
        out.unlink()
        return b

    def counter(b):
        return int.from_bytes(b[512:514], "little")

    port = free_port()
    log = open(tmp / "server.log", "w+")
    srv = subprocess.Popen([a.server, "--data", str(tmp / "data"), "--listen", f"127.0.0.1:{port}"], stdout=log, stderr=log)
    api = Api(f"http://127.0.0.1:{port}")
    checks = 0

    def check(cond, what):
        nonlocal checks
        if not cond:
            raise AssertionError(what)
        checks += 1

    try:
        if not wait_for(lambda: srv.poll() is None and _up(api), 15):
            raise AssertionError("server did not start")
        token = (tmp / "data" / "setup.token").read_text().strip()
        api.ok("POST", "/api/setup", {"token": token, "username": "admin", "password": PW}, 201)
        api.token = api.ok("POST", "/api/auth/login", {"username": "admin", "password": PW}, token=None)["token"]
        caps = {c["name"]: c["available"] for c in api.ok("GET", "/api/capabilities")}
        check(caps.get("hypervisor") and caps.get("instant-start") and caps.get("boot-check"), f"capabilities: {caps}")
        api.ok("PUT", "/api/settings", [{"key": "hypervisor.qmp", "value": str(qdir)}, {"key": "restore.allowed_roots", "value": str(tmp / "restore")}])
        start_vm("vm1", tmp / "vm1.qcow2", "qcow2")
        start_vm("vm2", tmp / "vm2.raw", "raw")
        check(wait_for(lambda: "boot=0001" in serial("vm1") and "boot=0001" in serial("vm2"), 30), f"guests boot: {serial('vm1')!r}")
        hv = api.ok("GET", "/api/hypervisors")[0]
        check(hv["ok"] and hv["vms"] == 2 and hv["running"] == 2 and hv["kvm"] == kvm, f"hypervisor: {hv}")
        vml = {v["name"]: v for v in api.ok("GET", "/api/vms")}
        v1 = vml["vm1"]
        check(v1["state"] == "running" and v1["disks"][0]["format"] == "qcow2" and v1["disks"][0]["size"] == 64 * MB and v1["disks"][0]["bus"] == "ide", f"vm1: {v1}")
        # vm2 stops: known with its disk, backed up from the file
        qmp(str(qdir / "vm2.qmp"), ("quit", {}))
        vms["vm2"].wait(10)
        time.sleep(11)
        vml = {v["name"]: v for v in api.ok("GET", "/api/vms")}
        check(vml["vm2"]["state"] == "stopped" and vml["vm2"]["disks"][0]["file"].endswith("vm2.raw"), f"vm2 stopped: {vml['vm2']}")

        repo = api.ok("POST", "/api/repositories", {"name": "R", "path": str(tmp / "repo"), "create": True, "immutable_days": 0}, 201)
        job = {"name": "VMs", "description": "", "kind": "qemu", "enabled": True, "host_id": "", "sources": [v1["id"], vml["vm2"]["id"]], "excludes": [],
               "repository_id": repo["id"], "schedule": {"cron": "", "tz": "UTC", "enabled": False},
               "retention": {"points": 5, "days": 0, "weekly": 0, "monthly": 0, "yearly": 0}, "encrypted": False, "passphrase": "", "key_hint": "",
               "compression": "lz4", "active_full_days": 0, "pre_script": "", "post_script": "", "source_job": "", "pool_id": "",
               "cdp": False, "cdp_rpo": 60, "cdp_keep_hours": 0}
        st, e = api.call("POST", "/api/jobs", dict(job, sources=["host:gibt-es-nicht"]))
        check(st == 400 and e["field"] == "sources", "unknown VMs refused")
        j = api.ok("POST", "/api/jobs", job, 201)
        o = wait_op(api, api.ok("POST", f"/api/jobs/{j['id']}/run", {"full": False}, 202)["operation_id"])
        p1 = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]
        lines = [l["msg"] for l in api.ok("GET", f"/api/sessions/{o['session_id']}/log")]
        check(any("Vollsicherung durch QEMU" in l for l in lines) and any("aus den Plattendateien" in l for l in lines), f"full run: {lines[:6]}")
        roots = {e["name"] for e in api.ok("GET", f"/api/points/{p1}/browse?path=")["entries"]}
        check(roots == {"vm1", "vm2"}, f"roots: {roots} {lines}")
        names = {e["name"] for e in api.ok("GET", f"/api/points/{p1}/browse?path=vm1")["entries"]}
        check(names == {"vm.json", "disk0.raw"}, f"files: {names}")
        wait_op(api, api.ok("POST", f"/api/points/{p1}/restore", {"items": [], "original": False, "target": str(tmp / "restore" / "p1"), "conflict": "skip"}, 202)["operation_id"])
        d1 = (tmp / "restore" / "p1" / "vm1" / "disk0.raw").read_bytes()
        check(d1 == disk_now(tmp / "vm1.qcow2", "qcow2") and counter(d1) == 1, f"vm1 disk restored (count {counter(d1)})")
        check((tmp / "restore" / "p1" / "vm2" / "disk0.raw").read_bytes() == (tmp / "vm2.raw").read_bytes(), "vm2 disk restored")
        cfg = json.loads((tmp / "restore" / "p1" / "vm1" / "vm.json").read_text())
        check(cfg["memory_mb"] == 64 and cfg["disks"][0]["node"] == "disk0", f"vm.json: {cfg}")

        # a reboot changes one block: the next run is incremental
        qmp(str(qdir / "vm1.qmp"), ("system_reset", {}))
        check(wait_for(lambda: "boot=0002" in serial("vm1"), 30), "vm1 rebooted")
        o = wait_op(api, api.ok("POST", f"/api/jobs/{j['id']}/run", {"full": False}, 202)["operation_id"])
        p2 = api.ok("GET", f"/api/sessions/{o['session_id']}")["point_id"]
        lines = [l["msg"] for l in api.ok("GET", f"/api/sessions/{o['session_id']}/log")]
        check(any("inkrementell" in l for l in lines) and any("1 von 64 Blöcken geändert" in l for l in lines), f"incremental: {[l for l in lines if 'vm1' in l]}")
        wait_op(api, api.ok("POST", f"/api/points/{p2}/restore", {"items": ["vm1"], "original": False, "target": str(tmp / "restore" / "p2"), "conflict": "skip"}, 202)["operation_id"])
        d2 = (tmp / "restore" / "p2" / "vm1" / "disk0.raw").read_bytes()
        check(d2 == disk_now(tmp / "vm1.qcow2", "qcow2") and counter(d2) == 2, f"incremental disk (count {counter(d2)})")

        # another program holds vm1's QMP socket: it stays "running", its backup waits for nothing
        holder = socket.socket(socket.AF_UNIX)
        holder.connect(str(qdir / "vm1.qmp"))
        holder.recv(4096)
        time.sleep(11)
        check({v["name"]: v["state"] for v in api.ok("GET", "/api/vms")}["vm1"] == "running", "a busy socket keeps the VM running")
        o = wait_op(api, api.ok("POST", f"/api/jobs/{j['id']}/run", {"full": False}, 202)["operation_id"])
        lines = [l["msg"] for l in api.ok("GET", f"/api/sessions/{o['session_id']}/log")]
        check(any("vm1" in l and "belegt" in l for l in lines) and not any("vm1: läuft nicht" in l for l in lines), f"busy socket: {lines}")
        holder.close()
        time.sleep(1)

        # instant start from the second point
        st, e = api.call("POST", f"/api/points/{p2}/instant-start", {"vm": "vm9", "memory_mb": 0, "network": "none"})
        check(st == 400 and e["field"] == "vm", "only VMs of the point")
        o = wait_op(api, api.ok("POST", f"/api/points/{p2}/instant-start", {"vm": "vm1", "memory_mb": 0, "network": "none"}, 202)["operation_id"])
        sess = [s for s in api.ok("GET", "/api/instant-start") if s["purpose"] == "instant"][0]
        check(o["status"] == "succeeded" and sess["state"] == "running" and sess["vm"] == "vm1", f"instant: {sess}")

        def console(sid):
            return api.ok("GET", f"/api/instant-start/{sid}/console")["text"]
        check(wait_for(lambda: "boot=0003" in console(sess["id"]), 30), f"boots from the point: {console(sess['id'])!r}")
        api.ok("POST", f"/api/instant-start/{sess['id']}/stop", {})
        check([s for s in api.ok("GET", "/api/instant-start") if s["id"] == sess["id"]][0]["state"] == "stopped", "stopped")
        wait_op(api, api.ok("POST", f"/api/points/{p2}/restore", {"items": ["vm1"], "original": False, "target": str(tmp / "restore" / "p2b"), "conflict": "skip"}, 202)["operation_id"])
        check(counter((tmp / "restore" / "p2b" / "vm1" / "disk0.raw").read_bytes()) == 2, "the point stays as it was")

        # merge a running instant VM into a folder
        o = wait_op(api, api.ok("POST", f"/api/points/{p2}/instant-start", {"vm": "vm1", "memory_mb": 128, "network": "isolated"}, 202)["operation_id"])
        sess = [s for s in api.ok("GET", "/api/instant-start") if s["state"] == "running"][0]
        check(sess["memory_mb"] == 128 and sess["network"] == "isolated", "memory and network as asked")
        check(wait_for(lambda: "boot=0003" in console(sess["id"]), 30), "second instant boot")
        st, e = api.call("POST", f"/api/instant-start/{sess['id']}/migrate", {"target": "/somewhere/else"})
        check(st == 403, "merge only into restore folders")
        o = wait_op(api, api.ok("POST", f"/api/instant-start/{sess['id']}/migrate", {"target": str(tmp / "restore" / "merged")}, 202)["operation_id"])
        merged = tmp / "restore" / "merged" / "vm1-disk0.qcow2"
        cur = [s for s in api.ok("GET", "/api/instant-start") if s["id"] == sess["id"]][0]
        check(o["status"] == "succeeded" and cur["state"] == "migrated" and merged.exists(), f"merged: {cur}")
        check(counter(disk_now(merged, "qcow2")) == 3, "the merged disk has the instant VM's writes")
        api.ok("POST", f"/api/instant-start/{sess['id']}/stop", {})
        check(merged.exists(), "the merged disk stays after the stop")

        # boot checks
        o = wait_op(api, api.ok("POST", f"/api/points/{p2}/boot-check", {"vm": "vm1", "boot_seconds": 3, "console_text": "BACKUPCRAFT-GUEST-OK", "tcp_port": 0, "timeout_s": 60}, 202)["operation_id"])
        res = api.ok("GET", "/api/boot-checks")[0]
        check(o["status"] == "succeeded" and res["ok"] and [c["name"] for c in res["checks"]] == ["Start", "Konsole"] and "boot=0003" in res["console"], f"boot check: {res}")
        o = wait_op(api, api.ok("POST", f"/api/points/{p2}/boot-check", {"vm": "vm1", "boot_seconds": 2, "console_text": "GIBT-ES-NICHT", "tcp_port": 0, "timeout_s": 8}, 202)["operation_id"], success=False)
        res = api.ok("GET", "/api/boot-checks")[0]
        check(not res["ok"] and res["checks"][0]["ok"] and not res["checks"][1]["ok"], f"the boot check finds the missing text: {res['checks']}")
        check(not [s for s in api.ok("GET", "/api/instant-start") if s["state"] == "running"], "nothing left running")
        acts = {e["action"] for e in api.ok("GET", "/api/audit?limit=200")}
        check({"instant.start", "instant.stop", "instant.migrate", "bootcheck"} <= acts, f"audit: {sorted(acts)}")
        print(f"vm_smoke: {checks} checks passed ({'KVM' if kvm else 'ohne KVM'})")
        return 0
    except AssertionError as e:
        print("FAIL", e)
        log.flush()
        print((tmp / "server.log").read_text()[-3000:])
        return 1
    finally:
        for p in vms.values():
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(10)
                except subprocess.TimeoutExpired:
                    p.kill()
        srv.terminate()
        try:
            srv.wait(10)
        except subprocess.TimeoutExpired:
            srv.kill()
        log.close()
        if a.keep:
            print("kept", tmp)
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
