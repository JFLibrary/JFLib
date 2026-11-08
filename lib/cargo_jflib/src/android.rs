// cargo jflib android: the app as an APK, as tools/android.py of JFLib's repository makes it.  The
// app's library becomes a static library (Cargo), the NDK links it with the C bridge into
// libjfgui_android.so, the Java host (package org.jflib.gallery: the window, input, files,
// notifications, sensors) is compiled with javac and d8, aapt2 links the manifest and resources,
// zipalign and apksigner finish the APK.  No Gradle.  The host's sources are copies in this
// package (android/ of the repository, kept equal there).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cargo;

const MIN_API: u32 = 26;
const TARGET_API: u32 = 35;
const C: &[(&str, &str)] = &[
    ("bridge.c", include_str!("../android/bridge.c")),
    ("audio.c", include_str!("../android/audio.c")),
    ("system.c", include_str!("../android/system.c")),
];
const JAVA: &[(&str, &str)] = &[
    ("GalleryAccessibility.java", include_str!("../android/GalleryAccessibility.java")),
    ("GalleryActivity.java", include_str!("../android/GalleryActivity.java")),
    ("GalleryAudio.java", include_str!("../android/GalleryAudio.java")),
    ("GalleryFiles.java", include_str!("../android/GalleryFiles.java")),
    ("GalleryHost.java", include_str!("../android/GalleryHost.java")),
    ("GalleryInputConnection.java", include_str!("../android/GalleryInputConnection.java")),
    ("GalleryNotify.java", include_str!("../android/GalleryNotify.java")),
    ("GallerySecrets.java", include_str!("../android/GallerySecrets.java")),
    ("GallerySensors.java", include_str!("../android/GallerySensors.java")),
    ("GallerySystem.java", include_str!("../android/GallerySystem.java")),
];

/// the tools of the Android SDK, the NDK and the JDK
struct Tools {
    sdk: PathBuf,
    llvm: PathBuf,
    build: PathBuf,
    platform: PathBuf,
    java: PathBuf,
}

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

fn newest(dir: &Path) -> Option<PathBuf> {
    let key = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).map(|n| n.split('.').map(|v| v.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir() && key(p).is_some_and(|k| k[0] > 0)).max_by_key(key)
}

fn tools() -> Result<Tools, String> {
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let candidates = [env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from), env::var_os("ANDROID_HOME").map(PathBuf::from),
                      env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Android/Sdk")), Some(home.join("Android/Sdk")),
                      Some(home.join("Library/Android/sdk"))];
    let sdk = candidates.into_iter().flatten().find(|p| p.join("build-tools").is_dir())
        .ok_or("the Android SDK is missing: set ANDROID_SDK_ROOT (Android Studio installs it)")?;
    let ndk = env::var_os("ANDROID_NDK_HOME").or_else(|| env::var_os("ANDROID_NDK_ROOT")).map(PathBuf::from)
        .or_else(|| newest(&sdk.join("ndk"))).filter(|n| n.join("toolchains").is_dir())
        .ok_or("the Android NDK is missing: install it with the SDK manager (ndk;29) or set ANDROID_NDK_HOME")?;
    let host = if cfg!(windows) { "windows-x86_64" } else if cfg!(target_os = "macos") { "darwin-x86_64" } else { "linux-x86_64" };
    let platform = sdk.join("platforms/android-36/android.jar");
    if !platform.is_file() {
        return Err("Android SDK Platform 36 is missing: install platforms;android-36".to_string());
    }
    let java = env::var_os("JAVA_HOME").map(|j| PathBuf::from(j).join("bin")).filter(|b| b.join(format!("javac{EXE}")).is_file())
        .or_else(|| env::split_paths(&env::var_os("PATH").unwrap_or_default()).find(|p| p.join(format!("javac{EXE}")).is_file()))
        .ok_or("a JDK is missing: JDK 17 or newer, on the PATH or as JAVA_HOME")?;
    Ok(Tools { build: newest(&sdk.join("build-tools")).ok_or("no build-tools in the Android SDK")?, llvm: ndk.join("toolchains/llvm/prebuilt").join(host),
               sdk, platform, java })
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!("{:?} failed:\n{}", cmd.get_program(), text.trim()));
    }
    Ok(text)
}

/// the linked library's machine, its LOAD segments and RELRO end on 16 KiB pages (Android 15+)
fn verify_elf(data: &[u8], abi: &str) -> Result<(), String> {
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64_at = |at: usize| data.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    if !data.starts_with(b"\x7fELF\x02\x01") {
        return Err("the Android library is no little-endian ELF64 file".to_string());
    }
    if u16_at(18) != Some(if abi == "x86_64" { 62 } else { 183 }) {
        return Err(format!("the Android library is not for {abi}"));
    }
    let (Some(ph), Some(size), Some(count)) = (u64_at(32), u16_at(54), u16_at(56)) else { return Err("bad ELF header".to_string()) };
    let mut loads = 0;
    for i in 0..count as usize {
        let at = ph as usize + i * size as usize;
        let (Some(kind), Some(off), Some(addr), Some(mem), Some(align)) = (u32_at(at), u64_at(at + 8), u64_at(at + 16), u64_at(at + 40), u64_at(at + 48))
        else { return Err("bad ELF program header".to_string()) };
        if kind == 1 {
            loads += 1;
            if align < 16384 || (off.wrapping_sub(addr)) % 16384 != 0 {
                return Err("a LOAD segment is not on 16 KiB pages".to_string());
            }
        }
        if kind == 0x6474_e552 && (addr + mem) % 16384 != 0 {
            return Err("the RELRO end is not on a 16 KiB page".to_string());
        }
    }
    if loads == 0 { Err("the Android library has no loadable segment".to_string()) } else { Ok(()) }
}

/// the manifest of an app without android/AndroidManifest.xml: the shared host under the app's id
/// (PACKAGE app_id=, else org.example.NAME), its display name, the location permission when
/// PERMISSIONS names `location`
fn default_manifest(dir: &Path, name: &str, version: &str, icon: bool) -> String {
    let meta = crate::pkgmeta::Meta::read(dir);
    let get = |k: &str| meta.get(k).map(String::from);
    let id = get("app_id").unwrap_or_else(|| format!("org.example.{}", name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>()));
    let label = get("display").unwrap_or_else(|| name.to_string()).replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let nums: Vec<u32> = version.split(['.', '-', '+']).take(3).map(|v| v.parse().unwrap_or(0)).collect();
    let code = nums.first().unwrap_or(&0) * 10000 + nums.get(1).unwrap_or(&0) * 100 + nums.get(2).unwrap_or(&0);
    let location = fs::read_to_string(dir.join("PERMISSIONS")).unwrap_or_default().lines()
        .any(|l| l.split('#').next().unwrap_or("").split_whitespace().any(|w| w == "location"));
    let perms = if location {
        "    <uses-permission android:name=\"android.permission.ACCESS_FINE_LOCATION\" />\n    <uses-permission android:name=\"android.permission.ACCESS_COARSE_LOCATION\" />\n"
    } else { "" };
    format!(r#"<?xml version="1.0" encoding="utf-8"?>
<!-- made by cargo jflib android: JFLib's Java host (package org.jflib.gallery) under the app's id.
     Write android/AndroidManifest.xml to change it (and android/res/ for its icon). -->
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{id}" android:versionCode="{code}" android:versionName="{version}">
    <uses-feature android:name="android.hardware.touchscreen" android:required="false" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
{perms}    <application android:label="{label}"{icon}
        android:theme="@android:style/Theme.Material.Light.NoActionBar"
        android:allowBackup="false" android:extractNativeLibs="false" android:supportsRtl="true">
        <activity android:name="org.jflib.gallery.GalleryActivity" android:exported="true"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboardHidden|density|uiMode|fontScale"
            android:windowSoftInputMode="adjustResize">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
        <provider android:name="org.jflib.gallery.GalleryFiles" android:authorities="{id}.jfgui.files"
            android:exported="false" android:grantUriPermissions="true" />
    </application>
</manifest>
"#, icon = if icon { " android:icon=\"@drawable/ic_launcher\"" } else { "" })
}

/// what PERMISSIONS allows but the manifest does not declare, and a manifest that hides the app
/// from phones without Vulkan (tools/android.py's manifest_gaps)
fn manifest_gaps(manifest: &str, dir: &Path) -> Result<(), String> {
    for tag in manifest.split("<uses-feature").skip(1).map(|t| t.split('>').next().unwrap_or("")) {
        if tag.contains("android.hardware.vulkan") && !tag.contains("android:required=\"false\"") {
            return Err("the manifest requires Vulkan: the app must start without it (android:required=\"false\")".to_string());
        }
    }
    let location = fs::read_to_string(dir.join("PERMISSIONS")).unwrap_or_default().lines()
        .any(|l| l.split('#').next().unwrap_or("").split_whitespace().any(|w| w == "location"));
    if location && !manifest.contains("ACCESS_FINE_LOCATION") && !manifest.contains("ACCESS_COARSE_LOCATION") {
        return Err("PERMISSIONS has `location`: the manifest needs ACCESS_FINE_LOCATION and / or ACCESS_COARSE_LOCATION".to_string());
    }
    Ok(())
}

fn files_below(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    for e in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_below(root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).unwrap_or(&p).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            out.push((rel, fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?));
        }
    }
    Ok(())
}

pub(crate) fn run_cmd(args: &[String]) -> Result<(), String> {
    let mut abi = "arm64-v8a".to_string();
    let mut install = false;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--abi" => {
                i += 1;
                abi = args.get(i).cloned().ok_or("--abi needs arm64-v8a or x86_64")?;
            }
            "--install" => install = true,
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    let triple = match abi.as_str() {
        "arm64-v8a" => "aarch64-linux-android",
        "x86_64" => "x86_64-linux-android",
        other => return Err(format!("--abi {other}: arm64-v8a or x86_64")),
    };
    let o = crate::opts(&rest)?;
    let t = tools()?;
    let p = cargo::project(o.package.as_deref())?;
    if p.package.lib.is_none() {
        return Err(format!("{}: Android loads an app as a library: its code in src/lib.rs, src/main.rs only links it \
                            (the template of cargo jflib new)", p.package.name));
    }
    let manifest_file = p.package.dir.join("android/AndroidManifest.xml");
    let res = p.package.dir.join("android/res");
    let manifest = if manifest_file.is_file() {
        fs::read_to_string(&manifest_file).map_err(|e| format!("{}: {e}", manifest_file.display()))?
    } else {
        default_manifest(&p.package.dir, &p.package.name, &p.package.version, res.join("drawable").is_dir())
    };
    manifest_gaps(&manifest, &p.package.dir)?;
    let work = p.target_dir.join("jflib-android").join(&abi).join(&p.package.name);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;

    // the app as a static library
    let mut cmd = cargo::command();
    cmd.args(["rustc", "-p", &p.package.name, "--lib", "--crate-type", "staticlib", "--target", triple]);
    if o.release {
        cmd.arg("--release");
    }
    let archive = cargo::build(&mut cmd)?.into_iter().filter(|a| a.package_id == p.package.id).flat_map(|a| a.filenames)
        .find(|f| f.extension().is_some_and(|x| x == "a")).ok_or("cargo built no static library")?;

    // the C bridge and the app -> libjfgui_android.so
    let csrc = work.join("c");
    fs::create_dir_all(&csrc).map_err(|e| e.to_string())?;
    for (name, text) in C {
        fs::write(csrc.join(name), text).map_err(|e| e.to_string())?;
    }
    let native = work.join("libjfgui_android.so");
    let bin = t.llvm.join("bin");
    run(Command::new(bin.join(format!("clang{EXE}")))
        .arg(format!("--target={triple}{MIN_API}")).arg(format!("--sysroot={}", t.llvm.join("sysroot").display()))
        .args(["-std=c11", "-O2", "-fPIC", "-fvisibility=hidden", "-Wall", "-Wextra", "-Werror", "-shared"])
        .args(C.iter().map(|(n, _)| csrc.join(n))).arg(&archive)
        .args(["-Wl,--gc-sections,--exclude-libs,ALL,--no-undefined", "-Wl,-z,max-page-size=16384", "-Wl,-z,common-page-size=16384",
               "-Wl,-z,relro,-z,now", "-llog", "-ljnigraphics", "-landroid", "-laaudio", "-ldl", "-lm", "-o"])
        .arg(&native))?;
    run(Command::new(bin.join(format!("llvm-strip{EXE}"))).arg("--strip-unneeded").arg(&native))?;
    let so = fs::read(&native).map_err(|e| e.to_string())?;
    verify_elf(&so, &abi)?;

    // the Java host -> classes.dex
    let java = work.join("java");
    let classes = work.join("classes");
    let dex = work.join("dex");
    for d in [&java, &dex] {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(&classes);
    fs::create_dir_all(&classes).map_err(|e| e.to_string())?;
    for (name, text) in JAVA {
        fs::write(java.join(name), text).map_err(|e| e.to_string())?;
    }
    run(Command::new(t.java.join(format!("javac{EXE}"))).args(["--release", "8", "-Xlint:-options", "-encoding", "UTF-8", "-classpath"])
        .arg(&t.platform).arg("-d").arg(&classes).args(JAVA.iter().map(|(n, _)| java.join(n))))?;
    let mut class_files = Vec::new();
    files_below(&classes, &classes, &mut class_files)?;
    run(Command::new(t.java.join(format!("java{EXE}"))).arg("-cp").arg(t.build.join("lib/d8.jar"))
        .args(["com.android.tools.r8.D8", "--min-api", &MIN_API.to_string(), "--lib"]).arg(&t.platform)
        .arg("--output").arg(&dex).args(class_files.iter().map(|(rel, _)| classes.join(rel))))?;

    // the manifest and resources -> the APK
    let manifest_out = work.join("AndroidManifest.xml");
    fs::write(&manifest_out, &manifest).map_err(|e| e.to_string())?;
    let unsigned = work.join("unsigned.apk");
    let aapt2 = t.build.join(format!("aapt2{EXE}"));
    let mut link = Command::new(&aapt2);
    link.arg("link").arg("-o").arg(&unsigned).arg("-I").arg(&t.platform).arg("--manifest").arg(&manifest_out)
        .args(["--min-sdk-version", &MIN_API.to_string(), "--target-sdk-version", &TARGET_API.to_string(), "--auto-add-overlay"]);
    if res.is_dir() {
        let compiled = work.join("compiled-res.zip");
        run(Command::new(&aapt2).arg("compile").arg("--dir").arg(&res).arg("-o").arg(&compiled))?;
        link.arg("-R").arg(&compiled);
    }
    run(&mut link)?;
    let notices = work.join("licenses");
    let _ = fs::remove_dir_all(&notices);
    crate::notices::copy(&p, triple, &notices)?;
    let mut extra = vec![("classes.dex".to_string(), fs::read(dex.join("classes.dex")).map_err(|e| e.to_string())?),
                         (format!("lib/{abi}/libjfgui_android.so"), so)];
    let mut licences = Vec::new();
    files_below(&work, &notices, &mut licences)?;
    extra.extend(licences.into_iter().map(|(rel, data)| (format!("assets/{rel}"), data)));
    crate::zip::append(&unsigned, &extra)?;
    let aligned = work.join("aligned.apk");
    let zipalign = t.build.join(format!("zipalign{EXE}"));
    run(Command::new(&zipalign).args(["-f", "-P", "16", "4"]).arg(&unsigned).arg(&aligned))?;

    // signed: the release key from the environment, else a debug key in the target folder
    let kind = if o.release { "release" } else { "debug" };
    let apk = p.target_dir.join("jflib-android").join(format!("{}-{abi}-{kind}.apk", p.package.name));
    let mut sign = Command::new(t.java.join(format!("java{EXE}")));
    sign.arg("-jar").arg(t.build.join("lib/apksigner.jar")).arg("sign");
    if o.release {
        let keystore = env::var("jfgui_ANDROID_KEYSTORE").map_err(|_| "--release signs with the release key: set jfgui_ANDROID_KEYSTORE, \
                                                                        jfgui_ANDROID_KEYSTORE_PASS and jfgui_ANDROID_KEY_ALIAS (jfgui_ANDROID_KEY_PASS \
                                                                        if the key has a password of its own)")?;
        let alias = env::var("jfgui_ANDROID_KEY_ALIAS").map_err(|_| "set jfgui_ANDROID_KEY_ALIAS")?;
        let key_pass = if env::var_os("jfgui_ANDROID_KEY_PASS").is_some() { "env:jfgui_ANDROID_KEY_PASS" } else { "env:jfgui_ANDROID_KEYSTORE_PASS" };
        sign.args(["--ks", &keystore, "--ks-key-alias", &alias, "--ks-pass", "env:jfgui_ANDROID_KEYSTORE_PASS", "--key-pass", key_pass]);
    } else {
        let keystore = p.target_dir.join("jflib-android").join("debug.keystore");
        if !keystore.is_file() {
            run(Command::new(t.java.join(format!("keytool{EXE}"))).args(["-genkeypair", "-keystore"]).arg(&keystore)
                .args(["-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey", "-dname", "CN=Android Debug,O=jflib,C=DE",
                       "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000"]))?;
        }
        sign.arg("--ks").arg(&keystore).args(["--ks-pass", "pass:android", "--key-pass", "pass:android"]);
    }
    run(sign.arg("--out").arg(&apk).arg(&aligned))?;
    run(Command::new(t.java.join(format!("java{EXE}"))).arg("-jar").arg(t.build.join("lib/apksigner.jar")).args(["verify", "--verbose"]).arg(&apk))?;
    run(Command::new(&zipalign).args(["-c", "-P", "16", "4"]).arg(&apk))?;
    println!("wrote {} ({} bytes)", apk.display(), fs::metadata(&apk).map(|m| m.len()).unwrap_or(0));
    if install {
        let adb = t.sdk.join(format!("platform-tools/adb{EXE}"));
        print!("{}", run(Command::new(adb).args(["install", "-r"]).arg(&apk))?);
    }
    Ok(())
}
