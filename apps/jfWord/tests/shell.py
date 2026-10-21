"""Independent process checks for local recent/pinned files and draft-safe navigation."""
import configparser


def run(invoke, rel, out, root, sample):
    settings = out / 'shell-settings'
    settings.mkdir(exist_ok=True)
    ini = settings / 'recent.ini'
    if ini.exists():
        ini.unlink()
    document = out / 'shell-source.docx'
    missing = out / 'shell-missing.docx'
    document.write_bytes(sample.read_bytes())
    missing.write_bytes(sample.read_bytes())

    def script(name, source, *args):
        path = out / (name + '.uis')
        path.write_text(source, encoding='utf-8')
        invoke('--settings-dir', rel(settings), *args, '--script', rel(path), '--verify-dirty')

    script('shell-pin', '''cmd 28
expect button:"shell-source.docx" exists
click button:"Anheften"
expect button:"Loslösen" exists
expect_nowarn
''', '--file', rel(document))
    config = configparser.ConfigParser()
    config.read(ini, encoding='utf-8')
    assert config['recent0']['pinned'] == '1'
    assert config['recent0']['path'].replace('\\', '/').endswith('/shell-source.docx')

    script('shell-remember-missing', '''cmd 28
expect button:"shell-missing.docx" exists
expect button:"shell-source.docx" exists
expect button:"Loslösen" exists
expect_nowarn
''', '--file', rel(missing))
    missing.unlink()
    script('shell-reload', '''expect text:"Willkommen bei jfWord" exists
expect button:"shell-source.docx" exists
expect button:"shell-missing.docx" exists
expect button:"Loslösen" exists
click #2252
expect button:"shell-source.docx" exists
expect button:"shell-missing.docx" missing
click #2251
click button:"shell-source.docx"
expect text:"Dokument geöffnet · Quellen und Zitatverweise geladen" exists
click #10000
key ctrl+home
type "Ungespeicherter Entwurf"
cmd 28
click button:"shell-missing.docx"
expect dialog:"Ungespeicherte Änderungen" exists
click "Verwerfen"
expect text:"Willkommen bei jfWord" exists
click button:"Zum Dokument"
expect text:"shell-source.docx — Geändert" exists
expect #10000 text "Ungespeicherter EntwurfGrüße & Forschung <2026>\\nα β\\t𝄞(Lovelace, 1843, S. 42–44) Danach.\\nLovelace, Ada (1843). Notizen & Methoden. London."
cmd 28
click #2132
type "source"
expect button:"shell-source.docx" exists
expect button:"shell-missing.docx" missing
expect_nowarn
''')
