// AppImage, RPM and Flatpak of an app (tools/pkg_linux.py of the JFLib repository,
// docs/PACKAGING.md "AppImage", "RPM", "Flatpak").
//
//   appimages()   one NAME-VERSION-STYLE-x86_64.AppImage per style: the AppDir (AppRun, .desktop,
//                 icon, metainfo, the program, the licences) as SquashFS behind the runtime
//                 jflib-appimagert (built here, static); no Linux tool needed
//   rpm()         rpm/SPECS/PKG.spec + rpm/SOURCES/PKG-VERSION-root.tar.gz, and the .rpm when
//                 rpmbuild is there (Linux, or WSL on Windows)
//   flatpak()     flatpak/ID.json + flatpak/files/: the manifest, finish-args from PERMISSIONS;
//                 ID.flatpak when flatpak-builder and the runtime are there

use std::collections::BTreeMap;
use std::fs;

use crate::debfile::{self, Tree};
use crate::deflate;
use crate::linux::Linux;
use crate::package::{self, App, APPIMAGERT};
use crate::pkgmeta::{self, About};
use crate::squashfs::{self, Item};

const UPD_INFO_SIZE: usize = 1024;
/// the sections of a type-2 runtime: name, size
const RUNTIME_SECTIONS: [(&str, usize); 3] = [(".upd_info", UPD_INFO_SIZE), (".sha256_sig", 1024), (".sig_key", 8192)];
const FLATPAK_RUNTIME: &str = "org.freedesktop.Platform";
const FLATPAK_VERSION: &str = "26.08";

fn u16_at(b: &[u8], at: usize) -> usize {
    b.get(at..at + 2).map_or(0, |x| u16::from_le_bytes([x[0], x[1]]) as usize)
}

fn u32_at(b: &[u8], at: usize) -> usize {
    b.get(at..at + 4).map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap_or_default()) as usize)
}

fn u64_at(b: &[u8], at: usize) -> usize {
    b.get(at..at + 8).map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap_or_default()) as usize)
}

/// where an ELF64 file's data ends: its section table, sections and segments
fn elf_end(b: &[u8]) -> Result<usize, String> {
    if !b.starts_with(b"\x7fELF\x02\x01") {
        return Err("the AppImage runtime is no ELF64 little-endian file".to_string());
    }
    let (phoff, shoff) = (u64_at(b, 0x20), u64_at(b, 0x28));
    let (phsize, phnum, shsize, shnum) = (u16_at(b, 0x36), u16_at(b, 0x38), u16_at(b, 0x3A), u16_at(b, 0x3C));
    let mut end = shoff + shsize * shnum;
    for i in 0..phnum {
        let ph = phoff + i * phsize;
        end = end.max(u64_at(b, ph + 8) + u64_at(b, ph + 32));
    }
    for i in 0..shnum {
        let sh = shoff + i * shsize;
        if u32_at(b, sh + 4) != 8 {
            end = end.max(u64_at(b, sh + 24) + u64_at(b, sh + 32)); // SHT_NOBITS takes no room
        }
    }
    Ok(end)
}

/// {name: (offset, size)} of an ELF64 file's sections
fn elf_sections(b: &[u8]) -> BTreeMap<String, (usize, usize)> {
    let shoff = u64_at(b, 0x28);
    let (shsize, shnum, shstrndx) = (u16_at(b, 0x3A), u16_at(b, 0x3C), u16_at(b, 0x3E));
    let names_at = u64_at(b, shoff + shstrndx * shsize + 24);
    let mut out = BTreeMap::new();
    for i in 0..shnum {
        let sh = shoff + i * shsize;
        let n = names_at + u32_at(b, sh);
        let name: Vec<u8> = b.get(n..).unwrap_or_default().iter().take_while(|&&c| c != 0).copied().collect();
        out.insert(String::from_utf8_lossy(&name).to_string(), (u64_at(b, sh + 24), u64_at(b, sh + 32)));
    }
    out
}

/// the ELF64 file with the sections `new` (zero-filled, not loaded) added at the end, before a
/// new section table (the old one must end the file)
fn add_sections(b: &[u8], new: &[(&str, usize)]) -> Result<Vec<u8>, String> {
    let shoff = u64_at(b, 0x28);
    let (shsize, shnum, shstrndx) = (u16_at(b, 0x3A), u16_at(b, 0x3C), u16_at(b, 0x3E));
    if shoff + shsize * shnum != b.len() {
        return Err("the AppImage runtime's section table does not end the file".to_string());
    }
    let mut heads: Vec<Vec<u8>> = (0..shnum).map(|i| b[shoff + i * shsize..shoff + (i + 1) * shsize].to_vec()).collect();
    let (noff, nsize) = (u64_at(&heads[shstrndx], 24), u64_at(&heads[shstrndx], 32));
    let mut names = b[noff..noff + nsize].to_vec();
    let mut out = b[..shoff].to_vec();
    let mut added = Vec::new();
    for (name, size) in new {
        out.resize(out.len().div_ceil(8) * 8, 0);
        added.push((names.len(), out.len(), *size));
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        out.resize(out.len() + size, 0);
    }
    let names_at = out.len();
    out.extend_from_slice(&names);
    out.resize(out.len().div_ceil(8) * 8, 0);
    heads[shstrndx][24..32].copy_from_slice(&(names_at as u64).to_le_bytes());
    heads[shstrndx][32..40].copy_from_slice(&(names.len() as u64).to_le_bytes());
    let new_shoff = out.len();
    for h in &heads {
        out.extend_from_slice(h);
    }
    for (name_at, at, size) in added {
        // Elf64_Shdr: name, SHT_PROGBITS, no flags (not loaded), addr 0, offset, size, link, info, align 1
        let mut h = Vec::with_capacity(shsize);
        h.extend_from_slice(&(name_at as u32).to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        for v in [0u64, 0, at as u64, size as u64] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        h.extend_from_slice(&[0u8; 8]);
        h.extend_from_slice(&1u64.to_le_bytes());
        h.extend_from_slice(&0u64.to_le_bytes());
        h.resize(shsize, 0);
        out.extend(h);
    }
    out[0x28..0x30].copy_from_slice(&(new_shoff as u64).to_le_bytes());
    out[0x3C..0x3E].copy_from_slice(&((shnum + new.len()) as u16).to_le_bytes());
    Ok(out)
}

/// the runtime made an AppImage runtime: "AI\x02" at byte 8, the sections .upd_info (the update
/// information), .sha256_sig and .sig_key (zero until signed); the image starts where it ends
pub(crate) fn runtime_image(elf: &[u8], info: &str) -> Result<Vec<u8>, String> {
    if info.len() >= UPD_INFO_SIZE {
        return Err(format!("appimage_update: longer than {} bytes", UPD_INFO_SIZE - 1));
    }
    if elf_end(elf)? != elf.len() {
        return Err("the AppImage runtime has data behind its ELF end (the image would be found in the wrong place)".to_string());
    }
    let mut b = elf.to_vec();
    b[8..11].copy_from_slice(b"AI\x02");
    let have = elf_sections(&b);
    let missing: Vec<(&str, usize)> = RUNTIME_SECTIONS.iter().filter(|(n, _)| !have.contains_key(*n)).copied().collect();
    if !missing.is_empty() {
        b = add_sections(&b, &missing)?;
    }
    let sec = elf_sections(&b);
    let (off, size) = sec[".upd_info"];
    b[off..off + size].fill(0);
    b[off..off + info.len()].copy_from_slice(info.as_bytes());
    for n in [".sha256_sig", ".sig_key"] {
        let (off, size) = sec[n];
        b[off..off + size].fill(0);
    }
    Ok(b)
}

fn mode(data: &[u8]) -> u16 {
    if data.starts_with(b"\x7fELF") || data.starts_with(b"#!") { 0o755 } else { 0o644 }
}

impl App<'_> {
    fn about(&self) -> About<'_> {
        About { display: &self.display, summary: self.summary(), license: self.license(), homepage: self.homepage(), version: &self.version }
    }

    /// the program of `style` as PREFIX/lib/PKG/PKG and the licences in PREFIX/share/doc/PKG/licenses/
    fn program_files(&self, style: &str, prefix: &str, pkg: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut out = BTreeMap::from([(format!("{prefix}lib/{pkg}/{pkg}"), package::read(&self.exe(style, "linux"))?)]);
        for (rel, d) in self.notices("linux")? {
            out.insert(format!("{prefix}share/doc/{pkg}/licenses/{rel}"), d.clone());
        }
        Ok(out)
    }

    /// the .desktop file, the icons and the own MIME types below PREFIX/share, named NAME
    fn desktop_files(&self, prefix: &str, name: &str, exe: &str, wm_class: Option<&str>) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut out = BTreeMap::from([(format!("{prefix}share/applications/{name}.desktop"),
                                       pkgmeta::desktop_entry(&self.meta, &self.display, exe, name, wm_class)?.into_bytes())]);
        for size in [48, 128, 256] {
            out.insert(format!("{prefix}share/icons/hicolor/{size}x{size}/apps/{name}.png"), self.icon(size)?);
        }
        let types = pkgmeta::filetypes(&self.meta)?;
        let own: Vec<_> = types.iter().filter(|t| t.own).collect();
        if !own.is_empty() {
            out.insert(format!("{prefix}share/mime/packages/{name}.xml"), pkgmeta::mime_package(&own).into_bytes());
        }
        Ok(out)
    }

    pub(crate) fn appimages(&self) -> Result<(), String> {
        let (pkg, aid) = (self.pkg()?, pkgmeta::app_id(&self.meta, &self.name)?);
        if self.homepage().is_empty() {
            self.note("no homepage (PACKAGE homepage= or Cargo.toml): appstreamcli validate warns, Flathub wants one");
        }
        println!("[linux] AppImage runtime");
        let runtime = runtime_image(&package::stub(self.p, &self.jflib, &APPIMAGERT)?, self.meta.or("appimage_update", ""))?;
        for style in &self.styles {
            println!("[linux] AppImage {style}");
            let mut files = self.program_files(style, "usr/", &pkg)?;
            files.extend(self.desktop_files("usr/", &aid, &pkg, Some(&pkg))?);
            files.insert(format!("{aid}.desktop"), files[&format!("usr/share/applications/{aid}.desktop")].clone());
            files.insert(format!("{aid}.png"), self.icon(256)?);
            files.insert(format!("usr/share/metainfo/{aid}.appdata.xml"), pkgmeta::metainfo(&self.meta, &self.about(), &aid, &pkg)?.into_bytes());
            let mut items: BTreeMap<String, Item> = files.into_iter().map(|(p, data)| (p, Item::File { mode: mode(&data), data })).collect();
            items.insert("AppRun".into(), Item::Link(format!("usr/lib/{pkg}/{pkg}")));
            items.insert(format!("usr/bin/{pkg}"), Item::Link(format!("../lib/{pkg}/{pkg}")));
            items.insert(".DirIcon".into(), Item::Link(format!("{aid}.png")));
            let mut out = runtime.clone();
            out.extend(squashfs::image(&items, pkgmeta::now() as u32)?);
            let path = self.dist.join(format!("{}-{}-{style}-x86_64.AppImage", self.name, self.version));
            package::write(&path, &out)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
            }
            self.made(path)?;
        }
        Ok(())
    }

    pub(crate) fn rpm(&self) -> Result<(), String> {
        println!("[linux] rpm");
        let pkg = self.pkg()?;
        let ver = self.version.replace('-', "~");
        if !ver.chars().all(|c| c.is_ascii_alphanumeric() || "._+~^".contains(c)) {
            return Err(format!("version {ver:?} is no RPM version"));
        }
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for s in &self.styles {
            files.insert(format!("usr/lib/{pkg}/{}-{s}", self.name), package::read(&self.exe(s, "linux"))?);
        }
        for (rel, d) in self.notices("linux")? {
            files.insert(format!("usr/share/doc/{pkg}/licenses/{rel}"), d.clone());
        }
        files.extend(self.desktop_files("usr/", &pkg, &pkg, None)?);
        let tarball = format!("{pkg}-{ver}-root.tar.gz");
        let top = self.dist.join("rpm");
        let tree: Tree = files.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        package::write(&top.join("SOURCES").join(&tarball), &deflate::gzip(&debfile::tar(&tree, pkgmeta::now(), false)))?;
        let spec = top.join("SPECS").join(format!("{pkg}.spec"));
        package::write(&spec, self.rpm_spec(&pkg, &ver, &tarball, &files)?.as_bytes())?;
        println!("  {}", spec.display());
        let linux = match Linux::with("rpmbuild") {
            Ok(l) => l,
            Err(why) => {
                self.skip("rpm", &format!("{why}; only the .spec and its tarball in {}", top.display()));
                return Ok(());
            }
        };
        let script = self.work.join("rpm.sh");
        package::write(&script, b"set -e\nt=$(mktemp -d /tmp/jflib-rpm-XXXXXX)\ntrap 'rm -rf \"$t\"' EXIT\n\
            mkdir -p \"$t/SPECS\" \"$t/SOURCES\" \"$t/BUILD\" \"$t/RPMS\" \"$t/SRPMS\" \"$t/BUILDROOT\"\n\
            cp \"$1\" \"$t/SPECS/\"\ncp \"$2\" \"$t/SOURCES/\"\n\
            rpmbuild -bb --define \"_topdir $t\" \"$t/SPECS/$(basename \"$1\")\" >&2\n\
            for f in \"$t\"/RPMS/*/*.rpm; do cp \"$f\" \"$3/\"; basename \"$f\"; done\n")?;
        let out = linux.run(&script, &[linux.path(&spec), linux.path(&top.join("SOURCES").join(&tarball)), linux.path(&self.dist)])?;
        if !out.status.success() {
            return Err(format!("rpmbuild ({}):\n{}", linux.name(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        for name in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.ends_with(".rpm")) {
            self.made(self.dist.join(name.trim()))?;
        }
        Ok(())
    }

    /// the .spec: a binary package from the prebuilt tree (Source0), the styles as alternatives
    fn rpm_spec(&self, pkg: &str, ver: &str, tarball: &str, files: &BTreeMap<String, Vec<u8>>) -> Result<String, String> {
        let text = |s: &str| s.replace('%', "%%");
        let quote = |p: &str| format!("\"/{}\"", p.replace('%', "%%"));
        let default = self.default_style("linux");
        let alt = "%{_sbindir}/update-alternatives";
        let install: Vec<String> = self.styles.iter().map(|s| format!("{alt} --install %{{_bindir}}/{pkg} {pkg} /usr/lib/{pkg}/{}-{s} {}",
                                                                       self.name, if s == default { 50 } else { 10 })).collect();
        let remove: Vec<String> = self.styles.iter().map(|s| format!("  {alt} --remove {pkg} /usr/lib/{pkg}/{}-{s} || :", self.name)).collect();
        // the folders the package owns
        let own = [format!("usr/lib/{pkg}"), format!("usr/share/doc/{pkg}")];
        let mut dirs = std::collections::BTreeSet::new();
        for f in files.keys() {
            let mut d = f.as_str();
            while let Some((parent, _)) = d.rsplit_once('/') {
                if own.iter().any(|o| parent == o || parent.starts_with(&format!("{o}/"))) {
                    dirs.insert(parent.to_string());
                }
                d = parent;
            }
        }
        let (y, m, d, wd) = pkgmeta::civil(pkgmeta::now());
        let when = format!("{} {} {d:02} {y}", ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wd as usize],
                           ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m as usize - 1]);
        let homepage = self.homepage();
        Ok(format!(
            "# {pkg}.spec -- written by cargo jflib package from PACKAGE; do not edit\n\
             # rpmbuild -bb --define \"_topdir $PWD\" SPECS/{pkg}.spec   (SOURCES/{tarball})\n\
             %global debug_package %{{nil}}\n%global __os_install_post %{{nil}}\n%global _build_id_links none\n\
             %global _missing_build_ids_terminate_build 0\n\n\
             Name:           {pkg}\nVersion:        {ver}\nRelease:        1\nSummary:        {}\nLicense:        {}\n{}\
             Source0:        {tarball}\nExclusiveArch:  x86_64\nAutoReqProv:    no\n{}\
             Requires(post): {alt}\nRequires(preun): {alt}\n\n%description\n{}\n\n\
             Enthält das Programm in {} Stil(en) ({}); voreingestellt ist {default},\nändern mit „sudo update-alternatives --config {pkg}“.\n\n\
             %prep\n%setup -q -c -T\n\n%build\n\n%install\nmkdir -p %{{buildroot}}\ntar -xzf %{{SOURCE0}} -C %{{buildroot}}\n\n\
             %post\n{}\n\n%preun\nif [ $1 -eq 0 ]; then\n{}\nfi\n\n%files\n{}{}\n%changelog\n\
             * {when} {} <packages@localhost> - {ver}-1\n- built by cargo jflib package\n",
            text(self.summary()), self.license(), if homepage.is_empty() { String::new() } else { format!("URL:            {homepage}\n") },
            if self.glibc()? { "Requires:       glibc\nRecommends:     vulkan-loader, mesa-vulkan-drivers\n" } else { "" },
            text(self.meta.or("description", self.summary())), self.styles.len(), self.styles.join(", "),
            install.join("\n"), remove.join("\n"),
            dirs.iter().map(|d| format!("%dir {}\n", quote(d))).collect::<String>(),
            files.keys().map(|f| format!("{}\n", quote(f))).collect::<String>(),
            text(self.meta.or("publisher", &self.display))))
    }

    pub(crate) fn flatpak(&self) -> Result<(), String> {
        println!("[linux] flatpak");
        let (pkg, aid) = (self.pkg()?, pkgmeta::app_id(&self.meta, &self.name)?);
        let style = self.default_style("linux");
        let mut files = self.program_files(style, "", &pkg)?;
        files.extend(self.desktop_files("", &aid, &pkg, Some(&pkg))?);
        files.insert(format!("share/metainfo/{aid}.metainfo.xml"), pkgmeta::metainfo(&self.meta, &self.about(), &aid, &pkg)?.into_bytes());
        let d = self.dist.join("flatpak");
        let _ = fs::remove_dir_all(d.join("files"));
        for (path, data) in &files {
            package::write(&d.join("files").join(path), data)?;
        }
        let mut extra: Vec<String> = self.meta.or("flatpak_args", "").split_whitespace().map(String::from).collect();
        if self.glibc()? && !extra.iter().any(|a| a == "--device=dri" || a == "--device=all") {
            extra.push("--device=dri".into()); // the windows go through the GPU
        }
        let (args, notes) = finish_args(pkgmeta::permissions(&self.p.package.dir).as_deref(), extra);
        for n in notes {
            println!("  [flatpak] {n}");
        }
        let runtime = self.meta.or("flatpak_runtime", "").to_string();
        let runtime = if runtime.is_empty() { format!("{FLATPAK_RUNTIME}//{FLATPAK_VERSION}") } else { runtime };
        let (name, branch) = runtime.split_once("//").unwrap_or((&runtime, FLATPAK_VERSION));
        let sdk = name.strip_suffix(".Platform").map(|n| format!("{n}.Sdk")).unwrap_or_else(|| "org.freedesktop.Sdk".to_string());
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let sh = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let mut cmds: Vec<String> = files.keys().filter(|p| !p.starts_with("share/"))
            .map(|p| format!("install -Dm{:o} {} {}", mode(&files[p]), sh(&format!("files/{p}")), sh(&format!("/app/{p}")))).collect();
        cmds.extend(["mkdir -p /app/bin".to_string(), format!("ln -s ../lib/{pkg}/{pkg} /app/bin/{pkg}"),
                     "cp -r --no-preserve=mode,ownership files/share /app/".to_string()]);
        let list = |v: &[String], indent: &str| v.iter().map(|a| format!("{indent}{}", q(a))).collect::<Vec<_>>().join(",\n");
        let manifest = format!(
            "{{\n  \"id\": {},\n  \"runtime\": {},\n  \"runtime-version\": {},\n  \"sdk\": {},\n  \"command\": {},\n  \"finish-args\": [\n{}\n  ],\n  \
             \"modules\": [\n    {{\n      \"name\": {},\n      \"buildsystem\": \"simple\",\n      \"build-commands\": [\n{}\n      ],\n      \
             \"sources\": [\n        {{\n          \"type\": \"dir\",\n          \"path\": \"files\",\n          \"dest\": \"files\"\n        }}\n      ]\n    }}\n  ]\n}}\n",
            q(&aid), q(name), q(branch), q(&sdk), q(&pkg), list(&args, "    "), q(&pkg), list(&cmds, "        "));
        let mf = d.join(format!("{aid}.json"));
        package::write(&mf, manifest.as_bytes())?;
        println!("  {}", mf.display());
        let linux = match Linux::with("flatpak-builder") {
            Ok(l) => l,
            Err(why) => {
                self.skip("flatpak", &format!("{why}; only the manifest {}", mf.display()));
                return Ok(());
            }
        };
        let script = self.work.join("flatpak.sh");
        package::write(&script, b"set -e\nid=\"$2\"\n\
            for r in \"$4//$5\" \"$6//$5\"; do\n  flatpak info \"$r\" >/dev/null 2>&1 || { echo \"the runtime $r is not installed (flatpak install --user flathub $r)\"; exit 3; }\ndone\n\
            t=$(mktemp -d /tmp/jflib-flatpak-XXXXXX)\ntrap 'rm -rf \"$t\"' EXIT\ncp -r \"$1\" \"$t/src\"\n\
            flatpak-builder --disable-rofiles-fuse --force-clean --disable-updates --state-dir=\"$t/state\" --repo=\"$t/repo\" \"$t/build\" \"$t/src/$id.json\" >&2\n\
            flatpak build-bundle \"$t/repo\" \"$t/out.flatpak\" \"$id\" >&2\ncp \"$t/out.flatpak\" \"$3\"\n")?;
        let bundle = self.dist.join(format!("{aid}.flatpak"));
        let out = linux.run(&script, &[linux.path(&d), aid.clone(), linux.path(&bundle), name.to_string(), branch.to_string(), sdk.clone()])?;
        match out.status.code() {
            Some(0) => self.made(bundle),
            Some(3) => {
                self.skip("flatpak", &format!("{} ({}); only the manifest", String::from_utf8_lossy(&out.stdout).trim(), linux.name()));
                Ok(())
            }
            _ => Err(format!("flatpak-builder ({}):\n{}", linux.name(), String::from_utf8_lossy(&out.stderr).trim())),
        }
    }
}

/// a PERMISSIONS path pattern -> (Flatpak filesystem location, note)
fn fs_location(pattern: &str) -> (Option<String>, Option<String>) {
    let b = pattern.as_bytes();
    if pattern.starts_with("//") || (b.len() > 2 && b[1] == b':' && b[2] == b'/') {
        return (None, None); // a Windows path
    }
    let (head, rest) = pattern.split_once('/').unwrap_or((pattern, ""));
    let sub: Vec<&str> = rest.split('/').take_while(|s| !s.is_empty() && !s.contains(['*', '?'])).collect();
    let sub = sub.join("/");
    if pattern == "**" {
        return (Some("host".into()), Some("** (every path): --filesystem=host".into()));
    }
    let inside = ["$CONFIG", "$APPIMAGE", "$APPCONFIG", "$APPDATA", "$APPCACHE", "$APPLOG", "$TEMP", "$EXE", "$FONTS"];
    if inside.contains(&head) {
        return (None, Some(format!("{pattern}: no hole needed (inside the sandbox)")));
    }
    let xdg = [("$HOME", "home"), ("$DESKTOP", "xdg-desktop"), ("$DOCUMENTS", "xdg-documents"), ("$DOWNLOADS", "xdg-download"),
               ("$PICTURES", "xdg-pictures"), ("$MUSIC", "xdg-music"), ("$VIDEOS", "xdg-videos")];
    if let Some((_, base)) = xdg.iter().find(|(v, _)| *v == head) {
        return (Some(match (*base, sub.is_empty()) {
            ("home", true) => "home".to_string(),
            ("home", false) => format!("~/{sub}"),
            (b, true) => b.to_string(),
            (b, false) => format!("{b}/{sub}"),
        }), None);
    }
    if pattern.starts_with('/') {
        return if sub.is_empty() { (Some("host".into()), Some(format!("{pattern}: the whole file system (--filesystem=host)"))) }
               else { (Some(format!("/{sub}")), None) };
    }
    (None, Some(format!("{pattern}: an unknown pattern, left out")))
}

/// the finish-args of a PERMISSIONS manifest (None: none, the app may do everything) -> (args, notes)
pub(crate) fn finish_args(perms: Option<&[(String, Vec<String>)]>, extra: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut args: Vec<String> = ["--share=ipc", "--socket=wayland", "--socket=fallback-x11"].map(String::from).to_vec();
    let mut notes = Vec::new();
    let Some(perms) = perms else {
        args.extend(["--share=network", "--filesystem=host", "--talk-name=org.freedesktop.secrets", "--socket=pulseaudio"].map(String::from));
        notes.push("no PERMISSIONS: wide rights (network, every file, Secret Service, sound server); a PERMISSIONS file narrows them".to_string());
        args.extend(extra);
        return (args, notes);
    };
    let push = |args: &mut Vec<String>, a: &str| {
        if !args.iter().any(|x| x == a) {
            args.push(a.to_string());
        }
    };
    let mut fs: BTreeMap<String, bool> = BTreeMap::new();
    for (key, words) in perms {
        match key.as_str() {
            "fs.read" | "fs.write" | "plugins" => {
                for w in words {
                    let (loc, note) = fs_location(w);
                    notes.extend(note);
                    if let Some(l) = loc {
                        let e = fs.entry(l).or_insert(false);
                        *e |= key == "fs.write";
                    }
                }
            }
            "net.connect" | "net.listen" => push(&mut args, "--share=network"),
            "secrets" => push(&mut args, "--talk-name=org.freedesktop.secrets"),
            "audio.output" => {
                push(&mut args, "--socket=pulseaudio");
                notes.push("audio.output: --socket=pulseaudio (the socket also allows recording)".into());
            }
            "process" if words.iter().any(|w| !w.starts_with("$EXE")) => {
                notes.push("process: programs outside the sandbox cannot be reached".into());
            }
            "fs.user" => notes.push("fs.user: files through the portals (file dialog, document portal)".into()),
            "url" | "print" | "autostart" | "input.global" | "screen.capture" => notes.push(format!("{key}: through the portal, no hole needed")),
            "recent" => push(&mut args, "--filesystem=xdg-data/recently-used.xbel"),
            _ => {}
        }
    }
    for (loc, write) in fs {
        args.push(format!("--filesystem={loc}{}", if write { "" } else { ":ro" }));
    }
    args.extend(extra);
    (args, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_args_of_a_manifest() {
        let perms = vec![("fs.read".to_string(), vec!["$PICTURES/**".to_string(), "C:/x".to_string()]),
                         ("fs.write".to_string(), vec!["$DOCUMENTS/Notes/**".to_string()]), ("net.connect".to_string(), vec![])];
        let (args, _) = finish_args(Some(&perms), vec!["--device=dri".into()]);
        assert_eq!(args, ["--share=ipc", "--socket=wayland", "--socket=fallback-x11", "--share=network",
                          "--filesystem=xdg-documents/Notes", "--filesystem=xdg-pictures:ro", "--device=dri"]);
        assert!(finish_args(None, vec![]).0.contains(&"--filesystem=host".to_string()));
    }

    #[test]
    fn a_runtime_gets_its_sections() {
        // a tiny ELF64: header, no segments, a section table of the null section and .shstrtab
        let mut b = vec![0u8; 64];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        let names = b"\0.shstrtab\0";
        b.extend_from_slice(names);
        b.resize(b.len().div_ceil(8) * 8, 0);
        let shoff = b.len();
        b.extend_from_slice(&[0u8; 64]);
        let mut sh = vec![0u8; 64];
        sh[0..4].copy_from_slice(&1u32.to_le_bytes());
        sh[4..8].copy_from_slice(&3u32.to_le_bytes());
        sh[24..32].copy_from_slice(&64u64.to_le_bytes());
        sh[32..40].copy_from_slice(&(names.len() as u64).to_le_bytes());
        b.extend(sh);
        b[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        b[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes());
        b[0x3C..0x3E].copy_from_slice(&2u16.to_le_bytes());
        b[0x3E..0x40].copy_from_slice(&1u16.to_le_bytes());
        let rt = runtime_image(&b, "zsync|https://example.org/x.zsync").unwrap();
        assert_eq!(&rt[8..11], b"AI\x02");
        assert_eq!(elf_end(&rt).unwrap(), rt.len());
        let sec = elf_sections(&rt);
        let (off, size) = sec[".upd_info"];
        assert_eq!(size, 1024);
        assert!(rt[off..].starts_with(b"zsync|https://"));
        assert_eq!(sec[".sig_key"].1, 8192);
    }
}
