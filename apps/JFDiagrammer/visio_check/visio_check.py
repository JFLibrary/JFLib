r"""visio_check.py -- JFDiagrammer's files checked in a real Visio on another Windows machine, over SSH.

    python apps/JFDiagrammer/tools/visio_check.py [options] FILE...

Each run copies FILE... to the Visio machine, opens every Visio file there in an invisible Visio
(visio_worker.vbs, started in the logged-on user's session by visio_run.ps1) and brings back into
--out (default: a folder "visio-out" next to the first file):

    report.txt           per file: opened or not, REPAIRED (Visio would only open it with a repair),
                         "changed on open", pages with sizes, masters of a stencil, data recordsets,
                         errors of exports and saves, the check script's lines
    BASE-pNN.png / .svg  every page as Visio draws it (--pages png,svg / none)
    BASE-shapes.tsv      every shape (subshapes four levels deep): page, id, parent, NameU, master,
                         type, 1-D, PinX, PinY, Width, Height, Angle (internal units, inches and
                         radians), geometry sections, text
    BASE-visio.EXT       the file saved again by Visio (--save same: .vsdx/.vssx/.vstx; --save old:
                         .vsd/.vss/.vst of Visio 2003-2010)
    BASE.pdf             --pdf

--script CHECK.vbs adds a check of your own: VBScript run in the same Visio, which may define
    Sub Check(doc, base)   called for each file after the generic part (doc is open read-only)
    Sub AfterAll()         called once at the end
and use the worker's helpers: L "text" (a line in report.txt), OpenCopy(name) (a file of the job
opened read-write as a copy in out\), ExportPages doc, base, "png", SaveCopy doc, "name.vsdx",
CellText(shape, "PinX") ("formula = value"), Num(x), app (Visio.Application), inDir, outDir.
Other files given on the command line (an .xlsx a drawing links to, ...) go along into the same
folder, so a check script can use them.

The machine: --host USER@HOST or JFD_VISIO_HOST, --key or JFD_VISIO_KEY (an SSH key the machine
takes without a password).  It needs Visio, a logged-on user (Visio runs in that session, invisibly)
and an account allowed to create its own scheduled tasks.  Nothing of the machine is in the repo.
Several runs may go at once: each has its own folder, task and Visio.

Visio driven this way repairs a broken file without a word (a page cut in half opens with the shapes
it could read, a missing page part as an empty page).  So for the XML formats (.vsdx .vssx .vstx and
the macro ones) this script counts what the file holds itself -- pages, top-level shapes per page,
masters -- and reports every difference to what Visio read as MISMATCH (exit code 1).
"""
import argparse
import datetime
import os
import posixpath
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REMOTE = 'jfd-visio-check'          # under the remote user's profile
NS_V = '{http://schemas.microsoft.com/office/visio/2012/main}'
NS_R = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'
NS_PR = '{http://schemas.openxmlformats.org/package/2006/relationships}'


def counts(path):
    """what an XML Visio file holds -> {'pages': [top-level shapes per page], 'masters': n}, None when it is
    not one this script reads (binary formats, a broken package: Visio's report says what became of it)"""
    try:
        z = zipfile.ZipFile(path)
        names = set(z.namelist())

        def rels(part):
            d, f = posixpath.split(part)
            rp = posixpath.join(d, '_rels', f + '.rels')
            if rp not in names:
                return {}
            return {r.get('Id'): posixpath.normpath(posixpath.join(d, r.get('Target')))
                    for r in ET.fromstring(z.read(rp)).iter(NS_PR + 'Relationship')}
        pages = []
        if 'visio/pages/pages.xml' in names:
            prel = rels('visio/pages/pages.xml')
            for p in ET.fromstring(z.read('visio/pages/pages.xml')).iter(NS_V + 'Page'):
                rel = p.find(NS_V + 'Rel')
                part = prel.get(rel.get(NS_R + 'id')) if rel is not None else None
                if part not in names:
                    pages.append(None)          # the page's part is missing
                    continue
                shapes = ET.fromstring(z.read(part)).find(NS_V + 'Shapes')
                pages.append(0 if shapes is None else len(shapes.findall(NS_V + 'Shape')))
        masters = 0
        if 'visio/masters/masters.xml' in names:
            masters = len(ET.fromstring(z.read('visio/masters/masters.xml')).findall(NS_V + 'Master'))
        return {'pages': pages, 'masters': masters}
    except (zipfile.BadZipFile, ET.ParseError, KeyError, OSError):
        return None


def mismatches(files, report):
    """the differences between what each file holds and what Visio read (report.txt's lines) -> lines"""
    out, cur, seen = [], None, {}
    for line in report.splitlines():
        m = re.search(r'FILE (.+) \(\d+ bytes\)$', line)
        if m:
            cur = m.group(1)
            seen[cur] = {'pages': {}, 'masters': None, 'opened': False}
            continue
        if cur is None:
            continue
        if re.search(r'  opened in ', line):
            seen[cur]['opened'] = True
        m = re.search(r'  page (\d+) .*: shapes (\d+),', line)
        if m:
            seen[cur]['pages'][int(m.group(1))] = int(m.group(2))
        m = re.search(r'  type \d .*, pages (\d+), masters (\d+),', line)
        if m:
            seen[cur]['npages'], seen[cur]['masters'] = int(m.group(1)), int(m.group(2))
    for f in files:
        if f.suffix.lower() not in ('.vsdx', '.vssx', '.vstx', '.vsdm', '.vssm', '.vstm'):
            continue
        want = counts(f)
        got = seen.get(f.name)
        if got is None or not got['opened']:
            out.append(f'MISMATCH {f.name}: Visio did not open it')
            continue
        if want is None:
            out.append(f'MISMATCH {f.name}: not a package this script can read, Visio opened it anyway')
            continue
        is_stencil = f.suffix.lower() in ('.vssx', '.vssm')
        if not is_stencil and got.get('npages') != len(want['pages']):
            out.append(f'MISMATCH {f.name}: {len(want["pages"])} pages in the file, Visio has {got.get("npages")}')
        if not is_stencil:
            for i, n in enumerate(want['pages'], 1):
                if n is None:
                    out.append(f'MISMATCH {f.name}: page {i} has no part in the file (Visio shows {got["pages"].get(i)} shapes)')
                elif got['pages'].get(i) != n:
                    out.append(f'MISMATCH {f.name}: page {i} holds {n} shapes, Visio read {got["pages"].get(i)}')
        if got['masters'] is not None and got['masters'] != want['masters']:
            out.append(f'MISMATCH {f.name}: {want["masters"]} masters in the file, Visio has {got["masters"]}')
    return out


def tool(name):
    if os.name == 'nt':
        p = Path(os.environ.get('WINDIR', r'C:\Windows')) / 'System32' / 'OpenSSH' / f'{name}.exe'
        if p.exists():
            return str(p)
    return name


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0], formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__)
    ap.add_argument('files', nargs='+', type=Path)
    ap.add_argument('--host', default=os.environ.get('JFD_VISIO_HOST'))
    ap.add_argument('--key', default=os.environ.get('JFD_VISIO_KEY'))
    ap.add_argument('--out', type=Path)
    ap.add_argument('--save', default='', help='same, old or same,old')
    ap.add_argument('--pdf', action='store_true')
    ap.add_argument('--pages', default='png,svg')
    ap.add_argument('--no-shapes', action='store_true')
    ap.add_argument('--data', action='store_true',
                    help="each data recordset's command, rows and columns (Visio may wait for a data source it cannot reach)")
    ap.add_argument('--script', type=Path)
    ap.add_argument('--timeout', type=int, default=0, help='seconds for the Visio run (default 120 + 60 per file)')
    ap.add_argument('--keep', action='store_true', help='leave the job folder on the machine')
    a = ap.parse_args()
    if not a.host:
        sys.exit('visio_check: no machine (--host USER@HOST or JFD_VISIO_HOST)')
    for f in a.files + ([a.script] if a.script else []):
        if not f.is_file():
            sys.exit(f'visio_check: no file {f}')
    out = a.out or a.files[0].resolve().parent / 'visio-out'
    out.mkdir(parents=True, exist_ok=True)
    ssh_opts = ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', '-o', 'ServerAliveInterval=30']
    if a.key:
        ssh_opts += ['-i', a.key]

    def ssh(cmd, check=True):
        r = subprocess.run([tool('ssh')] + ssh_opts + [a.host, cmd], capture_output=True, text=True, encoding='utf-8', errors='replace')
        if check and r.returncode:
            sys.exit(f'visio_check: ssh {cmd!r} failed ({r.returncode}): {r.stderr.strip() or r.stdout.strip()}')
        return r

    def scp(src, dst, recursive=False):
        r = subprocess.run([tool('scp')] + ssh_opts + (['-r'] if recursive else []) + [str(s) for s in src] + [dst],
                           capture_output=True, text=True, encoding='utf-8', errors='replace')
        if r.returncode:
            sys.exit(f'visio_check: scp to {dst} failed ({r.returncode}): {r.stderr.strip()}')

    job = 'job-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S') + '-' + secrets.token_hex(2)
    rjob = f'{REMOTE}\\{job}'
    ssh(f'mkdir "{rjob}\\in" "{rjob}\\out"')
    scp([HERE / 'visio_worker.vbs', HERE / 'visio_run.ps1', HERE / 'visio_watch.ps1'], f'{a.host}:{REMOTE}/')
    with tempfile.TemporaryDirectory() as tmp:
        jobtxt = Path(tmp) / 'job.txt'
        lines = [f'save={a.save}', f'pdf={1 if a.pdf else 0}', f'pages={a.pages}', f'shapes={0 if a.no_shapes else 1}',
                 f'data={1 if a.data else 0}']
        if a.script:
            lines.append(f'script={a.script.name}')
        jobtxt.write_bytes(('\r\n'.join(lines) + '\r\n').encode('ascii'))
        scp([jobtxt], f'{a.host}:{REMOTE}/{job}/')
    scp(a.files + ([a.script] if a.script else []), f'{a.host}:{REMOTE}/{job}/in/')
    timeout = a.timeout or 120 + 60 * len(a.files)
    r = ssh(f'powershell -NoProfile -ExecutionPolicy Bypass -File "{REMOTE}\\visio_run.ps1" -Job "%USERPROFILE%\\{rjob}" -Timeout {timeout}', check=False)
    print(r.stdout.strip())
    if r.returncode:
        print(r.stderr.strip(), file=sys.stderr)
    with tempfile.TemporaryDirectory() as tmp:
        scp([f'{a.host}:{REMOTE}/{job}/out'], tmp, recursive=True)
        for f in (Path(tmp) / 'out').iterdir():
            dst = out / f.name
            if f.suffix in ('.txt', '.tsv'):
                raw = f.read_bytes()
                text = raw.decode('utf-16') if raw[:2] in (b'\xff\xfe', b'\xfe\xff') else raw.decode('utf-8', 'replace')
                dst.write_text(text.replace('\r\n', '\n'), encoding='utf-8')
            else:
                shutil.copyfile(f, dst)
    if not a.keep:
        ssh(f'rmdir /s /q "{rjob}"', check=False)
    report = out / 'report.txt'
    text = report.read_text(encoding='utf-8') if report.exists() else ''
    bad = mismatches([f for f in a.files], text)
    if bad:
        text = text.rstrip('\n') + '\n' + '\n'.join(bad) + '\n'
        report.write_text(text, encoding='utf-8')
    print(text.rstrip())
    print(f'visio_check: results in {out}')
    return 1 if (bad or 'OPEN FAILED' in text or 'FATAL' in text or 'status: ok' not in r.stdout) else 0


if __name__ == '__main__':
    sys.exit(main())
