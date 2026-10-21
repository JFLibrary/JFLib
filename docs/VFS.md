# One file namespace (`plat::vfs`) and bundled assets

Every file function of `plat` also accepts names of the form `scheme://path`: `read_file`,
`write_file`, `file_exists`, `file_exists_checked`, `mkdir`, `delete`, `rename`, `dir_list`
and `trash`.
Everything built on them gets the virtual names for free, for example `img_core::io::load`
and `doc_io`'s stores. Code that opens files as streams works on every mount: `plat::File`
(`open`, `create_new`, `open_temp`) and `plat::FileLock`, so `base::Ini::load`, the font
registry's `load_file`, `datastore` and the log can use `mem://`, `app://` or a ZIP archive
as well as the folder mounts. `plat::fs` (`stat`, `read_dir`, links, times ...) takes the
names too. Names without a scheme stay paths of the OS, or of the browser's virtual files.
The same code runs on Windows, Linux, the web and Android; on Android the app's folders are
real files of its internal storage, and the documents the user picks in the system's picker
are `content://` names ([below](#android-internal-storage-and-content)).

```rust
let logo = plat::read_file("app://images/logo.png");          // built into the program
let mut ini = base::Ini::load("config://settings.ini");      // %APPDATA%/MyTool, ~/.config/MyTool
ini.set_int("view", "zoom", 150);
ini.save("config://settings.ini");                           // the folder is created
plat::write_file("cache://thumbs/1.png", &png);
plat::write_file("mem://scratch.txt", b"only in memory");
plat::write_file("documents://Report.txt", text.as_bytes());  // needs fs.write $DOCUMENTS/**
plat::trash("documents://old.txt");                          // into the recycle bin / trash
```

| Part | Where |
| --- | --- |
| Mounts, providers, bundle format | [`lib/plat/vfs.rs`](../lib/plat/vfs.rs) |
| The user's folders, the log folder | [`lib/plat/dirs.rs`](../lib/plat/dirs.rs) |
| The trash | [`lib/plat/trash.rs`](../lib/plat/trash.rs) |
| Routing in the file functions, `File`, `FileLock` and `AtomicFile` | [`lib/plat/guarded.rs`](../lib/plat/guarded.rs) |
| Files without blocking: chunked reads, atomic writes and copies with progress and cancellation | [`lib/plat/aio.rs`](../lib/plat/aio.rs) on [`lib/plat/task.rs`](../lib/plat/task.rs) |
| Routing in `plat::fs` | [`lib/plat/fs.rs`](../lib/plat/fs.rs) |
| ZIP archives as a tree | `archive::ZipFs` in [`lib/archive/lib.rs`](../lib/archive/lib.rs) |
| Packing `apps/NAME/assets` | [`tools/assetpack.py`](../tools/assetpack.py), `app_assets_hash` and `Builder.boot_args` in [`build_rs.py`](../build_rs.py) |
| Mounting at start-up | [`lib/plat/boot.rs`](../lib/plat/boot.rs): `ui::boot` ([`lib/ui/perm.rs`](../lib/ui/perm.rs), called by `ui::main!`) and `rt::main!` ([`lib/rt/boot.rs`](../lib/rt/boot.rs)) |
| `content://` (Android's documents), its host | [`lib/plat/content.rs`](../lib/plat/content.rs), [`lib/plat/android_host.rs`](../lib/plat/android_host.rs), [`android/GalleryHost.java`](../android/GalleryHost.java) |
| Android's internal storage | [`lib/plat/android_files.rs`](../lib/plat/android_files.rs) |
| Tests | `plattest --test content` ([`t_content.rs`](../apps/plattest/t_content.rs): `content://` against a storage provider in memory), on an emulator [`apps/androidGallery/tests/android_files.py`](../apps/androidGallery/tests/android_files.py), `plattest --test vfs` ([`t_vfs.rs`](../apps/plattest/t_vfs.rs): also streams, `plat::fs` on mounts, whiteouts), `--test async` ([`t_async.rs`](../apps/plattest/t_async.rs): `plat::aio` and `AtomicFile` on OS files, `mem://`, a folder mount and a gated mount), `--test dirs` ([`t_dirs.rs`](../apps/plattest/t_dirs.rs)), `--test trash` ([`t_trash.rs`](../apps/plattest/t_trash.rs)), [`apps/permtest`](../apps/permtest/main.rs) with its `assets/`, [`apps/permcli`](../apps/permcli/src/main.rs) (`app://` in console programs and `BINS`) |

## Built-in schemes

| Scheme | Contents | Windows | Linux | Browser | Android |
| --- | --- | --- | --- | --- | --- |
| `app://` | the app's assets (read-only) | inside the program | inside the program | inside the program | inside the APK's library |
| `config://` | settings | `%APPDATA%/<app>` | `$XDG_CONFIG_HOME/<app>` (`~/.config`) | `/config/<app>` | `/config/<app>` = `<filesDir>/fs/config/<app>` |
| `data://` | data of the app | `%LOCALAPPDATA%/<app>` | `$XDG_DATA_HOME/<app>` (`~/.local/share`) | `/data/<app>` | `/data/<app>` = `<filesDir>/fs/data/<app>` |
| `cache://` | data that can be rebuilt | `%LOCALAPPDATA%/<app>/Cache` | `$XDG_CACHE_HOME/<app>` (`~/.cache`) | `/cache/<app>` | `/cache/<app>` = `<cacheDir>/cache/<app>` |
| `logs://` | log files (`AF_LOG_FILE`) | `%LOCALAPPDATA%/<app>/Logs` | `$XDG_STATE_HOME/<app>` (`~/.local/state`) | `/data/<app>/Logs` | `/data/<app>/Logs` |
| `mem://` | files in memory, gone at exit | – | – | – | – |
| `content://` | documents of other apps the user chose | – | – | – | the system's picker (Storage Access Framework) |

`<app>` is `plat::vfs::app_id()`. `build_rs.py` sets it to the app's folder name
(`JFGUI_APP`). Otherwise it is the program's file name. `plat::vfs::app_dir(APP_CONFIG |
APP_DATA | APP_CACHE | APP_LOG)` returns these folders. `plat::data_dir()`,
`plat::cache_dir()` and `plat::log_dir()` return the base folders, like `config_dir()`.

The user's own folders have schemes too:

| Scheme | `plat::user_dir(...)` | Windows (Known Folder) | Linux (`user-dirs.dirs`) | Browser, Android |
| --- | --- | --- | --- | --- |
| `desktop://` | `DIR_DESKTOP` | Desktop | `XDG_DESKTOP_DIR` | – |
| `documents://` | `DIR_DOCUMENTS` | Documents | `XDG_DOCUMENTS_DIR` | `/documents` |
| `downloads://` | `DIR_DOWNLOADS` | Downloads | `XDG_DOWNLOAD_DIR` | – |
| `pictures://` | `DIR_PICTURES` | Pictures | `XDG_PICTURES_DIR` | – |
| `music://` | `DIR_MUSIC` | Music | `XDG_MUSIC_DIR` | – |
| `videos://` | `DIR_VIDEOS` | Videos | `XDG_VIDEOS_DIR` | – |

- **Windows** asks `SHGetKnownFolderPath`, so a folder the user moved (to another drive or to
  OneDrive) is found where it is now.
- **Linux** reads `$XDG_CONFIG_HOME/user-dirs.dirs` (`~/.config/user-dirs.dirs`, written by
  `xdg-user-dirs-update` in the user's language, for example `XDG_DOCUMENTS_DIR="$HOME/Dokumente"`).
  A value is `"$HOME/..."` or an absolute path; backslash escapes are undone, and the last
  line of a key wins. `"$HOME/"` (a disabled folder) is the home folder. Without the file, or
  for a key it lacks, the folder is `$HOME/Desktop`, `Documents`, `Downloads`, `Pictures`,
  `Music` or `Videos`.
- **Browser and Android** have only the virtual `/documents` folder; the other kinds return
  `None`, and their schemes are not mounted. On Android `/documents` is the app's own folder
  on internal storage (`<filesDir>/fs/documents`, kept across restarts, removed with the
  app): Android's shared Documents folder is reachable only through the system's picker, so
  documents the user should see elsewhere go through the file dialog (`content://`).

The folders need not exist; `write_file` and `mkdir` create missing folders.
`plat::user_dir_scheme(kind)` names the scheme of a kind.

The built-in schemes are mounted when they are first used. `config://`, `data://`,
`cache://`, `logs://` and the user's folders are folders of the OS:
`vfs::os_path("config://a.ini")` returns the real path, and `File::open`, `File::open_temp`,
`FileLock::acquire` and `plat::fs` open the real file there (checked against the app's
permissions like the path itself).

## Streams, metadata and locks on the other mounts

| Mount | `File::open` | `plat::fs::stat` | times | `FileLock` |
| --- | --- | --- | --- | --- |
| folder mounts (`DirFs`, `config://` ...) | the OS file | the OS file's | `set_times` on the OS file | the OS lock (other processes see it) |
| `mem://`, `MemFs` | a stream on the file's bytes: every handle and `read_file` see a write at once | kind, size, times | set by every write; `set_times` | inside this process |
| `app://`, `Bundle` | read only; a stored file is read in place, an LZ4 one unpacked once | kind, size | none (0) | inside this process, existing files only |
| `archive::ZipFs` | read only | kind, size | the entry's DOS time, read as UTC | inside this process, existing files only |
| `Overlay`, `Sub` | as their layers; an overlay writes to its top layer (FO_RW copies a lower file up first) | as the layer that shows the name | the top layer (a lower file is copied up) | as the layer |
| your own `Provider` without `open` | the whole file, read when it is opened and written back by `sync` and when the `File` is dropped | kind and size (the folder's listing) | `set_times` if it has it | inside this process |
| `content://` (Android, `plat::content::ContentFs`) | the document's file descriptor from the `ContentResolver` (positional; a provider that streams through a pipe: read whole at the start, written in order) | `vfs::stat`: kind, the provider's size and time (no `plat::fs` on Android) | none | inside this process |

- `File::create_new` is exclusive inside this process on mounts without OS files, and
  `File::open_temp(Some("mem://dir"))` returns a file in memory that has no name at all.
- `write_file` replaces a `mem://` file the way it does on disk: an open handle keeps the old
  bytes; `delete` and `rename` leave open handles alone.
- `plat::fs` exists on Windows and Linux. On mounts without OS files it has no links (`read_link` is `None`, `symlink` and
  `hard_link` fail), no owners or modes (`set_mode` / `set_owner` fail on Linux, `set_attrs` on
  Windows; the calls that do nothing on an OS succeed), no `free_space`. `stat` reports Linux
  modes `0644` / `0755` and `ctime` = `mtime`. `write_file_private` and `mkdir_private` are
  the mount's `write` and `mkdir`.
- A `symlink` target with a scheme stands for its OS path; links work on OS paths and folder
  mounts only.

## Writing in pieces, and without blocking

`plat::AtomicFile::create(name)` is `write_file` in pieces: `write_at` fills a sibling of the
target, `commit` puts it in place in one step, and dropping it without `commit` removes the
sibling, so readers only ever see the old file or all of the new one. On OS paths the sibling
is an exclusive `NAME.tmp-PID-SEQ-TIME` beside the file, flushed and renamed (Linux keeps the
access bits of the file it replaces; the browser keeps the bytes in memory until the commit).
On mounts it is a `NAME.tmp-..` file of the mount, opened like `File::open` and renamed by the
provider (`DirFs` renames the OS file, an `Overlay` writes its top layer, a provider without
`rename` gets the whole file written under the new name). A provider whose rename cannot
replace a file (`Provider::atomic_rename() == false`: `content://`) gets the bytes collected in
memory and written in place at `commit`. A folder at the name refuses.

`plat::aio` runs the file work on the blocking pool of `plat::task` and reports progress:

```rust
let ctl = plat::aio::Ctl::new();                     // progress (bytes) + cancel token + chunk size
let copy = plat::aio::copy_file("documents://big.iso", "data://big.iso", &ctl);
let task = ui.spawn(copy);                          // App::frame: ctl.progress.permille(), task.take()
ctl.cancel.cancel();                                // stops between two chunks; data://big.iso stays as it was
```

`read_file`, `write_file` and `copy_file` take a `Ctl`; `Reader::open(name)` / `read(max)` and
`Writer::create(name)` / `write(data)` / `commit()` work chunk by chunk, each chunk a job of the
pool. Names, mounts and permissions are those of the blocking functions (the same calls, made
on a pool thread; a caller inside `Key::trusted` stays trusted there). A cancelled, failed or
dropped write removes its sibling. `netio::Client::download(url, headers, name, &ctl)` writes an
HTTP body the same way.

## The trash

`plat::trash(path)` moves a file or folder into the trash instead of deleting it. `true`
means the item is in the trash and gone from its place. With `false` it stays where it was:
`trash` never deletes anything for good. Call `delete` yourself if the app wants that as a
fallback.

- **Windows**: the recycle bin (`IFileOperation` with `FOF_ALLOWUNDO` and
  `FOFX_RECYCLEONDELETE`, no dialogs). Where the shell would delete instead of recycling,
  for example on a drive without a recycle bin or for an item too big for it, the call
  refuses and returns `false`. The operation runs on a short-lived thread of its own,
  because `IFileOperation` needs a single-threaded COM apartment; the caller's thread keeps
  its own COM state.
- **Linux**: the [freedesktop trash](https://specifications.freedesktop.org/trash-spec/latest/).
  An item on the file system of `$XDG_DATA_HOME` goes to `$XDG_DATA_HOME/Trash`
  (`~/.local/share/Trash`); file managers show it there and can restore it. An item on
  another file system goes to its mount's top folder: `$topdir/.Trash/$uid` when the
  administrator created a sticky `.Trash`, else `$topdir/.Trash-$uid`. `info/NAME.trashinfo`
  holds the original `Path` (URL-escaped; relative to the top folder there) and the
  `DeletionDate`. A name already in the trash gets `.2`, `.3` ... before its extension
  (`a.txt`, `a.2.txt`). Nothing is copied across file systems.
- **Browser and Android**: there is no trash; `trash` returns `false`.

A name with a scheme works when its mount is a folder (`documents://old.txt`,
`data://x`, a `DirFs`). `trash` counts as writing the path ([PERMISSIONS.md](PERMISSIONS.md)).

## Paths inside a mount

- `/` separates, `\` is turned into `/`, and `.` and empty segments are dropped.
- `..` is refused: the name leads nowhere, and every call fails.
- `config:`, `config:/` and `config://` all name the root. Code that cuts a name at its last
  `/`, like `base::Ini` creating the parent folder, therefore stays inside the mount.
- A scheme has at least two characters: lower-case letters, digits, `+`, `-` and `.`.
  Windows drive letters (`C:/`) are never schemes.
- A scheme that is not mounted is an error, not a relative OS path. `https://` is no file.
- Writes create missing folders.
- `rename` works inside one mount. Across mounts (or between a mount and the OS) it fails;
  read and write instead.

## Mounting your own providers

```rust
use alloc::sync::Arc;
use plat::vfs::{self, DirFs, MemFs, Overlay, Sub};

// a project folder under its own scheme
vfs::mount("proj", Arc::new(DirFs::new("D:/Projects/Book")));

// user themes over the built-in ones: user files win, writes go to the user folder
let user = Arc::new(DirFs::new(&format!("{}/themes", vfs::app_dir(vfs::APP_DATA).unwrap())));
let builtin = Arc::new(Sub::new(vfs::provider("app").unwrap(), "themes").unwrap());
vfs::mount("theme", Arc::new(Overlay::new(vec![user, builtin])));

// a ZIP archive, read-only
let zip = archive::ZipFs::new(&bytes, archive::Limits::default())?;
vfs::mount("pack", Arc::new(zip));
let tex = plat::read_file("pack://textures/wood.png");
```

| Provider | Use |
| --- | --- |
| `MemFs::new()` | files in memory (also behind `mem://`) |
| `DirFs::new(root)` | an OS folder; goes through the checked OS functions |
| `Overlay::new(layers)` | several layers: reads from the first layer that has the path, lists every layer, writes to the top layer; deleting or renaming what a lower layer has leaves a whiteout (below) |
| `Sub::new(provider, folder)` | one folder of another provider as a root |
| `Bundle::new(bytes)` / `from_static` | a JFA1 bundle (read-only) |
| `archive::ZipFs::new(bytes, limits)` | a ZIP32 archive (stored / DEFLATE), checked and unpacked once |

Your own types implement `plat::vfs::Provider`: `kind`, `read` and `list`, and optionally
`write`, `mkdir`, `delete`, `rename`, `os_path`, `stat` and `set_times` (`vfs::Stat`), and
`open` for streams of their own (`vfs::Opened::File` with a `vfs::Stream`, `Refused`, or the
default `Generic`: the OS file of `os_path`, else the whole file), `display_name` (what the
user sees; `plat::display_name(name)` asks it, else the last segment) and `atomic_rename`
(false: `AtomicFile` writes in place). `mount` replaces an earlier mount of the
same scheme. `unmount` removes it. `provider(scheme)` returns a mount's provider, for example
to build a layer from `app://`.

<a id="android-internal-storage-and-content"></a>

## Android: internal storage and `content://`

Android builds keep the names of the browser's virtual files (`/config`, `/data`, `/cache`,
`/tmp`, `/documents`, `/imports`), so the permissions and the vfs see one layout, but the
files are real ([`android_files.rs`](../lib/plat/android_files.rs), Bionic): `/cache/..` and
`/tmp/..` live in the app's `cacheDir` (the system may clear the cache; `/tmp` is emptied when
the app starts), everything else in `<filesDir>/fs/`. They survive restarts and updates and
go with the app's data. `write_file` and `AtomicFile` write a sibling, `fsync` it and rename it
over the target; `rename` replaces a file in one step; `FileLock` is `flock`; `File::open_temp`
removes its name at once; writing needs the folder to exist, as on Linux (the app's folders
create theirs). `plat::web::persist(path)` syncs the file and is `Durable` at once.

The documents of other apps are `content://` names
([`content.rs`](../lib/plat/content.rs) over the `ContentResolver`,
[`android_host.rs`](../lib/plat/android_host.rs), [`GalleryHost.java`](../android/GalleryHost.java)).
The file dialog hands them out (`ui.filedlg` / `plat::file_dialog`, [GUIDE.md](GUIDE.md)
"File dialogs"): `FD_OPEN` is `ACTION_OPEN_DOCUMENT`, `FD_SAVE` `ACTION_CREATE_DOCUMENT` (the
document exists once the user saved), `FD_FOLDER` `ACTION_OPEN_DOCUMENT_TREE`. The grant the
user gave is taken as persistable, so the name works after a restart too (the app's recent
files list keeps them).

| Name | What it is | What works |
| --- | --- | --- |
| `content://AUTH/document/ID` | one document (open, save as) | `read_file`, `write_file` (in place, truncated), `File` streams, `AtomicFile` (in place), `vfs::stat`, `display_name` |
| `content://AUTH/tree/ID` | a folder the user chose | `dir_list`, `file_exists`, `display_name`; never deleted |
| `content://AUTH/tree/ID/a/b.txt` | below it, by display names (each looked up among the children) | everything above, plus `mkdir`, `delete` (files, empty folders), `rename` inside a folder (replacing: the target goes first, since providers pick `name (1)`), to another folder (copy, then delete; files only) |
| `content://AUTH/tree/ID/document/ID2` | Android's own form of a document in a tree | as a single document |

```rust
// the folder the user chose (ui.filedlg with FD_FOLDER), then files below it by name
let dir = chosen;                                       // content://com.android.externalstorage.documents/tree/primary%3ADocuments
plat::write_file(&format!("{dir}/Export/report.txt"), b"...");  // the folder Export is created
let name = plat::display_name(&dir);                    // "Documents", not "primary%3ADocuments"
let size = plat::vfs::stat(&format!("{dir}/Export/report.txt")).map(|s| s.size);
for (uri, read, write) in plat::web::content_grants() { /* the lasting grants */ }
plat::web::content_release(&old_uri);                   // Android keeps at most 128 / 512
```

Without a manifest every granted document is reachable; with one, `content://` needs
`fs.user` ([PERMISSIONS.md](PERMISSIONS.md)) -- Android itself lets the app open only what the
user granted. `plat::content::ContentFs` takes any `DocHost`, so the name logic is tested on
the desktop against a provider in memory (`plattest --test content`).

## Whiteouts in an overlay

An `Overlay` can hide what its lower layers have, so a user layer over the built-in themes can
also remove a file:

- `delete` of a lower layer's file (or of a folder that is empty in the merged view) writes a
  marker `.wh.NAME` next to it in the top layer. `kind`, `read`, `list`, `stat` and
  `File::open` then treat the name as missing, also everything below a hidden folder.
- `rename` copies a lower layer's file up under the new name and hides the old one. Folders
  can be renamed when only the top layer has them.
- Writing the name again (`write_file`, `File::open` for writing, `mkdir`) removes the marker.
  A hidden folder that is made again gets `.wh..wh..opq`: it is opaque and shows none of the
  lower layers' entries.
- The markers are files of the top layer (the names of aufs), so a `DirFs` on top keeps them
  on disk across runs. They are never listed, and names starting with `.wh.` cannot be
  written through the overlay.

```rust
let user = Arc::new(DirFs::new(&user_themes));
let builtin = Arc::new(Sub::new(vfs::provider("app").unwrap(), "themes").unwrap());
vfs::mount("theme", Arc::new(Overlay::new(vec![user, builtin])));
plat::delete("theme://dark/extra.css");   // gone from theme://, <user_themes>/dark/.wh.extra.css
```

## Assets (`apps/NAME/assets`)

1. Put the files into `apps/NAME/assets/`. Folders are kept; names starting with `.` are
   left out.
2. `python build_rs.py app NAME` packs the folder with `tools/assetpack.py` into
   `build/<target>/assets/NAME.jfa`. It passes `--cfg jfgui_assets` and the path
   (`JFGUI_ASSETS`) to `rustc`.
3. `ui::main!` and `rt::main!` embed the bundle with `include_bytes!` and mount it as
   `app://` (`plat::boot`), in every program of the app: the GUI program, the programs of
   `apps/NAME/BINS` and the Android APK. They are rebuilt when a file of the assets changes.

```text
python tools/assetpack.py apps/permtest/assets build/x.jfa      # pack by hand
python tools/assetpack.py --list build/x.jfa                    # size, packed size, codec, name
```

The bundle format JFA1 is a sorted index and the data. Each file is stored LZ4-compressed
when that saves at least a tenth, otherwise as it is. The LZ4 compressor of
`tools/assetpack.py` is the one of `lib/compress/lz4.rs`. `plat::vfs::Bundle` checks every
range of the index before it reads anything. Its `read` decompresses with
`compress::lz4::decompress`, which checks bounds. `plat::vfs::pack_bundle` writes the same
format from Rust.

## Permissions

`DirFs` and the app's folders go through the checked file functions, so
[the app's permissions](PERMISSIONS.md) apply to them. The app's own folders (`config://`,
`data://`, `cache://`, `logs://`) are always writable. `$APPCONFIG`, `$APPDATA`, `$APPCACHE`
and `$APPLOG` name them in a manifest. Android's `content://` documents need `fs.user` when
the app has a manifest. The user's folders are not allowed by themselves: an
app with a manifest needs a rule such as `fs.write $DOCUMENTS/**` or `fs.read $PICTURES/**`
before `documents://` or `pictures://` work. `app://`, `mem://` and other in-memory
providers touch no OS files.

## Limits

- `trash` on Linux writes the `DeletionDate` in UTC (`plat` has no local time zone on Linux;
  the specification asks for local time). It does not maintain the optional
  `directorysizes` cache. Items on other file systems need a writable top folder of their
  mount; otherwise `trash` returns `false`.
- On Windows, `trash` can only tell that the shell recycled the item; it cannot list or
  restore the recycle bin. On Linux the trash folders are ordinary folders and can be
  listed.
- A `FileLock` on a mount without OS files holds inside this process only; streams of your
  own providers without `open` hold the whole file in memory.
- Whiteouts work in the top layer of one `Overlay`; a lower layer's own markers mean nothing to
  it. Renaming a folder that a lower layer has fails.
- Document handles:
  - Flatpak's document portal: the portal's file dialog (`linux_portal.rs`) returns paths
    below `/run/user/UID/doc/`, which are ordinary OS paths and work with `fs.user` like any
    chosen file. A provider of their own is not needed for that, and none exists for
    handles kept across runs (the portal's document IDs).
  - Android's `content://`: writes are not atomic (SAF cannot rename over a document; a
    crash while writing can leave a part), `rename` over an existing file deletes it first,
    names below a tree are looked up anew on every call (no cache: each segment is one query),
    `FileLock` holds inside the process only, no times can be set, and the picker answers only
    while the app's process lives (one the system ended meanwhile drops the answer). Tested on
    the emulator against Android's storage provider (`android_files.py`); providers that
    stream through pipes only through the code path, not on a device.
  - The browser: the host copies a chosen file into `/imports` before the request completes
    (`pick_file`); writing back to the user's own file (File System Access API) needs a host
    request that does not exist yet.
