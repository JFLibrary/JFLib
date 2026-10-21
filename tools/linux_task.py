"""Small Linux runner used natively or through WSL; no shell interpolation."""
import argparse
import gzip
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--watch-stdin', action='store_true')
    p.add_argument('--cwd')
    p.add_argument('action', choices=['run', 'deb', 'repo', 'probe', 'appimage', 'rpm', 'flatpak'])
    p.add_argument('args', nargs=argparse.REMAINDER)
    a = p.parse_args()
    if a.cwd:
        os.chdir(a.cwd)
    if a.watch_stdin:
        child = subprocess.Popen([sys.executable, __file__, a.action, *a.args],
                                 stdin=subprocess.DEVNULL, start_new_session=True)
        def watch():
            # A daemon must not hold BufferedReader's lock during interpreter shutdown.
            # The raw descriptor still provides EOF-based owner-lifetime monitoring.
            while os.read(sys.stdin.fileno(), 4096):
                pass
            if child.poll() is not None:
                return
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        threading.Thread(target=watch, daemon=True).start()
        result = child.wait()
        sys.stdout.flush()
        sys.stderr.flush()
        os._exit(result)
    if a.action == 'run':
        return subprocess.call(a.args)
    if a.action == 'repo':
        return repo(Path(a.args[0]), a.args[1], a.args[2] if len(a.args) > 2 else '')
    if a.action == 'probe':
        return probe()
    if a.action == 'appimage':
        return appimage(Path(a.args[0]), Path(a.args[1]), Path(a.args[2]), a.args[3:])
    if a.action == 'rpm':
        return rpm(Path(a.args[0]), Path(a.args[1]), Path(a.args[2]))
    if a.action == 'flatpak':
        return flatpak(Path(a.args[0]), a.args[1], Path(a.args[2]))
    tree, destination = map(Path, a.args)
    with tempfile.TemporaryDirectory(prefix='jfgui-deb-') as tmp:
        package = Path(tmp) / 'pkg'
        shutil.copytree(tree, package)
        for entry in [package, *package.rglob('*')]:
            executable = entry.is_dir() or (entry.parent.name == 'DEBIAN' and entry.name in
                          ('config', 'postinst', 'prerm', 'postrm')) or '/usr/lib/' in entry.as_posix()
            entry.chmod(0o755 if executable else 0o644)
        output = Path(tmp) / 'out.deb'
        subprocess.run(['dpkg-deb', '--root-owner-group', '-Zxz', '--build', str(package), str(output)], check=True)
        shutil.copyfile(output, destination)
    return 0


TOOLS = ('mksquashfs', 'unsquashfs', 'appimagetool', 'rpmbuild', 'rpm', 'flatpak', 'flatpak-builder',
         'desktop-file-validate', 'appstreamcli', 'dpkg-deb', 'bwrap')


def probe():
    """the packaging tools of tools/pkg_linux.py as JSON: {tool: path or ""}, "flatpak_refs":
    the installed runtimes and SDKs ("org.freedesktop.Platform/26.08")"""
    out = {t: shutil.which(t) or '' for t in TOOLS}
    refs = []
    if out['flatpak']:
        r = subprocess.run(['flatpak', 'list', '--runtime', '--columns=application,branch'],
                           capture_output=True, text=True)
        refs = sorted({'/'.join(line.split()[:2]) for line in r.stdout.splitlines() if len(line.split()) >= 2})
    out['flatpak_refs'] = refs
    print(json.dumps(out))
    return 0


def fix_modes(root):
    """folders 0755, programs (ELF, #!) 0755, other files 0644; links stay"""
    for entry in [root, *root.rglob('*')]:
        if entry.is_symlink():
            continue
        if entry.is_dir():
            entry.chmod(0o755)
            continue
        with open(entry, 'rb') as f:
            head = f.read(4)
        entry.chmod(0o755 if head == b'\x7fELF' or head[:2] == b'#!' else 0o644)


def appimage(tree, runtime, destination, links):
    """an AppImage from the AppDir tree (copied into the Linux file system, modes fixed, the
    links "path=target" made): appimagetool --runtime-file RUNTIME when it is in PATH, else
    RUNTIME + mksquashfs -comp zstd (SOURCE_DATE_EPOCH: reproducible)"""
    with tempfile.TemporaryDirectory(prefix='jfgui-appimage-') as tmp:
        appdir = Path(tmp) / 'AppDir'
        shutil.copytree(tree, appdir, symlinks=True)
        fix_modes(appdir)
        for link in links:
            path, target = link.split('=', 1)
            (appdir / path).parent.mkdir(parents=True, exist_ok=True)
            if (appdir / path).is_symlink() or (appdir / path).exists():
                (appdir / path).unlink()
            os.symlink(target, appdir / path)
        image = Path(tmp) / 'out.AppImage'
        tool = shutil.which('appimagetool')
        if tool:
            subprocess.run([tool, '--runtime-file', str(runtime), str(appdir), str(image)], check=True,
                           env=dict(os.environ, ARCH='x86_64'))
        else:
            squash = Path(tmp) / 'image.squashfs'
            subprocess.run(['mksquashfs', str(appdir), str(squash), '-all-root', '-noappend', '-comp', 'zstd', '-quiet',
                            '-no-progress'], check=True)
            with open(image, 'wb') as out:
                out.write(Path(runtime).read_bytes())
                out.write(squash.read_bytes())
        image.chmod(0o755)
        shutil.copyfile(image, destination)
    return 0


def rpm(spec, source, outdir):
    """rpmbuild -bb in a fresh _topdir: the .rpm files go to outdir, their names to stdout"""
    with tempfile.TemporaryDirectory(prefix='jfgui-rpm-') as tmp:
        top = Path(tmp)
        for d in ('SPECS', 'SOURCES', 'BUILD', 'RPMS', 'SRPMS', 'BUILDROOT'):
            (top / d).mkdir()
        shutil.copyfile(spec, top / 'SPECS' / spec.name)
        shutil.copyfile(source, top / 'SOURCES' / source.name)
        subprocess.run(['rpmbuild', '-bb', '--define', f'_topdir {top}', str(top / 'SPECS' / spec.name)], check=True)
        for f in sorted((top / 'RPMS').rglob('*.rpm')):
            shutil.copyfile(f, outdir / f.name)
            print(f.name)
    return 0


def flatpak(src, app_id, bundle):
    """flatpak-builder on the manifest src/<app_id>.json (copied into the Linux file system),
    exported into a temporary repository, then a single-file bundle"""
    with tempfile.TemporaryDirectory(prefix='jfgui-flatpak-') as tmp:
        work = Path(tmp)
        shutil.copytree(src, work / 'src')
        subprocess.run(['flatpak-builder', '--disable-rofiles-fuse', '--force-clean', '--disable-updates',
                        f'--state-dir={work / "state"}', f'--repo={work / "repo"}', str(work / 'build'),
                        str(work / 'src' / f'{app_id}.json')], check=True)
        subprocess.run(['flatpak', 'build-bundle', str(work / 'repo'), str(work / 'out.flatpak'), app_id], check=True)
        shutil.copyfile(work / 'out.flatpak', bundle)
    return 0


def repo(out, origin, key):
    """a flat apt repository in out (the .deb files in out/pool): Packages(.gz) and Release
    from apt-ftparchive; with a key (jfgui_GPG_HOME, jfgui_GPG_PASSPHRASE from the environment)
    also InRelease, Release.gpg and the public key repo.gpg (for signed-by=)"""
    os.chdir(out)
    packages = subprocess.run(['apt-ftparchive', 'packages', 'pool'], check=True, capture_output=True).stdout
    Path('Packages').write_bytes(packages)
    with gzip.GzipFile('Packages.gz', 'wb', mtime=0) as f:
        f.write(packages)
    for name in ('Release', 'InRelease', 'Release.gpg', 'repo.gpg'):
        Path(name).unlink(missing_ok=True)
    release = subprocess.run(['apt-ftparchive', '-o', f'APT::FTPArchive::Release::Origin={origin}',
                              '-o', f'APT::FTPArchive::Release::Label={origin}', 'release', '.'],
                             check=True, capture_output=True).stdout
    Path('Release').write_bytes(release)
    if not key:
        return 0
    gpg = ['gpg', '--batch', '--yes']
    if os.environ.get('jfgui_GPG_HOME'):
        gpg += ['--homedir', os.environ['jfgui_GPG_HOME']]
    pw = os.environ.get('jfgui_GPG_PASSPHRASE')
    sign = gpg + ['--local-user', key, '--digest-algo', 'SHA512']
    if pw:
        sign += ['--pinentry-mode', 'loopback', '--passphrase-fd', '0']
    data = pw.encode() if pw else None
    subprocess.run(sign + ['--clearsign', '-o', 'InRelease', 'Release'], input=data, check=True)
    subprocess.run(sign + ['--armor', '--detach-sign', '-o', 'Release.gpg', 'Release'], input=data, check=True)
    exported = subprocess.run(gpg + ['--export', key], check=True, capture_output=True).stdout
    if not exported:
        print(f'repo: gpg has no public key {key}', file=sys.stderr)
        return 1
    Path('repo.gpg').write_bytes(exported)
    return 0


if __name__ == '__main__':
    sys.exit(main())
