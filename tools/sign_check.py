"""sign_check.py -- the signing paths of tools/package.py without real certificates
(docs/PACKAGING.md, "Signing")

    python tools/sign_check.py [--android] [--keep]

  ed25519     tools/update_manifest.py against the RFC 8032 vectors; keygen, sign, verify of a
              channel folder and a changed file found
  setup       (Windows) a setup with payload and an Authenticode certificate table at its end
              (the layout signtool writes) installs and uninstalls: the installer finds its
              payload before the signature, uninstall.exe carries no dangling signature
  signtool    (Windows, signtool and openssl in WSL) a self-signed code-signing certificate made
              for this run (never trusted, never installed): codesign.Signer signs the setup for
              real, it still installs; signtool verify finds the signature (the chain ends in an
              untrusted root: expected)
  report      package.py's Signing without certificates says what is missing; --sign fails
  apt         (WSL: gpg, apt-ftparchive) a throwaway GnuPG key in /tmp signs a flat repository
              with a test .deb: gpgv accepts InRelease and Release.gpg with repo.gpg, refuses a
              changed Release
  android     the release key from the environment: a clear error without it, a keystore made
              with keytool for this run is accepted; --android also builds androidGallery with
              --release and checks with apksigner that the APK carries the test certificate

Work files: build/sign_check/ (removed unless --keep).  Prints PASS / FAIL / SKIP; exit code 1
on failures.  Nothing here touches the certificate stores or the user's GnuPG keys.
"""
import argparse
import base64
import os
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
import android  # noqa: E402
import codesign  # noqa: E402
import package  # noqa: E402
import update_manifest as um  # noqa: E402
from tools.host_exec import linux_path, run_linux  # noqa: E402

WORK = ROOT / "build" / "sign_check"
FAILS = []


def check(ok, what):
    print(("PASS " if ok else "FAIL ") + what)
    if not ok:
        FAILS.append(what)
    return ok


def skip(what, why):
    print(f"SKIP {what}: {why}")


def t_ed25519():
    seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
    sig = um.sign(seed, b"")
    check(um.public_key(seed).hex() == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
          and sig.hex().startswith("e5564300c360ac729086e2cc806e828a") and um.verify(um.public_key(seed), b"", sig)
          and not um.verify(um.public_key(seed), b"x", sig), "ed25519: RFC 8032 test 1")
    key = WORK / "test.updkey"
    pub = um.keygen(key)
    check(um.load_key(key) and um.public_key(um.load_key(key)) == pub, "ed25519: keygen writes a key that loads again")
    try:
        um.keygen(key)
        check(False, "ed25519: keygen never overwrites a key")
    except SystemExit:
        check(True, "ed25519: keygen never overwrites a key")
    (WORK / "payload.bin").write_bytes(os.urandom(5000))
    ch = WORK / "channel"
    um.write_channel(ch, "signcheck", "1.2.0", [("windows-x86_64", "exe", WORK / "payload.bin", "app.exe")], um.load_key(key))
    check(um.check_channel(ch, pub) == [], "ed25519: a written channel verifies")
    (ch / "app.exe").write_bytes(os.urandom(5000))
    check(any("SHA-256" in p for p in um.check_channel(ch, pub)), "ed25519: a changed file is found")
    check(um.check_channel(ch, um.public_key(os.urandom(32))) == ["the signature does not match the key"], "ed25519: another key is refused")


def make_setup(style_exe):
    """a setup: the installer stub + a payload with one style (style_exe) + trailer"""
    stub = (ROOT / "build" / "win" / "installer.exe").read_bytes()
    man = b"app=signcheck\ndisplay=Signcheck\nversion=1.0.0\npublisher=JFLib\ndefault=winui\ndesc_winui=Test\n"
    entries = [(package.PK_MANIFEST, package.PF_ZLIB, "manifest", "", zlib.compress(man, 9), len(man)),
               (package.PK_STYLE, package.PF_ZLIB, "winui", "Test", zlib.compress(style_exe, 9), len(style_exe))]
    pay = package.payload(entries)
    return stub, stub + pay + b"ASMINST1" + struct.pack("<QQ", len(stub), len(pay))


def fake_signature(data):
    """the layout signtool writes: padding to 8 bytes, a WIN_CERTIFICATE, the security
    directory pointing at it (the content is no real PKCS #7: only the installer reads it)"""
    pad = (-len(data)) % 8
    cert = struct.pack("<IHH", 8 + 40, 0x0200, 0x0002) + b"\x30" * 40
    b = bytearray(data + b"\0" * pad + cert)
    struct.pack_into("<II", b, codesign.security_entry(b), len(data) + pad, len(cert))
    return bytes(b)


def install(setup, name, stub, style_exe):
    """install setup with a script into WORK/<name>/inst, check the files, uninstall"""
    d = WORK / name
    shutil.rmtree(d, ignore_errors=True)
    (d / "shell").mkdir(parents=True)
    (d / "inst.uis").write_text('click "Weiter"\nclick "Installieren"\nwait 500\nexpect "Fertigstellen" exists\n', encoding="utf-8")
    (d / "un.uis").write_text('click "Deinstallieren"\nwait 500\nexpect "Schließen" exists\n', encoding="utf-8")
    exe = d / "Signcheck_Setup.exe"
    exe.write_bytes(setup)
    env = dict(os.environ, ASMINST_SHELL_DIR=str(d / "shell"))
    r = subprocess.run([str(exe), "--script", str(d / "inst.uis"), "--dir", str(d / "inst")], env=env,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    if not check(r.returncode == 0, f"{name}: the setup installs"):
        print(r.stdout + r.stderr)
        return
    check((d / "inst" / "signcheck.exe").read_bytes() == style_exe, f"{name}: the program is the payload's")
    un = (d / "inst" / "uninstall.exe").read_bytes()
    e = codesign.security_entry(un)
    check(len(un) == len(stub) and e is not None and un[e:e + 8] == b"\0" * 8,
          f"{name}: uninstall.exe is the stub without a signature entry")
    r = subprocess.run([str(d / "inst" / "uninstall.exe"), "--uninstall", "--script", str(d / "un.uis")], env=env,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    check(r.returncode == 0 and not (d / "inst" / "signcheck.exe").exists(), f"{name}: uninstall.exe removes it again")


def t_setup():
    if os.name != "nt":
        return skip("setup", "Windows only")
    if not (ROOT / "build" / "win" / "installer.exe").exists():
        subprocess.run([sys.executable, "build_rs.py", "app", "installer", "--target", "win"], cwd=ROOT, check=True,
                       capture_output=True)
    style_exe = (ROOT / "build" / "win" / "installer.exe").read_bytes()[:4096] + os.urandom(1000)
    stub, setup = make_setup(style_exe)
    install(setup, "setup-plain", stub, style_exe)
    signed = fake_signature(setup)
    check(codesign.pe_signature_bytes(signed) is not None, "setup: the stand-in signature has signtool's layout")
    install(signed, "setup-signed", stub, style_exe)
    return stub, setup, style_exe


def t_signtool(made):
    if os.name != "nt" or not made:
        return skip("signtool", "Windows only")
    tool = codesign.find_signtool()
    if tool is None:
        return skip("signtool", "signtool.exe not found (Windows SDK)")
    d = WORK / "cert"
    d.mkdir(parents=True, exist_ok=True)
    pw = base64.b16encode(os.urandom(8)).decode()
    r = run_linux(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", linux_path(d / "key.pem"),
                   "-out", linux_path(d / "cert.pem"), "-days", "2", "-subj", "/CN=JFLib Signing Test (untrusted)",
                   "-addext", "extendedKeyUsage=codeSigning", "-addext", "keyUsage=critical,digitalSignature"])
    r2 = r.returncode == 0 and run_linux(["openssl", "pkcs12", "-export", "-out", linux_path(d / "test.pfx"),
                                          "-inkey", linux_path(d / "key.pem"), "-in", linux_path(d / "cert.pem"),
                                          "-passout", f"pass:{pw}"]).returncode == 0
    if not r2:
        return skip("signtool", "no openssl in WSL for the test certificate")
    stub, setup, style_exe = made
    env = {k: os.environ.get(k) for k in ("jfgui_SIGN_PFX", "jfgui_SIGN_PFX_PASS", "jfgui_SIGN_THUMBPRINT", "jfgui_SIGN_TIMESTAMP")}
    os.environ.update(jfgui_SIGN_PFX=str(d / "test.pfx"), jfgui_SIGN_PFX_PASS=pw)
    for k in ("jfgui_SIGN_THUMBPRINT", "jfgui_SIGN_TIMESTAMP"):
        os.environ.pop(k, None)
    try:
        signer, why = codesign.authenticode()
        if not check(signer is not None, f"signtool: a .pfx from the environment is used ({why or signer.what})"):
            return
        f = WORK / "Signcheck_Setup.exe"
        f.write_bytes(setup)
        try:
            signer.sign([f], "Signcheck Setup")
            check(True, "signtool: the setup is signed")
        except RuntimeError as e:
            check(False, f"signtool: the setup is signed ({e})")
            return
        check(codesign.pe_signature(f) is not None, "signtool: the certificate table is at the end")
        install(f.read_bytes(), "setup-signtool", stub, style_exe)
        v = subprocess.run([str(tool), "verify", "/pa", "/v", str(f)], capture_output=True, text=True, encoding="utf-8", errors="replace")
        out = v.stdout + v.stderr
        check("Issued to: JFLib Signing Test (untrusted)" in out and "No signature found" not in out,
              "signtool: verify reads the signature and its certificate (the test root is untrusted: expected)")
    finally:
        for k, v in env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def t_report():
    saved = {k: os.environ.pop(k, None) for k in ("jfgui_SIGN_PFX", "jfgui_SIGN_THUMBPRINT", "jfgui_UPDATE_KEY", "jfgui_UPDATE_KEY_FILE")}
    try:
        s = package.Signing(False, True)
        check(any("Authenticode" in w and ("jfgui_SIGN_PFX" in w or "signtool" in w) for w in s.skipped),
              "report: without a certificate the Authenticode gap is named")
        try:
            package.Signing(True, True)
            check(False, "report: --sign fails without a certificate")
        except SystemExit as e:
            check("jfgui_SIGN_PFX" in str(e) or "signtool" in str(e), "report: --sign fails without a certificate")
        s = package.Signing(False, False)
        package.update_channel("signcheck", {"version": "1.0.0"}, ["winui"], WORK, WORK / "chan", None, "", None, s, "rs")
        check(any("jfgui_UPDATE_KEY" in w for w in s.skipped), "report: without an update key the channel is skipped and named")
    finally:
        for k, v in saved.items():
            if v is not None:
                os.environ[k] = v


def t_apt():
    if run_linux(["sh", "-c", "command -v gpg && command -v apt-ftparchive && command -v gpgv"]).returncode:
        return skip("apt", "gpg / apt-ftparchive missing in Linux")
    home = f"/tmp/jfgui-signcheck-{os.getpid()}"
    run_linux(["mkdir", "-p", "-m", "700", home])
    try:
        r = run_linux(["gpg", "--homedir", home, "--batch", "--passphrase", "", "--quick-gen-key",
                       "JFLib Repo Test <repo-test@localhost>", "ed25519", "sign", "1d"])
        out = run_linux(["gpg", "--homedir", home, "--list-secret-keys", "--with-colons"]).stdout
        fpr = next((line.split(":")[9] for line in out.splitlines() if line.startswith("fpr:")), "")
        if not check(r.returncode == 0 and len(fpr) == 40, "apt: a throwaway GnuPG key in /tmp"):
            return
        tree = WORK / "debtree" / "signcheck"
        shutil.rmtree(tree.parent, ignore_errors=True)
        (tree / "DEBIAN").mkdir(parents=True)
        (tree / "usr" / "share" / "doc" / "signcheck").mkdir(parents=True)
        (tree / "DEBIAN" / "control").write_bytes(b"Package: signcheck\nVersion: 1.0.0\nArchitecture: all\n"
                                                   b"Maintainer: JFLib Test <test@localhost>\nDescription: sign_check test package\n")
        (tree / "usr" / "share" / "doc" / "signcheck" / "README").write_bytes(b"test\n")
        deb = WORK / "signcheck_1.0.0_all.deb"
        r = run_linux(["python3", linux_path(ROOT / "tools" / "linux_task.py"), "deb", linux_path(tree), linux_path(deb)])
        if not check(r.returncode == 0 and deb.exists(), "apt: a test .deb"):
            print(r.stdout + r.stderr)
            return
        repo = WORK / "repo"
        shutil.rmtree(repo, ignore_errors=True)
        saved = {k: os.environ.get(k) for k in ("jfgui_GPG_KEY", "jfgui_GPG_HOME", "jfgui_GPG_PASSPHRASE")}
        os.environ.update(jfgui_GPG_KEY=fpr, jfgui_GPG_HOME=home)
        os.environ.pop("jfgui_GPG_PASSPHRASE", None)
        try:
            why = codesign.apt_repo([deb], repo, "JFLib Test")
        finally:
            for k, v in saved.items():
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v
        check(why is None and all((repo / n).exists() for n in ("Packages", "Packages.gz", "Release", "InRelease", "Release.gpg", "repo.gpg")),
              "apt: Packages, Release, InRelease, Release.gpg, repo.gpg")
        check(b"Filename: pool/signcheck_1.0.0_all.deb" in (repo / "Packages").read_bytes(), "apt: Packages lists the .deb")
        kr = linux_path(repo / "repo.gpg")
        check(run_linux(["gpgv", "--keyring", kr, linux_path(repo / "InRelease")]).returncode == 0, "apt: gpgv accepts InRelease")
        check(run_linux(["gpgv", "--keyring", kr, linux_path(repo / "Release.gpg"), linux_path(repo / "Release")]).returncode == 0,
              "apt: gpgv accepts Release.gpg")
        changed = WORK / "Release.changed"
        changed.write_bytes((repo / "Release").read_bytes() + b"X")
        check(run_linux(["gpgv", "--keyring", kr, linux_path(repo / "Release.gpg"), linux_path(changed)]).returncode != 0,
              "apt: gpgv refuses a changed Release")
        saved = os.environ.pop("jfgui_GPG_KEY", None)
        try:
            why = codesign.apt_repo([deb], WORK / "repo-unsigned")
        finally:
            if saved is not None:
                os.environ["jfgui_GPG_KEY"] = saved
        check(why is not None and "jfgui_GPG_KEY" in why and not (WORK / "repo-unsigned" / "InRelease").exists(),
              "apt: without jfgui_GPG_KEY the repository is unsigned and that is said")
    finally:
        run_linux(["gpgconf", "--homedir", home, "--kill", "all"])
        run_linux(["rm", "-rf", home])


def t_android(full):
    saved = {k: os.environ.pop(k, None) for k in ("jfgui_ANDROID_KEYSTORE", "jfgui_ANDROID_KEYSTORE_PASS",
                                                   "jfgui_ANDROID_KEY_ALIAS", "jfgui_ANDROID_KEY_PASS")}
    try:
        try:
            android.release_key()
            check(False, "android: --release without a key fails")
        except RuntimeError as e:
            check(all(n in str(e) for n in ("jfgui_ANDROID_KEYSTORE", "jfgui_ANDROID_KEYSTORE_PASS", "jfgui_ANDROID_KEY_ALIAS")),
                  "android: --release without a key fails and names the variables")
        keytool = shutil.which("keytool")
        if os.environ.get("JAVA_HOME") and (Path(os.environ["JAVA_HOME"]) / "bin" / "keytool.exe").exists():
            keytool = str(Path(os.environ["JAVA_HOME"]) / "bin" / "keytool.exe")
        if not keytool:
            return skip("android keystore", "keytool missing (JDK)")
        ks = WORK / "release-test.jks"
        ks.unlink(missing_ok=True)
        pw = base64.b16encode(os.urandom(8)).decode()
        r = subprocess.run([keytool, "-genkeypair", "-keystore", str(ks), "-storepass", pw, "-keypass", pw, "-alias", "jfgui",
                            "-dname", "CN=JFLib Release Test", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2"],
                           capture_output=True, text=True)
        if not check(r.returncode == 0, "android: a test keystore"):
            return
        os.environ.update(jfgui_ANDROID_KEYSTORE=str(ks), jfgui_ANDROID_KEYSTORE_PASS=pw, jfgui_ANDROID_KEY_ALIAS="jfgui")
        check(android.release_key() == (ks, "jfgui"), "android: the keystore from the environment is taken")
        if not full:
            return skip("android build", "with --android: androidGallery --release, apksigner --print-certs")
        out = WORK / "android"
        r = subprocess.run([sys.executable, "build_rs.py", "app", "androidGallery", "--target", "android", "--release", "--out", str(out)],
                           cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
        apk = next(out.rglob("androidGallery-*-release.apk"), None)
        if not check(r.returncode == 0 and apk is not None, "android: androidGallery --release builds a -release.apk"):
            print(r.stdout[-3000:] + r.stderr[-3000:])
            return
        _, _, sdkbuild, _, java, suffix = android.sdk_tools()[1:]
        v = subprocess.run([str(java / ("java" + suffix)), "-jar", str(sdkbuild / "lib" / "apksigner.jar"), "verify", "--print-certs", str(apk)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
        check(v.returncode == 0 and "CN=JFLib Release Test" in v.stdout, "android: the APK carries the release test certificate")
    finally:
        for k in ("jfgui_ANDROID_KEYSTORE", "jfgui_ANDROID_KEYSTORE_PASS", "jfgui_ANDROID_KEY_ALIAS", "jfgui_ANDROID_KEY_PASS"):
            os.environ.pop(k, None)
        for k, v in saved.items():
            if v is not None:
                os.environ[k] = v


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--android", action="store_true", help="also build androidGallery with --release (slow)")
    ap.add_argument("--keep", action="store_true", help="keep build/sign_check")
    a = ap.parse_args()
    for s in (sys.stdout, sys.stderr):
        s.reconfigure(encoding="utf-8", errors="replace")
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    t_ed25519()
    made = t_setup()
    t_signtool(made)
    t_report()
    t_apt()
    t_android(a.android)
    if not a.keep and not FAILS:
        shutil.rmtree(WORK, ignore_errors=True)
    print("sign_check:", "all passed" if not FAILS else f"{len(FAILS)} FAILED")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
