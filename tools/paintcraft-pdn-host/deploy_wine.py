"""Deploy only PaintCraft's helper into a private Paint.NET-on-Wine directory.
Its bundled runtime and dependency manifest avoid mixing Windows .NET/Paint.NET
versions. No Paint.NET binaries are copied into JFLib or redistributed.
"""
import argparse
import json
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--host', required=True, type=Path, help='Built PaintCraft.PdnHost.dll')
    parser.add_argument('--paintnet', required=True, type=Path, help='Private unpacked Paint.NET-on-Wine directory')
    a = parser.parse_args()
    host, dest = a.host.resolve(), a.paintnet.resolve()
    if host.name != 'PaintCraft.PdnHost.dll' or not host.is_file():
        parser.error('--host must name a built PaintCraft.PdnHost.dll')
    try:
        configuration = json.loads((dest / 'PaintDotNet.Configuration.json').read_text(encoding='utf-8-sig'))
        runtime = json.loads((dest / 'paintdotnet.runtimeconfig.json').read_text(encoding='utf-8-sig'))
        json.loads((dest / 'paintdotnet.deps.json').read_text(encoding='utf-8-sig'))
    except (OSError, ValueError) as e:
        parser.error(str(e))
    if not configuration.get('EnableWineMode') or not runtime.get('runtimeOptions', {}).get('includedFrameworks'):
        parser.error('A self-contained Paint.NET-on-Wine package with EnableWineMode is required')
    for filename in ['coreclr.dll', 'hostfxr.dll', 'PaintDotNet.Effects.Core.dll']:
        if not (dest / filename).is_file():
            parser.error(f'Incomplete Paint.NET-on-Wine package: {filename}')
    for source, filename in [(host, 'PaintCraft.PdnHost.dll'),
            (dest / 'paintdotnet.runtimeconfig.json', 'PaintCraft.PdnHost.runtimeconfig.json'),
            (dest / 'paintdotnet.deps.json', 'PaintCraft.PdnHost.deps.json')]:
        target = dest / filename
        if source != target:
            shutil.copyfile(source, target)
    apphost = host.with_suffix('.exe')
    if apphost.is_file() and apphost.read_bytes()[:2] == b'MZ':
        target = dest / apphost.name
        if apphost != target:
            shutil.copyfile(apphost, target)
        print(target)
    else:
        print(dest / 'PaintCraft.PdnHost.dll')


if __name__ == '__main__':
    main()
