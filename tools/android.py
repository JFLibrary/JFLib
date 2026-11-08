"""Android native APK packaging for build_rs.py: the static library Cargo built + NDK + SDK, no Gradle.

The shared Java/JNI host presents the library's pixels and forwards input. Apps opt in
with `android` in TARGETS and supply android/AndroidManifest.xml and android/res/.
A program without a UI (`android` in TARGETS, ui not among its crates: apps/gputest) is a
plain executable for `adb shell` instead (build_exe, android/exe_main.c).
jfgui_ANDROID_PACKAGE=NAME (test builds): the APK under another package, beside what is installed
on a phone, which it leaves alone (other debug keys, the user's data there).
"""
from pathlib import Path
import hashlib
import os
import re
import shutil
import struct
import sys
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]
MIN_API = 26


def debug_key(directory, java, suffix, run):
    """Serialize signing-key creation across simultaneous ABI/style build processes."""
    keystore = directory / 'debug.keystore'
    lock = directory / 'debug-keystore.lock'
    deadline = time.monotonic() + 60
    while True:
        try:
            handle = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.close(handle)
            break
        except FileExistsError:
            if time.monotonic() >= deadline:
                raise RuntimeError(f'Debug signing-key lock is busy: {lock}')
            time.sleep(0.1)
    try:
        if not keystore.exists():
            run([java / ('keytool' + suffix), '-genkeypair', '-keystore', keystore, '-storepass', 'android',
                 '-keypass', 'android', '-alias', 'androiddebugkey', '-dname', 'CN=Android Debug,O=jfgui,C=DE',
                 '-keyalg', 'RSA', '-keysize', '2048', '-validity', '10000'])
        return keystore
    finally:
        lock.unlink()


def release_key():
    """--release: the release key from the environment -> (keystore, alias) or RuntimeError
    naming what is missing.  apksigner reads the passwords from the environment itself
    (env:jfgui_ANDROID_KEYSTORE_PASS / jfgui_ANDROID_KEY_PASS), so they never appear on a
    command line."""
    keystore = os.environ.get('jfgui_ANDROID_KEYSTORE')
    alias = os.environ.get('jfgui_ANDROID_KEY_ALIAS')
    missing = [n for n, v in (('jfgui_ANDROID_KEYSTORE', keystore), ('jfgui_ANDROID_KEYSTORE_PASS',
               os.environ.get('jfgui_ANDROID_KEYSTORE_PASS')), ('jfgui_ANDROID_KEY_ALIAS', alias)) if not v]
    if missing:
        raise RuntimeError('--release signs the APK with the release key; missing: ' + ', '.join(missing)
                           + ' (jfgui_ANDROID_KEY_PASS if the key has its own password; '
                           'jfgui_ANDROID_DEBUG_SIGN=1 signs a release build with the debug key, for tests only)')
    if not Path(keystore).is_file():
        raise RuntimeError(f'jfgui_ANDROID_KEYSTORE: no keystore at {keystore}')
    return Path(keystore), alias


def verify_elf(path, abi):
    """Check the actual linked ELF machine, LOAD and RELRO alignment, not compiler flags."""
    data = path.read_bytes()
    if len(data) < 64 or data[:6] != b'\x7fELF\x02\x01':
        raise RuntimeError('Android output must be a little-endian ELF64 library.')
    machine = struct.unpack_from('<H', data, 18)[0]
    if machine != {'arm64-v8a': 183, 'x86_64': 62}[abi]:
        raise RuntimeError(f'Wrong ELF machine {machine} for {abi}.')
    offset = struct.unpack_from('<Q', data, 32)[0]
    entry_size, count = struct.unpack_from('<HH', data, 54)
    loads = 0
    if entry_size < 56 or count == 0 or offset + entry_size * count > len(data):
        raise RuntimeError('Invalid ELF program header table.')
    for index in range(count):
        typ, _, file_offset, address, _, _, memory_size, alignment = struct.unpack_from('<IIQQQQQQ', data, offset + index * entry_size)
        if typ == 1:
            loads += 1
            if alignment < 16384 or (file_offset - address) % 16384:
                raise RuntimeError('Android ELF LOAD segment is not 16 KiB aligned.')
        if typ == 0x6474e552 and (address + memory_size) % 16384:
            raise RuntimeError('Android ELF RELRO end is not 16 KiB aligned.')
    if not loads:
        raise RuntimeError('Android ELF has no loadable segment.')


def newest(directory):
    versions = [p for p in directory.glob('*') if p.is_dir() and p.name[:1].isdigit()]
    return max(versions, key=lambda p: tuple(int(v) for v in p.name.split('.') if v.isdigit())) if versions else None


def sdk_tools():
    candidates = [os.environ.get('ANDROID_SDK_ROOT'), os.environ.get('ANDROID_HOME'),
                  str(Path(os.environ.get('LOCALAPPDATA', Path.home())) / 'Android' / 'Sdk'),
                  str(Path.home() / 'Android' / 'Sdk'), str(Path.home() / 'Library' / 'Android' / 'sdk')]
    sdk = next((Path(p) for p in candidates if p and (Path(p) / 'build-tools').is_dir()), None)
    if sdk is None:
        raise RuntimeError('Android SDK missing: set ANDROID_SDK_ROOT.')
    ndk_env = os.environ.get('ANDROID_NDK_HOME') or os.environ.get('ANDROID_NDK_ROOT')
    ndk = Path(ndk_env) if ndk_env else newest(sdk / 'ndk')
    if ndk is None or not (ndk / 'toolchains').is_dir():
        raise RuntimeError('Android NDK missing: install NDK r29 (29.0.14206865) or set ANDROID_NDK_HOME.')
    host = 'windows-x86_64' if os.name == 'nt' else 'darwin-x86_64' if sys.platform == 'darwin' else 'linux-x86_64'
    llvm = ndk / 'toolchains' / 'llvm' / 'prebuilt' / host
    build = newest(sdk / 'build-tools')
    platform = sdk / 'platforms' / 'android-36' / 'android.jar'
    if not platform.is_file():
        raise RuntimeError('Android SDK Platform 36 missing; install platforms;android-36.')
    java_home = os.environ.get('JAVA_HOME')
    java = Path(java_home) / 'bin' if java_home and (Path(java_home) / 'bin' / 'javac.exe').exists() else None
    if java is None:
        javac = shutil.which('javac')
        if not javac:
            raise RuntimeError('JDK missing: install JDK 17 or newer.')
        java = Path(javac).parent
    suffix = '.exe' if os.name == 'nt' else ''
    return sdk, ndk, llvm, build, platform, java, suffix


def manifest_gaps(perms, manifest):
    """what apps/NAME/PERMISSIONS allows but android/AndroidManifest.xml does not declare (Android
    would refuse it without a word) -> the message, or None.  Only the location needs a line:
    motion sensors have no permission, and the other rules have no Android permission behind them
    or ask for theirs elsewhere"""
    # the UI runs without Vulkan (GalleryActivity, docs/GPU.md "Ausgabe unter Android"): the store must
    # not hide the app from phones without it
    for tag in re.findall(r'<uses-feature[^>]*android\.hardware\.vulkan[^>]*>', manifest):
        if 'android:required="false"' not in tag:
            return f'AndroidManifest.xml requires Vulkan ({tag}): the app must start without it (android:required="false").'
    if perms is None:
        return None
    keys = set()
    for line in perms.splitlines():
        words = line.split('#')[0].split()
        if words:
            keys.add(words[1] if words[0] == 'ask' and len(words) > 1 else words[0])
    if 'location' in keys and 'ACCESS_FINE_LOCATION' not in manifest and 'ACCESS_COARSE_LOCATION' not in manifest:
        return ('PERMISSIONS has `location`: android/AndroidManifest.xml needs <uses-permission> for '
                'android.permission.ACCESS_FINE_LOCATION and / or ACCESS_COARSE_LOCATION.')
    return None


def manifest_for(manifest, work):
    """the manifest to link: the app's, under another package with jfgui_ANDROID_PACKAGE"""
    package = os.environ.get('jfgui_ANDROID_PACKAGE')
    if not package:
        return manifest
    text = manifest.read_text(encoding='utf-8')
    old = re.search(r'package="([^"]+)"', text).group(1)
    text = text.replace('android:name=".', f'android:name="{old}.').replace(f'package="{old}"', f'package="{package}"')
    text = text.replace(f'android:targetPackage="{old}"', f'android:targetPackage="{package}"')
    text = text.replace(f'android:authorities="{old}.', f'android:authorities="{package}.')
    out = work / 'AndroidManifest.xml'
    out.write_text(text, encoding='utf-8')
    return out


def native_link_flags():
    """what every native link of this target has: only what is used, 16 KiB pages, no text relocations"""
    return ['-Wl,--gc-sections,--exclude-libs,ALL,--no-undefined', '-Wl,-z,max-page-size=16384',
            '-Wl,-z,common-page-size=16384', '-Wl,-z,relro,-z,now']


def build_exe(builder, name, archive):
    """a program without a UI -> <out>/android/NAME-<abi>: `adb push` it to /data/local/tmp and run it
    from `adb shell` (the driver of a GPU needs no app for that); no APK, no Java.  archive: the
    static library Cargo built (build_rs.py's derived_workspace)"""
    build = sys.modules[builder.__class__.__module__]
    error, run = build.BuildError, build.run_cmd
    try:
        sdk, ndk, llvm, sdkbuild, platform, java, suffix = sdk_tools()
    except RuntimeError as exc:
        raise error(str(exc)) from exc
    work = builder.dir / 'apps' / name
    work.mkdir(parents=True, exist_ok=True)
    output = builder.android_output / f'{name}-{builder.android_abi}'
    digest = hashlib.sha256(Path(archive).read_bytes())
    for path in (Path(__file__), ROOT / 'build_rs.py', ROOT / 'android' / 'exe_main.c'):
        digest.update(path.read_bytes())
    digest.update(f'{ndk}'.encode())
    stamp = work / 'exe.sha256'
    fingerprint = digest.hexdigest()
    if output.exists() and stamp.exists() and stamp.read_text() == fingerprint:
        return output
    triple = 'aarch64-linux-android' if builder.android_abi == 'arm64-v8a' else 'x86_64-linux-android'
    run([llvm / 'bin' / ('clang' + suffix), f'--target={triple}{MIN_API}', f'--sysroot={llvm / "sysroot"}', '-std=c11', '-O2',
         '-fPIE', '-pie', '-Wall', '-Wextra', '-Werror', ROOT / 'android' / 'exe_main.c', archive, *native_link_flags(),
         '-llog', '-ldl', '-lm', '-o', output])
    run([llvm / 'bin' / ('llvm-strip' + suffix), '--strip-unneeded', output])
    stamp.write_text(fingerprint)
    return output


def build_apk(builder, name, style, variant, archive, notices):
    """archive: the static library Cargo built (build_rs.py's derived_workspace); notices: {relative
    Path: source} (build_rs.app_notices), packed below assets/"""
    # the error type and helpers of the active build entry point, including __main__
    build = sys.modules[builder.__class__.__module__]
    error, run = build.BuildError, build.run_cmd
    app = ROOT / 'apps' / name
    manifest = app / 'android' / 'AndroidManifest.xml'
    if not manifest.is_file():
        raise error(f'{name}: Android target needs android/AndroidManifest.xml.')
    try:
        sdk, ndk, llvm, sdkbuild, platform, java, suffix = sdk_tools()
    except RuntimeError as exc:
        raise error(str(exc)) from exc
    key = name + ('-' + style if variant else '')
    work = builder.dir / 'apps' / key
    work.mkdir(parents=True, exist_ok=True)
    # --release: the release key from the environment (jfgui_ANDROID_DEBUG_SIGN=1: the debug key)
    release = builder.release and os.environ.get('jfgui_ANDROID_DEBUG_SIGN') != '1'
    if release:
        try:
            keystore, alias = release_key()
        except RuntimeError as exc:
            raise error(f'{name}: {exc}') from exc
    output = builder.android_output / f'{key}-{builder.android_abi}-{"release" if release else "debug"}.apk'
    # Packaging and tool versions participate in freshness, including the host sources.
    missing = manifest_gaps(build.app_permissions(name), manifest.read_text(encoding='utf-8'))
    if missing:
        raise error(f'{name}: {missing}')
    digest = hashlib.sha256(Path(archive).read_bytes() + style.encode())
    sources = [Path(__file__), ROOT / 'build_rs.py']
    sources += sorted(p for p in (ROOT / 'android').rglob('*') if p.is_file())
    sources += sorted(p for p in (app / 'android').rglob('*') if p.is_file())
    for path in sources:
        digest.update(path.read_bytes())
    for relative, path in sorted(notices.items()):
        digest.update(relative.as_posix().encode() + b'\0' + path.read_bytes())
    digest.update(f'{ndk}:{sdkbuild}:{platform}:{java}'.encode())
    digest.update((f'release:{keystore}:{alias}' if release else 'debug').encode())
    digest.update(os.environ.get('jfgui_ANDROID_PACKAGE', '').encode())
    stamp = work / 'apk.sha256'
    fingerprint = digest.hexdigest()
    if output.exists() and stamp.exists() and stamp.read_text() == fingerprint:
        return output

    triple = 'aarch64-linux-android' if builder.android_abi == 'arm64-v8a' else 'x86_64-linux-android'
    native = work / 'libjfgui_android.so'
    run([llvm / 'bin' / ('clang' + suffix), f'--target={triple}{MIN_API}',
         f'--sysroot={llvm / "sysroot"}', '-std=c11', '-O2', '-fPIC', '-fvisibility=hidden',
         '-Wall', '-Wextra', '-Werror', '-shared', ROOT / 'android' / 'bridge.c', ROOT / 'android' / 'audio.c',
         ROOT / 'android' / 'system.c', archive, *native_link_flags(),
         '-llog', '-ljnigraphics', '-landroid', '-laaudio', '-ldl', '-lm', '-o', native])
    run([llvm / 'bin' / ('llvm-strip' + suffix), '--strip-unneeded', native])
    try:
        verify_elf(native, builder.android_abi)
    except RuntimeError as exc:
        raise error(str(exc)) from exc
    elf = run([llvm / 'bin' / ('llvm-readelf' + suffix), '-h', '-l', native], quiet=True)
    (work / 'native-elf.txt').write_bytes(elf.encode('utf-8'))
    if ('AArch64' if builder.android_abi == 'arm64-v8a' else 'X86-64') not in elf:
        raise error('NDK output architecture does not match requested ABI.')

    classes, dex = work / 'classes', work / 'dex'
    classes.mkdir(exist_ok=True)
    dex.mkdir(exist_ok=True)
    for stale_class in classes.rglob('*.class'):
        stale_class.unlink()
    run([java / ('javac' + suffix), '--release', '8', '-Xlint:-options', '-encoding', 'UTF-8', '-classpath', platform,
         '-d', classes, *sorted((ROOT / 'android').glob('*.java')),
         *sorted((app / 'android' / 'tests').glob('*.java'))])
    run([java / ('java' + suffix), '-cp', sdkbuild / 'lib' / 'd8.jar', 'com.android.tools.r8.D8',
         '--min-api', str(MIN_API), '--lib', platform, '--output', dex, *sorted(classes.rglob('*.class'))])
    resources = work / 'compiled-res.zip'
    run([sdkbuild / ('aapt2' + suffix), 'compile', '--dir', app / 'android' / 'res', '-o', resources])
    unsigned = work / 'unsigned.apk'
    run([sdkbuild / ('aapt2' + suffix), 'link', '-o', unsigned, '-I', platform, '--manifest', manifest_for(manifest, work),
         '-R', resources, '--min-sdk-version', str(MIN_API), '--target-sdk-version', '35', '--auto-add-overlay'])
    with zipfile.ZipFile(unsigned, 'a') as apk:
        apk.write(dex / 'classes.dex', 'classes.dex', compress_type=zipfile.ZIP_DEFLATED)
        apk.write(native, f'lib/{builder.android_abi}/libjfgui_android.so', compress_type=zipfile.ZIP_STORED)
        for relative, notice in sorted(notices.items()):
            apk.write(notice, 'assets/' + relative.as_posix(), compress_type=zipfile.ZIP_DEFLATED)
    aligned = work / 'aligned.apk'
    run([sdkbuild / ('zipalign' + suffix), '-f', '-P', '16', '4', unsigned, aligned])
    signer = [java / ('java' + suffix), '-jar', sdkbuild / 'lib' / 'apksigner.jar']
    if release:
        key_pass = 'jfgui_ANDROID_KEY_PASS' if os.environ.get('jfgui_ANDROID_KEY_PASS') else 'jfgui_ANDROID_KEYSTORE_PASS'
        run(signer + ['sign', '--ks', keystore, '--ks-key-alias', alias, '--ks-pass', 'env:jfgui_ANDROID_KEYSTORE_PASS',
                      '--key-pass', f'env:{key_pass}', '--out', output, aligned])
    else:
        try:
            keystore = debug_key(builder.android_output, java, suffix, run)
        except RuntimeError as exc:
            raise error(str(exc)) from exc
        run(signer + ['sign', '--ks', keystore, '--ks-pass', 'pass:android', '--key-pass', 'pass:android',
                      '--out', output, aligned])
    verification = run(signer + ['verify', '--verbose', output], quiet=True)
    verification += '\n' + run([sdkbuild / ('zipalign' + suffix), '-c', '-P', '16', '4', output], quiet=True)
    (work / 'apk-verification.txt').write_bytes(verification.encode('utf-8'))
    stamp.write_text(fingerprint)
    return output
