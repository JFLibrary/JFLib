# Secrets -- passwords, tokens and keys in the system's store

`plat::secret` puts small secrets into the store the operating system offers for them: the
Credential Manager on Windows and the Secret Service (gnome-keyring, KWallet, KeePassXC) on
Linux. An app should keep passwords, access and refresh tokens, API keys and private keys
there instead of writing them into its settings or data files.

| Part | Where |
| --- | --- |
| The API, names, limits, errors | [`lib/plat/secret.rs`](../lib/plat/secret.rs) |
| Windows: Credential Manager, DPAPI files | [`lib/plat/win_secret.rs`](../lib/plat/win_secret.rs) |
| Linux: Secret Service on the session bus | [`lib/plat/linux_secret.rs`](../lib/plat/linux_secret.rs) (over [`linux_dbus.rs`](../lib/plat/linux_dbus.rs)) |
| The manifest key `secrets` | [`lib/plat/policy.rs`](../lib/plat/policy.rs), [PERMISSIONS.md](PERMISSIONS.md) |
| AES decryption, constant-time `pow_ct` (DH) | [`lib/crypto/aes.rs`](../lib/crypto/aes.rs), [`lib/crypto/bn.rs`](../lib/crypto/bn.rs) |
| Tests | `plattest --test secret` ([`t_secret.rs`](../apps/plattest/t_secret.rs)), [`tools/platshared/secret_bus.py`](../tools/platshared/secret_bus.py), `coretest` (AES, DH vectors) |

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
  when the window must keep drawing meanwhile.
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

## Web and Android

Not available yet: `secret_available()` is `false` and every call returns
`SecretError::Unavailable`. For Android the Android Keystore is the candidate (a key in the
Keystore that encrypts the values in the app's private storage; [roadmap](ROADMAP.md) §1). The
browser has no store that would keep a value from the page's own scripts; a web host would
have to provide one.

## Limits

* **No separation between the programs of one user.** The app prefix keeps JFLib apps apart
  through this API, but every native program of the same user can read Credential Manager
  entries and the items of an unlocked keyring. The store protects against other users,
  stolen disks (DPAPI and the keyring are tied to the user's login) and backups of the profile
  folder -- not against malware running as the user.
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
