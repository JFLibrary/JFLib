// "05 Sicherer Speicher": a value in the system's secret store (plat::secret: the Android
// Keystore, in the browser a WebCrypto key in IndexedDB, the Credential Manager / Secret
// Service on the desktop).  "Sichern" stores the field and empties it, "Laden" fills it again.
// With jfgui_SELFTEST=secrets1 / secrets2 (the start intent's extras of
// android/tests/GallerySecretsInstrumentation.java) the app checks the store and writes
// data://secrets-selftest.txt ("ok NAME" / "FAIL NAME: detail" lines, "done PASSED/TOTAL" last).

use alloc::{format, string::String, vec::Vec};
use plat::secret::{SECRET_MAX, SECRET_NAME_MAX, SecretError, secret_available, secret_delete, secret_get, secret_get_checked, secret_set};
use ui::*;

const SERVICE: &str = "galerie";
const ACCOUNT: &str = "demo";
const RESULTS: &str = "data://secrets-selftest.txt";

#[derive(Default)]
pub struct Secrets {
    loaded: bool,
    available: bool,
    value: String,
    info: String,
    test: Option<SelfTest>,
}

impl Secrets {
    pub fn card(&mut self, ui: &mut Ui) {
        if !self.loaded {
            self.loaded = true;
            // headless script runs leave the user's store alone
            self.available = !ui.headless() && secret_available();
            if let Some(t) = plat::env("jfgui_SELFTEST").filter(|t| t.starts_with("secrets")) {
                let mut test = SelfTest { phase2: t == "secrets2", log: Vec::new() };
                test.run();
                self.test = Some(test);
            }
        }
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.heading(2, TS_SUBTITLE, "05  Sicherer Speicher");
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, if self.available {
            "Der Wert liegt verschlüsselt im Geheimnisspeicher des Systems."
        } else {
            "Kein sicherer Speicher verfügbar."
        });
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(500, Some("Geheimnis"), &mut self.value, 8192);
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        for (id, label) in [(510, "Sichern"), (511, "Laden"), (512, "Löschen")] {
            ui.next_size(0, 44);
            if !self.available {
                ui.next_flags(ST_DISABLED);
            }
            if ui.button(id, label) & EV_ACT != 0 {
                self.act(id);
            }
        }
        ui.box_end();
        ui.text(TS_BODY | TX_WRAP, if self.info.is_empty() { "Noch nichts gesichert." } else { &self.info });
        if let Some(t) = &self.test {
            ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &t.status());
        }
        ui.box_end();
    }

    fn act(&mut self, id: u32) {
        self.info = match id {
            510 => match secret_set(SERVICE, ACCOUNT, self.value.as_bytes()) {
                Ok(()) => {
                    let n = self.value.len();
                    self.value.clear();
                    format!("Gesichert ({n} Bytes).")
                }
                Err(e) => format!("Nicht gesichert: {e}"),
            },
            511 => match secret_get_checked(SERVICE, ACCOUNT) {
                Ok(Some(v)) => {
                    self.value = String::from_utf8_lossy(&v).into_owned();
                    format!("Geladen ({} Bytes).", v.len())
                }
                Ok(None) => String::from("Kein Geheimnis gesichert."),
                Err(e) => format!("Nicht geladen: {e}"),
            },
            _ => match secret_delete(SERVICE, ACCOUNT) {
                Ok(true) => String::from("Gelöscht."),
                Ok(false) => String::from("Es war keines gesichert."),
                Err(e) => format!("Nicht gelöscht: {e}"),
            },
        };
    }
}

// ---------------------------------------------------------------- the self-test

const SVC: &str = "selbsttest";
// what phase 1 leaves for phase 2 (GallerySecretsInstrumentation looks for it in the files)
const KEPT: &str = "überlebt den Neustart ✓";

struct SelfTest {
    phase2: bool,
    log: Vec<(bool, String)>,
}

// n bytes that differ from seed to seed (GallerySecretsInstrumentation.big)
fn big(seed: u32, n: usize) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 13) as u8).collect()
}

fn is(v: Option<Vec<u8>>, want: &[u8]) -> bool {
    v.as_deref() == Some(want)
}

impl SelfTest {
    fn status(&self) -> String {
        let bad = self.log.iter().filter(|l| !l.0).count();
        format!("Selbsttest Speicher: {} Prüfungen, {} Fehler, fertig", self.log.len(), bad)
    }

    fn check(&mut self, ok: bool, name: &str) {
        let line = if ok { format!("ok {name}") } else { format!("FAIL {name}") };
        plat::write_stdout(format!("SELFTEST {line}\n").as_bytes());
        self.log.push((ok, line));
    }

    fn run(&mut self) {
        if self.phase2 {
            self.phase2();
        } else {
            self.phase1();
        }
        let mut out = String::new();
        for (_, l) in &self.log {
            out.push_str(l);
            out.push('\n');
        }
        let passed = self.log.iter().filter(|l| l.0).count();
        out.push_str(&format!("done {passed}/{}\n", self.log.len()));
        plat::write_file(RESULTS, out.as_bytes());
        plat::write_stdout(format!("SELFTEST done {passed}/{}\n", self.log.len()).as_bytes());
    }

    // the contract of plat::secret (as plattest --test secret), then the entries for phase 2
    fn phase1(&mut self) {
        let app = plat::vfs::app_id();
        self.check(secret_available(), "available");
        self.check(secret_get_checked(SVC, "alice") == Ok(None), "nothing there yet (Ok(None))");
        let (first, second): (&[u8], &[u8]) = (b"hunter2", b"correct horse battery staple");
        self.check(secret_set(SVC, "alice", first).is_ok(), "set");
        self.check(is(secret_get(SVC, "alice"), first), "get");
        self.check(secret_set(SVC, "alice", second).is_ok() && is(secret_get(SVC, "alice"), second), "overwrite");
        let bin: Vec<u8> = (0..=255u8).rev().collect();
        self.check(secret_set(SVC, "bob", &bin).is_ok() && is(secret_get(SVC, "bob"), &bin), "binary value, second account");
        self.check(is(secret_get(SVC, "alice"), second), "first account unchanged");
        self.check(secret_set(SVC, "", b"api-key").is_ok() && is(secret_get(SVC, ""), b"api-key"), "empty account");
        self.check(secret_set(SVC, "empty", b"").is_ok() && is(secret_get(SVC, "empty"), b""), "empty value");
        let slashed = format!("{SVC}/x");
        self.check(secret_set(&slashed, "y", b"one").is_ok() && secret_set(SVC, "x/y", b"two").is_ok()
            && is(secret_get(&slashed, "y"), b"one") && is(secret_get(SVC, "x/y"), b"two"), "names differing in a slash stay apart");
        self.check(secret_set(SVC, "ü ✓ 名前", "wert ✓".as_bytes()).is_ok() && is(secret_get(SVC, "ü ✓ 名前"), "wert ✓".as_bytes()), "Unicode names");

        // another app id neither reads nor deletes the entry; its own one of that name is apart
        plat::vfs::set_app_id("androidGalleryOther");
        self.check(plat::vfs::app_id() == "androidGalleryOther", "app id switched");
        self.check(secret_get_checked(SVC, "alice") == Ok(None), "another app does not read the entry");
        self.check(secret_delete(SVC, "alice") == Ok(false), "another app does not delete it");
        self.check(secret_set(SVC, "alice", b"other app").is_ok() && is(secret_get(SVC, "alice"), b"other app"), "the other app's own entry");
        self.check(secret_delete(SVC, "alice") == Ok(true), "the other app deletes its entry");
        plat::vfs::set_app_id(&app);
        self.check(is(secret_get(SVC, "alice"), second), "the app's entry is unchanged");

        // large values
        let (k5, k40, max) = (big(1, 5000), big(2, 40_000), big(3, SECRET_MAX));
        self.check(secret_set(SVC, "big", &k5).is_ok() && is(secret_get(SVC, "big"), &k5), "5000 bytes (over 4 KB)");
        self.check(secret_set(SVC, "big", &k40).is_ok() && is(secret_get(SVC, "big"), &k40), "40000 bytes");
        self.check(secret_set(SVC, "big", &max).is_ok() && is(secret_get(SVC, "big"), &max), "SECRET_MAX bytes");
        self.check(secret_set(SVC, "big", &big(4, SECRET_MAX + 1)) == Err(SecretError::Invalid), "SECRET_MAX + 1 -> Invalid");
        self.check(is(secret_get(SVC, "big"), &max), "the stored value is unchanged");
        self.check(secret_set(SVC, "big", b"small").is_ok() && is(secret_get(SVC, "big"), b"small"), "overwrite large with small");

        // names
        let long = "x".repeat(SECRET_NAME_MAX + 1);
        self.check(secret_set("", "a", b"x") == Err(SecretError::Invalid), "empty service -> Invalid");
        self.check(secret_set(SVC, "a\0b", b"x") == Err(SecretError::Invalid) && secret_get_checked(&long, "a") == Err(SecretError::Invalid),
            "NUL and too long names -> Invalid");
        let (s, a) = ("s".repeat(SECRET_NAME_MAX), "a".repeat(SECRET_NAME_MAX));
        self.check(secret_set(&s, &a, b"x").is_ok() && secret_delete(&s, &a) == Ok(true), "names of SECRET_NAME_MAX bytes");

        // delete
        self.check(secret_delete(SVC, "alice") == Ok(true), "delete");
        self.check(secret_get_checked(SVC, "alice") == Ok(None), "gone after delete");
        self.check(secret_delete(SVC, "alice") == Ok(false), "delete of a missing entry -> Ok(false)");
        let mut left = 0;
        for (s, a) in [(SVC, "bob"), (SVC, ""), (SVC, "empty"), (slashed.as_str(), "y"), (SVC, "x/y"), (SVC, "ü ✓ 名前"), (SVC, "big")] {
            if secret_delete(s, a) != Ok(true) || secret_get(s, a).is_some() {
                left += 1;
            }
        }
        self.check(left == 0, "every test entry removed");

        // for phase 2 (a new process): two values, one the instrumentation damages
        self.check(secret_set(SVC, "kept", KEPT.as_bytes()).is_ok(), "kept for the restart");
        self.check(secret_set(SVC, "kept-big", &big(7, 5000)).is_ok(), "5000 bytes kept for the restart");
        self.check(secret_set(SVC, "tamper", b"wird verfaelscht").is_ok(), "an entry to damage");
    }

    // after the restart: the values are there, the damaged one is refused, then all are removed
    fn phase2(&mut self) {
        self.check(secret_available(), "available after the restart");
        self.check(is(secret_get(SVC, "kept"), KEPT.as_bytes()), "restart: value kept");
        self.check(is(secret_get(SVC, "kept-big"), &big(7, 5000)), "restart: 5000 bytes kept");
        self.check(matches!(secret_get_checked(SVC, "tamper"), Err(SecretError::Failed(_))), "a damaged file is refused (Failed)");
        self.check(secret_get(SVC, "tamper").is_none(), "... get gives None");
        self.check(secret_set(SVC, "tamper", b"neu").is_ok() && is(secret_get(SVC, "tamper"), b"neu"), "set replaces the damaged entry");
        for a in ["kept", "kept-big", "tamper"] {
            self.check(secret_delete(SVC, a) == Ok(true) && secret_get_checked(SVC, a) == Ok(None), "restart: deleted");
        }
    }
}
