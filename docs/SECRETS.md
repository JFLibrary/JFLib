# Secrets -- passwords, tokens and keys in the system's store

`plat::secret` puts small secrets into the store the operating system offers for them: the
Credential Manager on Windows, the Secret Service (gnome-keyring, KWallet, KeePassXC) on Linux,
the Android Keystore on Android and, in the browser, a store of the web host (a WebCrypto key
that cannot be exported, kept in IndexedDB). An app should keep passwords, access and refresh
tokens, API keys and private keys there instead of writing them into its settings or data
files.

| Part | Where |
| --- | --- |
| The API, names, limits, errors | [`lib/plat/secret.rs`](../lib/plat/secret.rs) |
| Windows: Credential Manager, DPAPI files | [`lib/plat/win_secret.rs`](../lib/plat/win_secret.rs) |
| Linux: Secret Service on the session bus | [`lib/plat/linux_secret.rs`](../lib/plat/linux_secret.rs) (over [`linux_dbus.rs`](../lib/plat/linux_dbus.rs)) |
| Android: Keystore key, encrypted files | [`lib/plat/android_secret.rs`](../lib/plat/android_secret.rs), [`android/GallerySecrets.java`](../android/GallerySecrets.java) |
| Browser: WebCrypto key and values in IndexedDB | [`lib/plat/web_secret.rs`](../lib/plat/web_secret.rs), [`web/secrets.js`](../web/secrets.js) (loader.js, worker.js) |
| The manifest key `secrets` | [`lib/plat/policy.rs`](../lib/plat/policy.rs), [PERMISSIONS.md](PERMISSIONS.md) |
| AES decryption, constant-time `pow_ct` (DH) | [`lib/crypto/aes.rs`](../lib/crypto/aes.rs), [`lib/crypto/bn.rs`](../lib/crypto/bn.rs) |
| Tests | `plattest --test secret` ([`t_secret.rs`](../apps/plattest/t_secret.rs)), [`tools/platshared/secret_bus.py`](../tools/platshared/secret_bus.py), `coretest` (AES, DH vectors), [`android_secrets.py`](../apps/androidGallery/tests/android_secrets.py), [`secrets_web.mjs`](../apps/androidGallery/tests/secrets_web.mjs) |

## In an app

```text
# apps/MyMail/PERMISSIONS
secrets
net.connect imap.example.org:993
```

```rust
use plat::secret::{secret_available, secret_delete, secret_get, secret_set, SecretError};

// "remember the password" only where a store exists (and the manifest allows it)
let can_remember = secret_available();

match secret_set("imap.example.org", "alice", password.as_bytes()) {
    Ok(()) => {}
    Err(SecretError::Dismissed) => { /* the user closed the keyring's unlock dialog */ }
    Err(e) => self.error = Some(format!("Das Passwort wurde nicht gespeichert ({e}).")),
}

let saved = secret_get("imap.example.org", "alice");        // Option<Vec<u8>>
let _ = secret_delete("imap.example.org", "alice");         // Ok(true): there was one
```

* An entry is named by the app (`plat::vfs::app_id()`, the app's folder name), a **service**
  (not empty) and an **account** (may be empty: one value per service). Names are UTF-8 of at
  most `SECRET_NAME_MAX` (256) bytes without NUL; values are bytes, at most `SECRET_MAX`
  (64 KiB).
* `secret_set` replaces an existing value. `secret_get` returns `None` for a missing entry and
  on every error; `secret_get_checked` tells them apart (`Ok(None)` = missing).
* Errors (`SecretError`): `Unavailable` (no store here), `Denied` (no `secrets` in the
  manifest), `Dismissed` (the user dismissed the store's prompt, or it timed out), `Invalid`
  (bad name or value size), `Failed(text)` (what the store said).
* **The calls block.** On Linux a locked keyring shows its unlock dialog and the call waits for
  the answer, at most `PROMPT_SECONDS` (300 s). Call them from a worker thread (`plat::Job`)
  when the window must keep drawing meanwhile. Android answers in milliseconds on the UI
  thread; in the browser `secret_set` and `secret_delete` return before the change reaches
  IndexedDB (see "Browser").
* There is **no fallback**: without a store the calls fail, and nothing is written to a file in
  plain text instead. The app decides what to do (ask every time, or tell the user).

## Windows

* One generic credential (`CRED_TYPE_GENERIC`) per entry: target
  `jflib:<app>/<service>/<account>` (`%` and `/` inside the names are written as `%25` and
  `%2F`), user name = the account, comment `JFLib secret of <app>`. They are visible under
  Control Panel > Credential Manager > Windows Credentials and in `cmdkey /list`.
* Persistence `CRED_PERSIST_LOCAL_MACHINE`: the entry lives for this user on this computer
  and survives logoff and restarts. `CRED_PERSIST_ENTERPRISE` would additionally copy it to
  every computer the user's roaming profile reaches. Tokens are often bound to the device that
  obtained them, an API key should not travel unnoticed, and the Linux keyring is local too --
  so the local form is the default. `CRED_PERSIST_SESSION` would lose the entry at logoff.
* The Credential Manager keeps at most 2560 bytes per value. Larger values are encrypted with
  DPAPI (`CryptProtectData`: the user's logon key, the target as additional entropy, no UI)
  into `data://secrets/<16 hex of the target's SHA-256>-<16 hex of the file's SHA-256>.bin`.
  The credential then holds an empty value and the attribute `JFLib_File` with the file's
  SHA-256, so a changed, older or foreign file is refused (`Failed`). The new file is written
  before the credential points to it; afterwards the entry's other files are removed (also
  ones a crash left behind). Deleting the entry deletes its file.
* In a logon session without a credential store (some service accounts) the calls report
  `Unavailable` (`ERROR_NO_SUCH_LOGON_SESSION`).

## Linux

* The Secret Service API (`org.freedesktop.secrets`) on the session bus, through plat's own
  D-Bus client (no libsecret, no libc). Every call opens its own connection and session and
  closes both again, so the functions can be called from any thread.
* **Transport encryption:** `OpenSession("dh-ietf1024-sha256-aes128-cbc-pkcs7")`:
  Diffie-Hellman in the 1024-bit MODP group of RFC 2409 (generator 2, 128 random private bytes;
  `crypto::bn::Mont::pow_ct`, which neither branches on nor indexes by the exponent), the shared
  value as 128 bytes, HKDF-SHA256 without salt and info to the AES-128 key, the values AES-CBC
  with PKCS #7 padding and a random IV per value. Only when the service answers `NotSupported`
  does plat use the `plain` algorithm, as libsecret does; the values then cross the session
  bus (which belongs to the user) unencrypted, but they still end up only in the keyring.
* Items go into the **default collection** (`ReadAlias("default")`; the login keyring). Without
  one, plat creates it like libsecret does (`CreateCollection("Default keyring", "default")`,
  whose prompt asks the user for a password).
* Attributes: `xdg:schema = org.freedesktop.Secret.Generic`, `app`, `service`, `account`;
  label `<app>: <service> (<account>)`; content type `text/plain` for UTF-8 values, else
  `application/octet-stream`. `secret-tool lookup app <app> service <service> account <account>`
  finds the entries; Seahorse / KWallet Manager show and delete them.
* **Prompts:** a locked collection or item (`IsLocked`, locked search results) is unlocked with
  `Unlock`; when the service returns a prompt, plat runs it (`Prompt("")`) and waits for
  `Completed`. Prompts of `CreateItem`, `Delete` and `CreateCollection` (KeePassXC asks for
  confirmation) are run the same way. A dismissed prompt gives `Dismissed`; after
  `PROMPT_SECONDS` plat dismisses it itself.
* `secret_available()` asks the bus whether `org.freedesktop.secrets` has an owner or can be
  started (`NameHasOwner`, `ListActivatableNames`). Without a session bus or without the service
  (WSL, minimal systems, a headless server) everything reports `Unavailable`.

## Android

* **The key:** one AES-256 key per app in the `AndroidKeyStore` (alias `jfgui-secrets`), made
  on the first `secret_set` with `KeyGenParameterSpec` (purposes encrypt and decrypt, GCM, no
  padding, randomized encryption). The key material never leaves the Keystore (it cannot be
  exported; on devices with a TEE or StrongBox it is kept there, on the emulator in software),
  and the Keystore binds it to the app's uid: no other app reaches it.
* **No user authentication, also while the device is locked.** The key is not bound to the
  screen lock or to a biometric prompt (`setUserAuthenticationRequired(false)`, no
  `setUnlockedDeviceRequired`): the calls never show a dialog, and an app can read its token
  right after the start or in a background job. Such a binding would also invalidate the key
  when the user removes the screen lock, which loses every stored value. An app that wants a
  confirmation before each use asks the user itself (a key bound to authentication is not
  offered).
* **The values:** each one is encrypted with the key (AES-GCM, a random 12-byte IV that the
  Keystore picks, the 16-byte tag, the name `app\0service\0account` as additional data) into
  `<noBackupFilesDir>/jfgui-secrets/<SHA-256 of the name, hex>.bin` (`JFS1`, the IV, the
  ciphertext with its tag), written through `AtomicFile` (fsync, then rename). A file that was
  changed, or copied to another entry's name, does not decrypt: `secret_get_checked` reports
  `Failed`, and `secret_set` replaces it. `no_backup`, because Android's Auto Backup would
  otherwise restore the files onto a device whose Keystore lacks the key.
* `plat` asks the Java host synchronously (`GalleryHost.call` -> `GallerySecrets`, the path of
  the clipboard and `content://`; [`android_host.rs`](../lib/plat/android_host.rs)), on the UI
  thread where `plat` runs on Android. A thread without Java gets `Unavailable`.
* Clearing the app's data or uninstalling it removes the key and the files together.
  `secret_available()` is true where the `AndroidKeyStore` loads (Android 6 and newer).

## Browser

* **The store of the web host** ([`web/secrets.js`](../web/secrets.js)): the IndexedDB database
  `jfgui-secrets-v1`. Store `keys`: one AES-GCM key (256 bits) per app as a WebCrypto
  `CryptoKey` made with `extractable: false` -- the page can encrypt and decrypt with it, but
  the raw key cannot be read (`exportKey` refuses). Store `values`: per entry app, service,
  account, a random 12-byte IV and the ciphertext with its tag; the additional data is
  `app\0service\0account`, so a record copied to another name does not decrypt. The names are
  stored in plain (as in the Credential Manager), the values never.
* **Asynchronous underneath.** WebCrypto and IndexedDB only answer later, the API answers at
  once. The host decrypts the app's entries before the app starts and hands them to the WASM
  instance ([`web_secret.rs`](../lib/plat/web_secret.rs)); `secret_get` answers from that copy.
  `secret_set` and `secret_delete` change the copy at once and send the change to the host
  (`REQUEST_SECRET`), which encrypts and commits the changes one after the other, usually
  within a few milliseconds. A commit that fails (storage full, storage blocked) leaves the
  entry `Failed` for `secret_get_checked` until it is set or deleted again; a page closed
  within those milliseconds can lose the last change. Other open tabs of the app see a change
  after a reload.
* **What it protects against:** other origins (IndexedDB and the key belong to the page's
  origin), reading the browser profile's storage on disk or a copy of it (the files hold only
  ciphertext; the browser keeps the key, not as bytes the page can read), and casual
  inspection of the storage in the developer tools.
* **What it does not protect against:** script running in the same origin. Every script of
  the page -- another app served from the same origin, an injected script (XSS), a browser
  extension with access to the page -- can take the key from IndexedDB and decrypt every value,
  or read the decrypted copy in the app's memory. Serve an app with secrets from its own origin
  and keep its Content Security Policy strict. "Clear site data" deletes key and values
  together; a browser that evicts the origin's storage loses them too.
* `secret_available()` is false without a secure context (WebCrypto's `crypto.subtle` exists
  only on https and on `http://localhost` / `127.0.0.1`), without IndexedDB, or with a stored
  key that does not work; then every call is `Unavailable`, and nothing is written.

## Limits

* **No separation between the programs of one user.** The app prefix keeps JFLib apps apart
  through this API, but every native program of the same user can read Credential Manager
  entries and the items of an unlocked keyring. The store protects against other users,
  stolen disks (DPAPI and the keyring are tied to the user's login) and backups of the profile
  folder -- not against malware running as the user. Android separates apps by uid (another
  app reaches neither the key nor the files); code running inside the app, or as root, can use
  the key. In the browser the boundary is the origin (see "Browser").
* The manifest only allows or forbids the store as a whole (`secrets`); the app's own names
  are always reachable once it is allowed.
* The app id comes from the build (the app's folder name). Renaming the app folder renames
  its entries: the old ones stay behind until they are deleted.

## Tests

* `plattest --test secret` (in the test suite, both systems): Windows against the real
  Credential Manager -- set, get, overwrite, a second and an empty account, binary and empty
  values, names that differ in a `/`, Unicode names, another app id that neither reads nor
  deletes the entry, values above 2560 bytes as DPAPI files (one file, not in plain, a changed
  file refused, old files removed), 2560 bytes and `SECRET_MAX`, `SECRET_MAX + 1` refused,
  invalid names, delete. Every test entry is removed at the end (`cmdkey /list` shows no
  `jflib:` entries). On Linux the same against the session bus's Secret Service; without one
  (WSL) every call must return `Unavailable` and no file may appear.
* `python3 tools/platshared/secret_bus.py` (inside WSL, needs `python3-cryptography`): a fake
  Secret Service against `plattest --test secret-bus --mode M`: `dh` (the full round trip, the
  wire carries only AES-CBC), `plain`, `locked` (every call unlocks through a prompt),
  `dismiss` (nothing changes), `nodefault` (CreateCollection, prompts for CreateItem and
  Delete), `badkey` (an invalid public key of the service), `none` (no service on the bus).
* `plattest --test policy`: without `secrets` every call is `Denied` and recorded as
  `PERMDENY secrets <service>`; `permtest` has the key in its manifest.
* `coretest`: AES-128 / AES-256 decryption (FIPS 197 and random round trips), `pow_ct` against
  `pow`, the DH key exchange and the HKDF key of the Secret Service against Python.
* `python apps/androidGallery/tests/android_secrets.py --serial SERIAL --apk APK` (debug APK of
  `androidGallery`, emulator or device): clears the app's data; phase 1 runs the gallery's
  self-test (`jfgui_SELFTEST=secrets1`, [`secrets.rs`](../apps/androidGallery/secrets.rs): the
  contract of `plattest --test secret` -- set, get, overwrite, binary and empty values, names,
  another app id, 5000 / 40000 / `SECRET_MAX` bytes, `SECRET_MAX + 1` refused, delete), then
  `GallerySecretsInstrumentation` checks from outside: the key (AES, 256 bits, GCM, not
  exportable, no user authentication), three `JFS1` files named by the SHA-256, no value in
  plain in any file of the app; it damages one file. The shell user (another uid) may not list
  the folder, `run-as` (the app's uid) sees the three files. Phase 2, a new process: the values
  survived, the damaged one is `Failed` and can be set again, everything is deleted.
* `node apps/androidGallery/tests/secrets_web.mjs build` (after `build_rs.py app androidGallery
  --target web`; Playwright, Chromium and Firefox; `--threads` for the shared-memory variant):
  the gallery's card "05 Sicherer Speicher" through its accessibility tree -- store, load,
  overwrite, a reload, 5200 bytes, delete -- while IndexedDB is read the way another script of
  the origin reads it: one non-extractable AES-GCM key per app (`exportKey` refuses), records of
  IV + ciphertext + tag, no value in plain in any database; a changed record is `Failed` after
  a reload; without a secure context the card reports no store, its buttons are off and no
  database is made.
