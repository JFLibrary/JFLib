// tr -- the console's languages: German source texts, English from locales/ui.tsv
//
//   init(lang)        install the catalogue and the language (also for the library's texts)
//   t(s)              a text in the current language
//   tf(s, args)       with "{}" placeholders filled
//   en()              English is active (number and date formats)
//   srv(s) / local(&mut s)   a text the server wrote (messages, titles, details: the server
//                     writes German, the demo server too) in the current language: SERVER
//                     knows it exactly or as a pattern whose "{}" parts (names, numbers,
//                     times) are taken over; unknown texts stay as they are

use alloc::string::String;
use alloc::vec::Vec;

use ui::i18n::{self, Catalog};

pub use ui::i18n::{LANG_DE, LANG_EN};

/// install the catalogue and select a language (LANG_DE / LANG_EN)
pub fn init(lang: u32) {
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")));
    i18n::set_language(lang);
}

/// the text in the current language
pub fn t(s: &'static str) -> &'static str {
    i18n::t(s)
}

/// the text with its "{}" placeholders filled in order
pub fn tf(s: &'static str, args: &[&str]) -> String {
    i18n::fill(i18n::t(s), args)
}

/// English is active
pub fn en() -> bool {
    i18n::language() == LANG_EN
}

/// a text the server wrote, in the current language (unchanged when German is active or
/// SERVER does not know it)
pub fn srv(s: &str) -> String {
    if !en() || s.is_empty() {
        return String::from(s);
    }
    server_text(s, true).unwrap_or_else(|| String::from(s))
}

/// translate a text the server wrote in place (see srv)
pub fn local(s: &mut String) {
    if en() && !s.is_empty() {
        if let Some(x) = server_text(s, true) {
            *s = x;
        }
    }
}

// the English of a server text: an exact entry first, then the first pattern that fits; the
// parts a pattern takes over are translated once more ("gestern 14:12" in a failover message)
fn server_text(s: &str, deep: bool) -> Option<String> {
    if let Some(k) = SERVER.iter().find(|k| **k == s) {
        return Some(String::from(t(k)));
    }
    for k in SERVER.iter().filter(|k| k.contains("{}")) {
        let Some(parts) = pattern_parts(k, s) else { continue };
        let args: Vec<String> = parts.iter().map(|a| if deep { server_text(a, false).unwrap_or_else(|| String::from(*a)) } else { String::from(*a) }).collect();
        let refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
        return Some(i18n::fill(t(k), &refs));
    }
    None
}

// the parts of `text` that stand for the "{}" of `pat` (none empty), or None when the text
// does not fit; a part ends where the pattern's next literal text first appears
fn pattern_parts<'a>(pat: &str, text: &'a str) -> Option<Vec<&'a str>> {
    let lits: Vec<&str> = pat.split("{}").collect();
    let (first, last) = (lits[0], lits[lits.len() - 1]);
    if text.len() < first.len() + last.len() || !text.starts_with(first) || !text.ends_with(last) {
        return None;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    let mut parts = Vec::new();
    for lit in &lits[1..lits.len() - 1] {
        let p = if lit.is_empty() { None } else { rest.find(lit) }?;
        if p == 0 {
            return None;
        }
        parts.push(&rest[..p]);
        rest = &rest[p + lit.len()..];
    }
    if rest.is_empty() {
        return None;
    }
    parts.push(rest);
    Some(parts)
}

// The texts of the server (server/ and the demo server fixture.rs) that the console shows:
// statuses and messages of hosts, repositories, sessions, VMs, tapes, replicas and watched
// servers, the overview's alerts, operation titles and results, audit details, session logs
// and error answers.  Their English is in locales/ui.tsv; patterns before the more general
// ones they would also fit.
// i18n
const SERVER: &[&str] = &[
    "Weniger als 10 % frei",
    "Der Ordner ist nicht erreichbar",
    "Keine Antwort seit 2 Tagen",
    "Keine Verbindung zum Agenten",
    "Abgeschlossen",
    "Restore-Point erstellt",
    "Dateien wiederhergestellt",
    "Alle Daten lesbar und unverändert",
    "{} ist fehlgeschlagen",
    "{} fehlgeschlagene Sicherungen in 24 h",
    "{} Sicherungen mit Warnungen in 24 h",
    "{} mit Warnungen",
    "Repository „{}“ fast voll",
    "Repository „{}“ nicht erreichbar",
    "{} % belegt, noch {} frei",
    "{} ist offline",
    "Letzter Kontakt vor 2 Tagen",
    "Agent nicht erreichbar",
    "{} Dateien waren gesperrt und wurden übersprungen",
    "{} Dateien waren gesperrt",
    "Agent {} nicht erreichbar (Zeitüberschreitung)",
    "Server {} nicht erreichbar",
    "Server {} lehnt die Überwachung ab",
    "Server {}: andere Zertifizierungsstelle",
    "{} Jobs, {} Restore-Points",
    "Verbindung abgelehnt ({})",
    "BackupCraft {} ist verfügbar",
    "Einstellungen › Über BackupCraft",
    "Heruntergeladen: {}",
    "Für eine spätere Version geplant",
    "E-Mail an {} gesendet",
    "Zusammenfassung",
    "Sitzung",
    "Nicht in der Bibliothek (ausgelagert?)",
    "Der Ordner {} fehlt",
    "läuft (KVM)",
    "läuft (ohne Hardware-Virtualisierung)",
    "Beendet; die Änderungen wurden verworfen",
    "Beendet; die zusammengeführten Platten bleiben",
    "Läuft von {}",
    "Die VM wurde im Gast heruntergefahren",
    "Der Server wurde neu gestartet",
    "Konsole",
    "läuft seit {} s",
    "„{}“ erschienen",
    "„{}“ nicht innerhalb von {} s",
    "keine Antwort innerhalb von {} s",
    "antwortet",
    "{} von {} Prüfungen bestanden",
    "{}: Zugriff verweigert auf {}",
    "Failover seit {}: {} auf {} ist in Gebrauch",
    "gestern {}",
    "Failover durch Plan „{}“",
    "Failback fehlgeschlagen: {}",
    "{} Replikate umgeschaltet",
    "{} umgeschaltet, nicht bereit: {}",
    "{} umgeschaltet, {} nicht: {}",
    "Zusammenführen fehlgeschlagen: {}",
    // operations: titles and results
    "{}: Sicherung",
    "{}: Aktive Vollsicherung",
    "{}: Wiederherstellung",
    "{}: Prüfung",
    "{}: Startprüfung {}",
    "{}: {} aus dem Restore-Point starten",
    "{}: zusammenführen",
    "{}: vom Band wiederherstellen",
    "{}: Bandsatz prüfen",
    "{}: auf Band ({})",
    "{}: auf Band",
    "Repository „{}“ prüfen",
    "Repository „{}“ aufräumen",
    "Konfiguration sichern",
    "Verbindung zu {} prüfen",
    "Keine Antwort innerhalb von 10 s",
    "Erreichbar, Zertifikat gültig",
    "Agent auf {} installieren",
    "Agent installiert und gekoppelt",
    "Agent wird installiert …",
    "{} freigegeben ({} Pakete gelöscht, {} neu geschrieben)",
    "{} freigegeben",
    "Keine Fehler gefunden",
    "Gesichert und verschlüsselt",
    "Replikat {}: Abgleich",
    "Abgeglichen: {} geändert, {} unverändert",
    "Replikat {}: Failover verwerfen",
    "Zurück auf den Restore-Point",
    "Replikat {}: Failback",
    "Zurückgeschrieben: {} Dateien",
    "Failover-Plan {}",
    "Band {} löschen",
    "Das Band ist leer",
    "{} Bänder gelesen ({} neu), {} Sätze katalogisiert",
    "Bandbibliothek einlesen",
    "{} Dateien vom Band wiederhergestellt",
    "{} Dateien von {} gelesen und geprüft",
    "{} Dateien auf {} geschrieben",
    "Die Platten wurden übertragen",
    "Die VM läuft",
    // audit details and targets
    "Anmeldung von {}",
    "Manuell gestartet",
    "Aktive Vollsicherung",
    "Aufbewahrung: {} Punkte, {} Tage",
    "{} Dateien nach {}",
    "Agent gekoppelt",
    "Falsches Passwort ({})",
    "Gesperrt: zu viele Fehlversuche",
    "Falscher Benutzername oder falsches Passwort",
    "Rolle: Betrachter",
    "Automatisch, verschlüsselt",
    "Restore-Points bleiben erhalten",
    "{} Einträge",
    "Freigegebene Ordner geändert",
    "Zertifikat gesperrt",
    "Pairing-Code erzeugt (15 min gültig)",
    "Neues Recovery-Kit erzeugt",
    "Die Dateien im Zielordner bleiben",
    "{} Bänder",
    "Demo-Server: nichts versendet",
    "Wird beim nächsten Start angewendet",
    // session logs
    "Sitzung gestartet: {}",
    "Vor-Skript ausgeführt: {} (Exitcode {})",
    "Quelle wird gelesen: {}",
    "Änderungen ermittelt (Größe, Änderungszeit, Datei-ID)",
    "Datei gesperrt, übersprungen: {}",
    "Sitzung abgebrochen, kein Restore-Point erstellt",
    "{} Dateien gelesen, {} neue Daten nach Dedupe und Kompression",
    "Keine Änderungen seit dem letzten Restore-Point: kein neuer Punkt",
    "Restore-Point signiert und veröffentlicht",
    "Signatur des Restore-Points geprüft",
    "Passphrase des Jobs im Repository hinterlegt",
    "Aufbewahrung angewendet: {} alter Punkt entfernt",
    "Aufbewahrung angewendet: {} alte Restore-Points entfernt",
    "Aufbewahrung: {}",
    "Sitzung beendet",
    // error answers
    "Unbekannter Pfad",
    "Bitte erneut anmelden",
    "Dafür fehlt die Berechtigung",
    "Diese Funktion ist in dieser Version noch nicht verfügbar",
    "Benutzername oder Passwort falsch",
    "Job nicht gefunden",
    "Ungültige Anfrage",
    "Der Name fehlt",
    "Ungültiger Zeitplan",
    "Der Job läuft bereits",
    "Restore-Point nicht gefunden",
    "Der Restore-Point ist noch unveränderlich",
    "Sitzung nicht gefunden",
    "Vorgang nicht gefunden",
    "Der Vorgang ist bereits beendet",
    "Host nicht gefunden",
    "Der Backup-Server selbst kann nicht entfernt werden",
    "Der Agent ist nicht erreichbar",
    "Die Adresse fehlt",
    "Bitte den Host-Schlüssel bestätigen",
    "Repository nicht gefunden",
    "Name und Pfad sind nötig",
    "Die Unveränderlichkeit kann nur verlängert werden",
    "Jobs verwenden dieses Repository noch",
    "Der Benutzername fehlt",
    "Das Passwort braucht mindestens 12 Zeichen",
    "Den Benutzer gibt es schon",
    "Benutzer nicht gefunden",
    "Der letzte Administrator kann nicht gelöscht werden",
    "Bitte einen Namen angeben",
    "Bitte einen Job wählen",
    "Bitte einen Zielhost wählen",
    "„{}“ liegt nicht in einem Wiederherstellungsordner von {}",
    "Replikat nicht gefunden",
    "Ein Failover-Plan verwendet dieses Replikat",
    "Das Replikat ist im Failover",
    "Nur aus dem Failover",
    "Das Replikat ist nicht bereit",
    "Bitte einen Namen und mindestens ein Replikat angeben",
    "Failover-Plan nicht gefunden",
    "Das Barcode-Präfix hat 2 bis 4 Buchstaben",
    "Band nicht gefunden",
    "Nur leere Bänder wechseln den Pool",
    "Das Band ist noch schreibgeschützt",
    "Medienpool nicht gefunden",
    "Der Pool hat beschriebene Bänder",
    "Bandsatz nicht gefunden",
    "Bitte einen Medienpool wählen",
    "Kein Band mit Platz im Pool",
    "Volumes von Agenten: in Vorbereitung (bisher nur die des Backup-Servers)",
    "Die VM läuft nicht aus einem Restore-Point",
    "Bitte eine VM wählen",
    "Bitte eine gültige Adresse angeben",
    "Überwachter Server nicht gefunden",
    "Bitte das Überwachungs-Token des Servers angeben",
    "Bitte zuerst die Zertifizierungsstelle des Servers bestätigen",
    "Bitte eine Bezeichnung angeben",
    "Überwachungs-Token nicht gefunden",
    "Im Demo-Server nicht vorhanden",
    "Ungültige Serveradresse",
];
