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
are `content://` names ([below](#android-internal-storage-and-content)). In a browser with the
File System Access API the user's own files are `/handles/ID/NAME`, written back to the file
([below](#the-browser-the-users-own-files)); in a Flatpak the documents portal's files open
again after a restart ([below](#flatpak-the-documents-portal)).

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
| The browser's files of the user (`/handles/`) | [`web/handles.js`](../web/handles.js), [`web/loader.js`](../web/loader.js), [`lib/plat/web.rs`](../lib/plat/web.rs), [`lib/plat/web_files.rs`](../lib/plat/web_files.rs) |
| Flatpak's documents portal, `plat::persistent_name` | [`lib/plat/linux_docs.rs`](../lib/plat/linux_docs.rs) over [`lib/plat/linux_dbus.rs`](../lib/plat/linux_dbus.rs) |
| Tests | `plattest --test content` ([`t_content.rs`](../apps/plattest/t_content.rs): `content://` against a storage provider in memory, the name cache, the journal), on an emulator [`apps/androidGallery/tests/android_files.py`](../apps/androidGallery/tests/android_files.py), in Chromium and Firefox [`apps/androidGallery/tests/files_web.mjs`](../apps/androidGallery/tests/files_web.mjs) (`/handles/`), in WSL [`tools/platshared/docs_bus.py`](../tools/platshared/docs_bus.py) (`plattest --test docs-bus`: a fake documents portal, `--real` xdg-document-portal and xdg-desktop-portal's file chooser for an app in a sandbox), `plattest --test vfs` ([`t_vfs.rs`](../apps/plattest/t_vfs.rs): also streams, `plat::fs` on mounts, whiteouts), `--test async` ([`t_async.rs`](../apps/plattest/t_async.rs): `plat::aio` and `AtomicFile` on OS files, `mem://`, a folder mount and a gated mount), `--test dirs` ([`t_dirs.rs`](../apps/plattest/t_dirs.rs)), `--test trash` ([`t_trash.rs`](../apps/plattest/t_trash.rs)), [`apps/permtest`](../apps/permtest/main.rs) with its `assets/`, [`apps/permcli`](../apps/permcli/src/main.rs) (`app://` in console programs and `BINS`) |

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
| `content://AUTH/document/ID` | one document (open, save as) | `read_file`, `write_file` (in place, through the journal, read back; below), `File` streams, `AtomicFile` (the same write at commit), `vfs::stat`, `display_name` |
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

**Writing a document.** SAF cannot replace a document by renaming another one over it (a
provider picks `name (1)`, and a new document gets a new ID, so the grants and the names the
app keeps would no longer reach it). `write_file` and `AtomicFile`'s commit therefore write the
document in place, as safely as SAF allows:

1. the new bytes and the document's URI go into the app's journal first:
   `data://.content-journal/<hash>.jw`, written atomically on internal storage;
2. the document is opened truncated (`"rwt"`, else `"wt"`), written, synced, then read back and
   compared with the bytes (size and contents; a second try if that fails);
3. the journal file goes. A crash, a killed process or a failed write leave it: the next time
   the process uses `content://`, `ContentFs::recover` writes it into the document again (if
   the document still exists and is granted; otherwise the copy is dropped).

The document keeps its URI, so the grants stay valid. What remains: while the write runs, other
apps can see a part; a crash leaves a part until the app starts again; a `File` stream opened
for writing writes in place without the journal (opening it, deleting or renaming the
document drops a pending copy). On the desktop `ContentFs::with_journal(dir)` takes any folder
(`plattest --test content` uses `mem://`).

**Names below a tree** are cached: each name keeps its document's URI once a walk or a listing
found it, so a name costs one query of the document itself instead of one listing per folder
(`plat::content::listings()` counts the listings, for tests). A cached document that no longer
answers (gone, renamed, no longer a file or folder) is walked again from the nearest cached
folder, a name missing below a cached folder only counts once that folder still answers to
its name, otherwise the walk starts at the tree's root. `mkdir`, `write`, `delete` and `rename`
through the provider keep the cache right; a change the provider reports (a `ContentObserver`
on its authority, registered when the app first lists a tree: `DocHost::generation`) empties
it, and an entry is used for `CACHE_TTL_US` (60 s) at most. A folder another app moved
elsewhere (with a stable document ID) can stay cached until then.

<a id="the-browser-the-users-own-files"></a>

## The browser: the user's own files

Chromium has the File System Access API; the host ([`web/handles.js`](../web/handles.js),
[`web/loader.js`](../web/loader.js)) tells the app so at start, and then:

- `ui.filedlg` / `plat::file_dialog` with `FD_OPEN` or `FD_SAVE` shows `showOpenFilePicker` /
  `showSaveFilePicker` (the dialog's types, the suggested name; a `/handles/` path as `path`
  starts beside that file) and answers later (`FD_PENDING`, `plat::file_dialog_poll`). The
  answer is `/handles/ID/NAME`, a path of the browser's virtual files bound to the handle of
  the chosen file; an opened file is read in before the answer. `FD_FOLDER` keeps the
  library's own dialog. `plat::web::pick_file` uses the open picker too (the files arrive as
  drops at `/handles/` paths).
- Every change of such a file is written back to the user's file: `write_file`, `AtomicFile`'s
  commit, `rename` onto it, a `File` stream's `sync` and drop each start a `persist` request
  for the revision, which the host writes with `createWritable` (the browser writes a swap file
  and puts it in place at `close()`: a failed or interrupted write leaves the file as it was).
  `plat::web::persist(path)` returns that request (`Durable` once written; `Denied` when the user
  refused the permission). Writes go one after the other per file. Imports are never written
  back.
- The handles are kept in IndexedDB (`jfgui-handles-v1`, at most 256 per app; the same file
  chosen again keeps its path). After a reload the host reads the remembered files whose
  permission the browser still grants (at most 64 MiB in all) before the app starts, so
  `read_file` and the recent-files list (`ui.recent_add`, kept in the browser's storage) work
  at once. Browsers ask again after a reload or a restart: `plat::web::reopen(path)`, called
  from the user's click or key (the browser wants a gesture), asks for the permission, reads
  the file in and completes (`Completed`, `Denied`, `Failed` for a file that is gone,
  `Unsupported` for an unknown path). Writing back asks for write permission the first time
  (from the gesture that saved).

```rust
// a file of the recent list after a reload
if plat::file_exists(&path) != 1 {
    self.reopening = plat::web::reopen(&path);          // from the click on the entry
}
// later frames: request_status(id) == Some(Completed) -> plat::read_file(&path)
plat::write_file(&path, &bytes);                        // written back into the user's file
```

With a manifest, `/handles/` paths need `fs.user` (no grant needed after a reload). Firefox
and Safari have no pickers: `file_dialog` returns -1, `ui.filedlg` shows the library's own
dialog over the virtual files (`/documents` ..., kept with `persist` in IndexedDB),
`pick_file` uses the `<input type=file>` copy into `/imports`, and the user gets a file out
with `plat::web::download(path)` (save as / download). Tested with
[`files_web.mjs`](../apps/androidGallery/tests/files_web.mjs): Chromium with the pickers
stubbed to hand out files of the origin private file system (open, save as, save over the opened
file, a failed write, a reload with the permission granted and with "prompt" granted / denied,
a cancelled picker), Firefox's own dialog.

<a id="flatpak-the-documents-portal"></a>

## Flatpak: the documents portal

Inside a Flatpak sandbox the files the user hands the app are documents of the documents
portal: OS paths below its FUSE mount, `$XDG_RUNTIME_DIR/doc/ID/NAME`. The portal's file dialog
(`linux_portal.rs`) exports the chosen file as a lasting document: the ID stays with the app's
permissions across log-outs and restarts (checked with xdg-desktop-portal 1.18: the document
is still there after the document portal restarted). With `fs.user` every path below the
mount is allowed without a grant inside the sandbox (the sandbox shows only what was given to
the app), so a file from the recent-files list opens again after a restart without a dialog.

`plat::persistent_name(path)` returns the name to keep for a chosen file; `ui.recent_add` keeps
that one. For a document below the mount it asks the portal
(`org.freedesktop.portal.Documents.Add` with an `O_PATH` descriptor, `reuse_existing` and
`persistent`; [`linux_docs.rs`](../lib/plat/linux_docs.rs) over the library's own bus client,
which now passes descriptors: `Conn::session_fds`, `Msg::fds`, `Value::H`) and returns
`<mount>/<ID>/<NAME>` of its answer. Where the portal can reach the real file this is a lasting
document of it; for a descriptor on its own mount the portal answers that document's own ID
(xdg-document-portal 1.18, Documents version 4: it refuses `reuse_existing` false and has no
`GetHostPaths`), so inside the sandbox the name stays what the file dialog gave. Every other
name comes back unchanged.

Tested in WSL by [`docs_bus.py`](../tools/platshared/docs_bus.py): a fake portal on a private bus
(the call, the descriptor, the flags, one bus connection only for document paths), and
`--real` in a private session (`dbus-run-session`, scratch `XDG_RUNTIME_DIR` and
`XDG_DATA_HOME`): xdg-document-portal's answers, a lasting and a session document across a
restart of the portal, and plattest inside a `bwrap` sandbox with a `/.flatpak-info` choosing a
file through xdg-desktop-portal's file chooser (a fake chooser behind it), reading it with a
manifest of `fs.user` only, then -- after the restart -- reading and writing it again without a
dialog.

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
  - Flatpak's documents portal: files of "Open with" (flatpak's file forwarding) and of the
    portal's file transfer are documents for the session only; the app cannot make them
    lasting (the portal answers a descriptor on its mount with the document's own ID), so after
    a log-out they need the dialog again. Folders the portal exports and files inside them are
    not asked about (`persistent_name` leaves them). Tested against the real portals in a
    private session and a `bwrap` sandbox, not with an installed Flatpak bundle (`flatpak run`).
  - Android's `content://`: a write is in place (journal and read-back above; others see a
    part while it runs), `rename` over an existing file deletes it first, a cached folder that
    another app moved elsewhere stays cached until a reported change or `CACHE_TTL_US`,
    changes of providers that report none are noticed only by the check of each document,
    `FileLock` holds inside the
    process only, no times can be set, and the picker answers only while the app's process
    lives (one the system ended meanwhile drops the answer). Tested on the emulator against
    Android's storage provider (`android_files.py`: saved over, cached names, a reported change,
    a write interrupted after six bytes and finished by the next start); providers that stream
    through pipes only through the code path, not on a device.
  - The browser: only Chromium has the pickers and handles (Firefox, Safari: the own dialog,
    `/imports`, downloads); `FD_FOLDER` and dropped files (`getAsFileSystemHandle`) are not bound
    to handles; a remembered file needs `reopen` from a gesture when the browser asks again; at
    most 64 MiB of remembered files are read in at the start, larger ones through `reopen`.
    The real pickers are not automated (`files_web.mjs` stubs them with files of the origin
    private file system).
