// boot -- what build_rs.py builds into every program of an app, set up before the program's
// own code runs (docs/PERMISSIONS.md, docs/VFS.md)
//
//   start(&rt::boot::Boot) -> Result<(), exit code>   console programs (rt::main!): the
//                           process entry registers it with rt::boot; name, app://, manifest
//   name_and_assets(app, assets) -> Result<(), i32>    vfs::set_app_id, the JFA1 bundle as app://
//   permissions(manifest) -> Result<Option<Key>, i32>  install the manifest; with fs.user the
//                           paths on the command line are the user's (read + write, folders
//                           with everything below; a local file:// URI as its path); without
//                           a manifest the process's key (policy::take_key: it answers the
//                           questions of agent actions).  A `sandbox` line then hands the
//                           manifest to the OS as well (sandbox::apply)
//
// ui::main! calls name_and_assets and permissions itself (ui::boot, with the user's languages
// in between) and keeps the key; rt::main! programs drop it, so their code cannot widen the
// rules.  A refused bundle or manifest is reported on stderr: exit code 2.

use alloc::sync::Arc;

use crate::policy::{self, Key, FS_WRITE};
use crate::vfs;

/// (rt::main!, through rt::boot) set a console program up: name, assets, permissions
pub fn start(b: &rt::boot::Boot) -> Result<(), i32> {
    name_and_assets(b.app, b.assets)?;
    permissions(b.manifest).map(|_| ())
}

/// the app's name for its folders and its assets as app:// (Err: a damaged bundle)
#[doc(hidden)]
pub fn name_and_assets(app: Option<&str>, assets: &'static [u8]) -> Result<(), i32> {
    if let Some(id) = app {
        vfs::set_app_id(id);
    }
    if !assets.is_empty() {
        let Some(b) = vfs::Bundle::from_static(assets) else {
            crate::write_stderr(b"error: the built-in assets (app://) are damaged\n");
            return Err(2);
        };
        vfs::mount("app", Arc::new(b));
    }
    Ok(())
}

/// install the app's manifest (None: the app has none, everything stays allowed) and grant
/// the command line's paths; Some(key) for the caller to keep or drop (also without a
/// manifest: policy::take_key)
#[doc(hidden)]
pub fn permissions(manifest: Option<&str>) -> Result<Option<Key>, i32> {
    let Some(text) = manifest else { return Ok(policy::take_key()) };
    let key = match policy::install(text) {
        Ok(k) => k,
        Err(e) => {
            crate::write_stderr(alloc::format!("error: {}\n", e).as_bytes());
            return Err(2);
        }
    };
    for a in crate::args().iter().skip(1) {
        // "--name=value": the value; other options are no paths
        let v = match a.strip_prefix("--") {
            Some(o) => match o.split_once('=') {
                Some((_, v)) => v,
                None => continue,
            },
            None => a.as_str(),
        };
        if !v.is_empty() && !v.starts_with('-') {
            // a local file:// URI (Exec=... %U of a .desktop file) is the file it names
            match crate::assoc::file_uri_path(v) {
                Some(p) => grant_user(&key, &p),
                None => grant_user(&key, v),
            }
        }
    }
    // the OS sandbox (a `sandbox` line in the manifest): after the command-line grants, so the
    // kernel also allows the user's files; before the app's own code and any worker thread.
    // Not for a one-shot that only registers the app's file types and exits (it writes the
    // user's config folders, which the sandbox does not grant; no app code runs).
    let registering = crate::args().iter().skip(1).any(|a| matches!(a.as_str(), "--register" | "--register-replace" | "--unregister"));
    if !registering {
        crate::sandbox::apply();
    }
    Ok(Some(key))
}

/// a path the user handed over: a folder with everything below, else the file (with fs.user)
#[doc(hidden)]
pub fn grant_user(key: &Key, path: &str) {
    if key.trusted(|| crate::file_exists(path)) == 2 {
        key.grant_dir(path, FS_WRITE);
    } else {
        key.grant(path, FS_WRITE);
    }
}
