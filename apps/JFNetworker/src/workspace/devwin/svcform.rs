// svcform -- the settings of a lab service as a form in the Dienste tab (SA3): labels with units, a line
// of help, the rarer settings folded away; a backend or a target chosen from the project's devices
//
//   form_of(kind) -> Option<&[Group]>     the form of a kind (http-server, http-load, lb, kv, cache, mq,
//                                         worker, db), none for the others: their settings stay JSON;
//                                         a database's tables are the form's list, one row each
//   fields(kind)                          its fields in the order of the widgets (ID_SF_FIELD + 2 i)
//   fill(kind, &config) -> Vals           what the form shows for a config
//   merged(kind, &base, &vals) -> Result<Value, String>   the config: only what changed in the form
//                                         changes, every other member and its place stay; an empty
//                                         field takes the member out, so the service's default counts
//   default_text(kind, i, &defaults)      the default a field shows, asked of the service
//                                         (jfn_sim::svcform::defaults)
//   Takes, takes(&doc, dev, j, &config)   what applying would do: nothing, live or a restart; the same
//                                         doc/sim.rs live_steps that Doc::apply then runs
//   Form::of(&def) / view(n, ui, dev, j)  the open form below the card of service j; "Übernehmen"
//                                         checks through svcedit::trial and is one undo step, "Als
//                                         JSON bearbeiten" hands over what is typed; read-only in a
//                                         view of the past
//   free_name(device, kind)               a name for a new service
//   add_button / add_dialog               "Dienst hinzufügen": a kind and a name, one undo step, then
//                                         the form
//
// A service that lacks what it cannot do without (jfn_sim::svcform::missing: backends, a target) is
// taken as it is: it does not start and sends nothing, its card and the problems list say so.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::{empty_object, Value};
use agent_core::json::Number;
use jfn_model::experiment::{fixed, set_path};
use jfn_model::project::{Device, ServiceDef};
use jfn_sim::httpsvc::rate_ppm;
use jfn_sim::svcform::{defaults, missing, port_of};
use ui::*;

use super::svcedit;
use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::edits;
use crate::doc::sim::live_steps;
use crate::doc::Doc;

/// the width of the labels (DIP)
const LABEL_W: i32 = 164;
/// rows of a list at most (their ids)
const ROWS_MAX: usize = 32;

/// a word of a choice and what it means
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ch {
    pub word: &'static str,
    pub label: &'static str,
}

/// how a field reads and writes its member
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fk {
    /// a whole number
    Int,
    /// a number with decimals
    Dec,
    /// a share in percent, written "5%"
    Pct,
    /// on or off, or the default
    Flag,
    /// one of these words, or the default
    Choice(&'static [Ch]),
    /// a word or a line
    Text,
    /// a list of words, separated by commas
    Words,
    /// an http address, chosen from the devices (the kinds whose port it takes) or typed
    Url(&'static [&'static str]),
    /// the form's list (Vals::rows): the load generator's targets, a balancer's backends, a KV store's
    /// nodes
    List,
}

/// one setting of a form
#[derive(Clone, Copy)]
pub(crate) struct Field {
    /// its member, "health.path" inside another one
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub help: &'static str,
    pub kind: Fk,
}

/// settings that belong together; rare ones are folded away
pub(crate) struct Group {
    pub title: &'static str,
    pub rare: bool,
    pub fields: &'static [Field],
}

const HTTP_TARGET: &[&str] = &["http-server", "lb", "cache"];
const QUEUE_TARGET: &[&str] = &["mq"];

// i18n: label
const CH_ALGO: &[Ch] = &[Ch { word: "round-robin", label: "der Reihe nach" }, Ch { word: "least-conn", label: "wenigste offene Anfragen" }, Ch { word: "weighted", label: "nach Gewicht" }, Ch { word: "hash", label: "Schlüssel auf einem Ring" }];
// i18n: label
const CH_LB_MODE: &[Ch] = &[Ch { word: "http", label: "HTTP (je Anfrage)" }, Ch { word: "tcp", label: "TCP (je Verbindung)" }];
// i18n: label
const CH_HEALTH: &[Ch] = &[Ch { word: "http", label: "HTTP-Anfrage" }, Ch { word: "tcp", label: "TCP-Verbindung" }, Ch { word: "none", label: "keine" }];
// i18n: label
const CH_LOAD_MODE: &[Ch] = &[Ch { word: "closed", label: "geschlossen: Nutzer warten" }, Ch { word: "open", label: "offen: feste Rate" }];
// i18n: label
const CH_ARRIVAL: &[Ch] = &[Ch { word: "uniform", label: "gleichmäßig" }, Ch { word: "poisson", label: "zufällig (Poisson)" }];
// i18n: label
const CH_THINK: &[Ch] = &[Ch { word: "fixed", label: "fest" }, Ch { word: "exp", label: "zufällig (exponentiell)" }];
// i18n: label
const CH_BALANCE: &[Ch] = &[Ch { word: "round-robin", label: "der Reihe nach" }, Ch { word: "random", label: "zufällig" }];
// i18n: label
const CH_CONSISTENCY: &[Ch] = &[Ch { word: "strong", label: "stark: Mehrheit" }, Ch { word: "eventual", label: "schwach: ein Knoten" }];

// i18n: label
const CH_DB_DIST: &[Ch] = &[Ch { word: "uniform", label: "gleichmäßig" }, Ch { word: "zipf", label: "Zipf: wenige Zeilen oft" }];
// i18n: label
const CH_DURABILITY: &[Ch] = &[Ch { word: "full", label: "voll: Antwort erst nach dem Schreiben" }, Ch { word: "relaxed", label: "locker: Antwort sofort, Verlust möglich" }];
// i18n: label
const CH_ISOLATION: &[Ch] = &[Ch { word: "read committed", label: "read committed: je Anweisung neu" }, Ch { word: "repeatable read", label: "repeatable read: ein Stand je Transaktion" }];

// what holds a service in memory, the same in every form
// i18n: label unit help
const MEM_FIELDS: [Field; 3] = [
    Field { key: "mem_mb", label: "Speicher des Dienstes", unit: "MiB", help: "So viel hält er immer im RAM, wenn das Gerät Hardware simuliert.", kind: Fk::Dec },
    Field { key: "conn_kb", label: "Speicher je Verbindung", unit: "KiB", help: "So viel kostet jede offene Verbindung zusätzlich.", kind: Fk::Dec },
    Field { key: "data_mb", label: "Speicher für Daten", unit: "MiB", help: "So viel hält er für seine Daten dazu.", kind: Fk::Dec },
];

// i18n: title label unit help
static HTTP_SERVER: [Group; 2] = [
    Group {
        title: "Anfragen",
        rare: false,
        fields: &[
            Field { key: "port", label: "Port", unit: "", help: "Hier nimmt der Webserver Verbindungen an.", kind: Fk::Int },
            Field { key: "cpu_ms", label: "CPU je Anfrage", unit: "ms", help: "Rechenzeit einer Anfrage auf einem Referenzkern; sie zählt nur mit simulierter Hardware.", kind: Fk::Dec },
            Field { key: "time_ms", label: "Wartezeit je Anfrage", unit: "ms", help: "So lange wartet eine Anfrage auf etwas anderes und belegt dabei keine CPU.", kind: Fk::Dec },
            Field { key: "workers", label: "Worker", unit: "", help: "So viele Anfragen laufen gleichzeitig; 0 heißt ohne Grenze.", kind: Fk::Int },
            Field { key: "queue", label: "Warteschlange", unit: "Anfragen", help: "So viele warten auf einen Worker, die nächste bekommt sofort 503; 0 heißt ohne Grenze.", kind: Fk::Int },
            Field { key: "serial_us", label: "Sperre je Anfrage", unit: "µs", help: "Dieser Teil läuft nur nacheinander und setzt die obere Grenze des Durchsatzes.", kind: Fk::Dec },
            Field { key: "error_rate", label: "Fehlerrate", unit: "%", help: "Dieser Anteil der Anfragen bekommt eine Fehlerantwort.", kind: Fk::Pct },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "jitter_ms", label: "Streuung der Wartezeit", unit: "ms", help: "Bis zu so viel kommt zufällig zur Wartezeit dazu.", kind: Fk::Dec },
            Field { key: "error_status", label: "Fehlerstatus", unit: "", help: "Dieser HTTP-Status geht mit der Fehlerrate zurück.", kind: Fk::Int },
            Field { key: "keepalive_ms", label: "Verbindung offen halten", unit: "ms", help: "So lange bleibt eine Verbindung ohne Anfrage offen.", kind: Fk::Dec },
            Field { key: "max_requests", label: "Anfragen je Verbindung", unit: "", help: "Danach schließt der Server die Verbindung; 0 heißt ohne Grenze.", kind: Fk::Int },
            Field { key: "header_timeout_ms", label: "Zeit für den Kopf", unit: "ms", help: "So lange darf das Lesen des Anfragekopfs dauern.", kind: Fk::Dec },
            Field { key: "max_body", label: "Größte Anfrage", unit: "Byte", help: "Größere Anfragen weist der Server ab.", kind: Fk::Int },
            Field { key: "backlog", label: "Wartende Verbindungen", unit: "", help: "So viele neue Verbindungen warten auf ihre Annahme; 0 heißt die Vorgabe des Systems.", kind: Fk::Int },
            Field { key: "server", label: "Server-Kopfzeile", unit: "", help: "Dieser Name steht in den Antworten.", kind: Fk::Text },
            Field { key: "root", label: "Ordner für Dateien", unit: "", help: "Ein absoluter Pfad des Geräts: was keine Route nimmt, kommt von dort.", kind: Fk::Text },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
            MEM_FIELDS[2],
        ],
    },
];

// i18n: title label unit help
static HTTP_LOAD: [Group; 2] = [
    Group {
        title: "Last",
        rare: false,
        fields: &[
            Field { key: "targets", label: "Ziele", unit: "", help: "Wohin die Anfragen gehen: ein Gerät wählen oder eine Adresse eintippen.", kind: Fk::List },
            Field { key: "mode", label: "Art der Last", unit: "", help: "Geschlossen: eine feste Zahl Nutzer, jeder wartet auf seine Antwort. Offen: eine feste Rate, egal wie schnell die Antworten kommen.", kind: Fk::Choice(CH_LOAD_MODE) },
            Field { key: "users", label: "Nutzer", unit: "", help: "Bei geschlossener Last: so viele fragen gleichzeitig.", kind: Fk::Int },
            Field { key: "think_ms", label: "Denkzeit", unit: "ms", help: "Bei geschlossener Last: so lange wartet ein Nutzer vor der nächsten Anfrage.", kind: Fk::Dec },
            Field { key: "rps", label: "Rate", unit: "Anfragen/s", help: "Bei offener Last: so viele Anfragen je Sekunde.", kind: Fk::Dec },
            Field { key: "timeout_ms", label: "Zeitgrenze", unit: "ms", help: "Keine Antwort bis dahin zählt als Fehler. Kurz unter Überlast kann das den Durchsatz bis auf 0 drücken.", kind: Fk::Dec },
            Field { key: "max_conns", label: "Verbindungen je Ziel", unit: "", help: "Darüber warten Anfragen beim Generator; 0 heißt ohne Grenze.", kind: Fk::Int },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "arrival", label: "Ankünfte", unit: "", help: "Bei offener Last: gleichmäßig oder zufällig verteilt.", kind: Fk::Choice(CH_ARRIVAL) },
            Field { key: "think", label: "Art der Denkzeit", unit: "", help: "Fest oder zufällig um den Mittelwert.", kind: Fk::Choice(CH_THINK) },
            Field { key: "balance", label: "Mehrere Ziele", unit: "", help: "Wie die Anfragen sich auf die Ziele verteilen.", kind: Fk::Choice(CH_BALANCE) },
            Field { key: "ramp_ms", label: "Anlaufzeit", unit: "ms", help: "Über diese Zeit steigt die Rate an.", kind: Fk::Dec },
            Field { key: "start_rps", label: "Rate am Anfang", unit: "Anfragen/s", help: "Von hier aus steigt die Rate während der Anlaufzeit.", kind: Fk::Dec },
            Field { key: "delay_ms", label: "Start nach", unit: "ms", help: "So lange wartet der Generator nach dem Start des Geräts.", kind: Fk::Dec },
            Field { key: "duration_ms", label: "Dauer", unit: "ms", help: "Danach hört er auf; 0 heißt ohne Ende.", kind: Fk::Dec },
            Field { key: "requests", label: "Anfragen insgesamt", unit: "", help: "Danach hört er auf; 0 heißt ohne Grenze.", kind: Fk::Int },
            Field { key: "connect_timeout_ms", label: "Zeit für den Aufbau", unit: "ms", help: "So lange darf eine neue Verbindung dauern; 0 heißt nur die Zeitgrenze.", kind: Fk::Dec },
            Field { key: "keepalive", label: "Verbindungen weiter nutzen", unit: "", help: "Aus: jede Anfrage baut eine neue Verbindung auf.", kind: Fk::Flag },
        ],
    },
];

// i18n: title label unit help
static LB: [Group; 2] = [
    Group {
        title: "Verteilen",
        rare: false,
        fields: &[
            Field { key: "backends", label: "Backends", unit: "", help: "An diese Server verteilt der Load Balancer: ein Gerät wählen oder eine Adresse eintippen.", kind: Fk::List },
            Field { key: "mode", label: "Art", unit: "", help: "HTTP verteilt jede Anfrage, TCP jede Verbindung.", kind: Fk::Choice(CH_LB_MODE) },
            Field { key: "port", label: "Port", unit: "", help: "Hier nimmt er Anfragen an.", kind: Fk::Int },
            Field { key: "algorithm", label: "Verfahren", unit: "", help: "Wie er das nächste Backend wählt.", kind: Fk::Choice(CH_ALGO) },
            Field { key: "health.type", label: "Prüfung der Backends", unit: "", help: "Woran er merkt, dass ein Backend nicht mehr antwortet.", kind: Fk::Choice(CH_HEALTH) },
            Field { key: "health.path", label: "Pfad der Prüfung", unit: "", help: "Diesen Pfad fragt die HTTP-Prüfung ab.", kind: Fk::Text },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "hash", label: "Schlüssel", unit: "", help: "Beim Ring: path, client oder header:NAME.", kind: Fk::Text },
            Field { key: "vnodes", label: "Punkte je Backend", unit: "", help: "Beim Ring: so oft steht jedes Backend darauf; mehr verteilt gleichmäßiger.", kind: Fk::Int },
            Field { key: "retries", label: "Wiederholungen", unit: "", help: "So oft versucht er ein anderes Backend, wenn eines nicht erreichbar ist.", kind: Fk::Int },
            Field { key: "timeout_ms", label: "Zeitgrenze", unit: "ms", help: "So lange wartet er auf ein Backend, dann antwortet er 504.", kind: Fk::Dec },
            Field { key: "connect_timeout_ms", label: "Zeit für den Aufbau", unit: "ms", help: "So lange darf eine Verbindung zum Backend dauern.", kind: Fk::Dec },
            Field { key: "drain_ms", label: "Auslaufen", unit: "ms", help: "So lange darf ein entferntes Backend zu Ende arbeiten.", kind: Fk::Dec },
            Field { key: "max_conns", label: "Verbindungen je Backend", unit: "", help: "Darüber nimmt er dieses Backend nicht; 0 heißt ohne Grenze.", kind: Fk::Int },
            Field { key: "keepalive", label: "Verbindungen weiter nutzen", unit: "", help: "Aus: jede Anfrage baut eine neue Verbindung zum Backend auf.", kind: Fk::Flag },
            Field { key: "tag", label: "Backend nennen", unit: "", help: "Die Antwort sagt in X-Served-By, wer sie gemacht hat.", kind: Fk::Flag },
            Field { key: "cpu_us", label: "CPU je Anfrage", unit: "µs", help: "Auch ein Load Balancer kann der Engpass sein.", kind: Fk::Dec },
            Field { key: "health.interval_ms", label: "Prüfen alle", unit: "ms", help: "So oft prüft er jedes Backend.", kind: Fk::Dec },
            Field { key: "health.timeout_ms", label: "Zeit für die Prüfung", unit: "ms", help: "Keine Antwort bis dahin zählt als gescheitert.", kind: Fk::Dec },
            Field { key: "health.rise", label: "Wieder da nach", unit: "Prüfungen", help: "So viele Prüfungen in Folge müssen gelingen.", kind: Fk::Int },
            Field { key: "health.fall", label: "Ausgefallen nach", unit: "Prüfungen", help: "So viele Prüfungen in Folge müssen scheitern.", kind: Fk::Int },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
        ],
    },
];

// i18n: title label unit help
static KV: [Group; 2] = [
    Group {
        title: "Speicher",
        rare: false,
        fields: &[
            Field { key: "nodes", label: "Knoten", unit: "", help: "Alle Knoten des Speichers, dieser eingeschlossen: ein Gerät wählen oder eine Adresse eintippen.", kind: Fk::List },
            Field { key: "rf", label: "Kopien je Schlüssel", unit: "", help: "Auf so vielen Knoten liegt jeder Schlüssel.", kind: Fk::Int },
            Field { key: "consistency", label: "Konsistenz", unit: "", help: "Stark wartet auf eine Mehrheit der Kopien, schwach nur auf eine.", kind: Fk::Choice(CH_CONSISTENCY) },
            Field { key: "w", label: "Schreibquorum", unit: "", help: "So viele Kopien müssen ein Schreiben bestätigen; 0 heißt die Mehrheit.", kind: Fk::Int },
            Field { key: "r", label: "Lesequorum", unit: "", help: "So viele Kopien müssen beim Lesen antworten; 0 heißt die Mehrheit.", kind: Fk::Int },
            Field { key: "port", label: "Port", unit: "", help: "Hier nehmen die Knoten Anfragen an.", kind: Fk::Int },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "read_repair", label: "Beim Lesen reparieren", unit: "", help: "Veraltete Kopien bekommen nach einem Lesen den neuesten Wert.", kind: Fk::Flag },
            Field { key: "vnodes", label: "Punkte je Knoten", unit: "", help: "So oft steht jeder Knoten auf dem Ring; mehr verteilt gleichmäßiger.", kind: Fk::Int },
            Field { key: "timeout_ms", label: "Zeitgrenze", unit: "ms", help: "So lange wartet ein Knoten auf das Quorum, dann antwortet er 503.", kind: Fk::Dec },
            Field { key: "heartbeat_ms", label: "Lebenszeichen alle", unit: "ms", help: "So oft melden sich die Knoten beieinander.", kind: Fk::Dec },
            Field { key: "down_ms", label: "Ausgefallen nach", unit: "ms", help: "So lange ohne Lebenszeichen gilt ein Knoten als weg.", kind: Fk::Dec },
            Field { key: "hb_port", label: "Port der Lebenszeichen", unit: "", help: "Über diesen UDP-Port sprechen die Knoten.", kind: Fk::Int },
            Field { key: "cpu_us", label: "CPU je Anfrage", unit: "µs", help: "Rechenzeit einer Anfrage; sie zählt nur mit simulierter Hardware.", kind: Fk::Dec },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
        ],
    },
];

// i18n: title label unit help
static DB: [Group; 3] = [
    Group {
        title: "Datenbank",
        rare: false,
        fields: &[
            Field { key: "tables", label: "Tabellen", unit: "", help: "Name, Zeilen, Bytes je Zeile, Schlüssel, Verteilung und Indizes. Spalten stehen im JSON. Nur geänderte Zeilen sind wirklich da, alle anderen folgen aus ihrem Schlüssel.", kind: Fk::List },
            Field { key: "pool_mb", label: "Puffer im RAM", unit: "MiB", help: "So viel hält die Datenbank von ihren Seiten im RAM. Zu groß für das Gerät: erst Swap, dann beendet sie der Speichermangel.", kind: Fk::Dec },
            Field { key: "max_conns", label: "Verbindungen", unit: "", help: "Darüber bekommt eine Verbindung 503 und wird geschlossen.", kind: Fk::Int },
            Field { key: "durability", label: "Beim Commit", unit: "", help: "Voll wartet auf die Platte. Locker antwortet sofort und ist schneller, ein Absturz verliert dann die letzten Commits und zählt sie.", kind: Fk::Choice(CH_DURABILITY) },
            Field { key: "isolation", label: "Isolation", unit: "", help: "Was eine Transaktion sieht, wenn andere gleichzeitig ändern.", kind: Fk::Choice(CH_ISOLATION) },
            Field { key: "port", label: "Port", unit: "", help: "Hier nimmt sie Anweisungen an: POST /query.", kind: Fk::Int },
        ],
    },
    Group {
        title: "Kosten (Annahmen dieses Labors)",
        rare: true,
        fields: &[
            Field { key: "stmt_us", label: "CPU je Anweisung", unit: "µs", help: "Annahme: Lesen und Planen einer Anweisung auf einem Referenzkern. Zählt nur mit simulierter Hardware.", kind: Fk::Dec },
            Field { key: "page_us", label: "CPU je Seite", unit: "µs", help: "Annahme: jede Seite, die eine Anweisung anfasst, ob im Puffer oder nicht.", kind: Fk::Dec },
            Field { key: "row_us", label: "CPU je Zeile", unit: "µs", help: "Annahme: jede Zeile, die eine Anweisung ansieht.", kind: Fk::Dec },
            Field { key: "write_us", label: "CPU je Änderung", unit: "µs", help: "Annahme: jede Zeile, die eine Anweisung schreibt.", kind: Fk::Dec },
            Field { key: "page_kb", label: "Seitengröße", unit: "KiB", help: "Annahme: 1, 2, 4, 8, 16, 32 oder 64. Daraus folgen die Zeilen je Seite und die Tiefe der Bäume.", kind: Fk::Int },
            Field { key: "fanout", label: "Äste je Baumseite", unit: "", help: "Annahme: 0 rechnet aus der Seitengröße mit 16 Bytes je Eintrag.", kind: Fk::Int },
            Field { key: "redo_mbps", label: "Log wieder anwenden", unit: "MB/s", help: "Annahme: So schnell läuft die Wiederherstellung nach einem Absturz.", kind: Fk::Dec },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "lock_timeout_ms", label: "Warten auf eine Sperre", unit: "ms", help: "Danach bricht die Transaktion ab und darf es noch einmal versuchen.", kind: Fk::Dec },
            Field { key: "txn_idle_ms", label: "Stille Transaktion", unit: "ms", help: "So lange darf eine Transaktion ohne Anweisung offen bleiben, dann gehen ihre Sperren.", kind: Fk::Dec },
            Field { key: "flush_ms", label: "Log schreiben alle", unit: "ms", help: "Nur bei lockerem Commit: das ist die Zeit, die ein Absturz höchstens verliert.", kind: Fk::Dec },
            Field { key: "checkpoint_s", label: "Prüfpunkt alle", unit: "s", help: "Ein Prüfpunkt schreibt geänderte Seiten; das Log davor braucht die Wiederherstellung nicht mehr.", kind: Fk::Int },
            Field { key: "max_rows", label: "Zeilen je Antwort", unit: "", help: "Mehr schickt sie nicht; die Antwort sagt dann truncated.", kind: Fk::Int },
            Field { key: "write_rows_max", label: "Zeilen je Änderung", unit: "", help: "Eine Anweisung, die mehr ändern würde, wird abgewiesen.", kind: Fk::Int },
            Field { key: "touched_max", label: "Geänderte Zeilen", unit: "", help: "Grenze des Simulators: darüber vergisst er die ältesten Werte und zählt das. Die Zeiten stimmen weiter, diese Werte nicht.", kind: Fk::Int },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
        ],
    },
];

// i18n: title label unit help
static CACHE: [Group; 2] = [
    Group {
        title: "Cache",
        rare: false,
        fields: &[
            Field { key: "backend", label: "Backend", unit: "", help: "Wer die Antworten macht: ein Gerät wählen oder eine Adresse eintippen.", kind: Fk::Url(HTTP_TARGET) },
            Field { key: "port", label: "Port", unit: "", help: "Hier nimmt der Cache Anfragen an.", kind: Fk::Int },
            Field { key: "entries", label: "Einträge", unit: "", help: "So viele Antworten hält er, die am längsten unbenutzten gehen zuerst.", kind: Fk::Int },
            Field { key: "ttl_ms", label: "Gültig für", unit: "ms", help: "So lange gilt eine Antwort als frisch.", kind: Fk::Dec },
            Field { key: "hit_us", label: "Zeit für einen Treffer", unit: "µs", help: "So lange braucht eine Antwort aus dem Cache.", kind: Fk::Dec },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "max_kb", label: "Größe", unit: "KiB", help: "So viel hält er höchstens.", kind: Fk::Int },
            Field { key: "cache_control", label: "Cache-Control beachten", unit: "", help: "Das Backend darf sagen, was wie lange gilt.", kind: Fk::Flag },
            Field { key: "collapse", label: "Gleiche Anfragen bündeln", unit: "", help: "Während ein Eintrag geladen wird, warten gleiche Anfragen auf ihn.", kind: Fk::Flag },
            Field { key: "timeout_ms", label: "Zeitgrenze", unit: "ms", help: "So lange wartet er auf das Backend.", kind: Fk::Dec },
            Field { key: "connect_timeout_ms", label: "Zeit für den Aufbau", unit: "ms", help: "So lange darf eine Verbindung zum Backend dauern.", kind: Fk::Dec },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
        ],
    },
];

// i18n: title label unit help
static MQ: [Group; 1] = [Group {
    title: "Warteschlange",
    rare: false,
    fields: &[
        Field { key: "port", label: "Port", unit: "", help: "Hier nimmt die Queue Nachrichten an.", kind: Fk::Int },
        Field { key: "topics", label: "Themen", unit: "", help: "Durch Kommas getrennt; ein Thema entsteht auch mit seiner ersten Nachricht.", kind: Fk::Words },
        Field { key: "visibility_ms", label: "Unsichtbar für", unit: "ms", help: "Ohne Bestätigung bis dahin kommt eine Nachricht noch einmal.", kind: Fk::Dec },
        Field { key: "max_deliveries", label: "Zustellungen", unit: "", help: "Danach legt die Queue eine Nachricht beiseite.", kind: Fk::Int },
        Field { key: "max_depth", label: "Wartende Nachrichten", unit: "", help: "Darüber antwortet sie 429; 0 heißt ohne Grenze.", kind: Fk::Int },
        Field { key: "durable", label: "Auf die Disk schreiben", unit: "", help: "Nachrichten überstehen dann einen Neustart, kosten aber Zeit auf der Disk.", kind: Fk::Flag },
    ],
}];

// i18n: title label unit help
static WORKER: [Group; 2] = [
    Group {
        title: "Arbeit",
        rare: false,
        fields: &[
            Field { key: "queue", label: "Queue", unit: "", help: "Woher die Aufträge kommen: ein Gerät wählen oder eine Adresse eintippen.", kind: Fk::Url(QUEUE_TARGET) },
            Field { key: "topic", label: "Thema", unit: "", help: "Aus diesem Thema nimmt er Nachrichten.", kind: Fk::Text },
            Field { key: "concurrency", label: "Gleichzeitig", unit: "", help: "An so vielen Nachrichten arbeitet er zugleich.", kind: Fk::Int },
            Field { key: "time_ms", label: "Wartezeit je Nachricht", unit: "ms", help: "So lange braucht eine Nachricht, ohne CPU zu belegen.", kind: Fk::Dec },
            Field { key: "cpu_ms", label: "CPU je Nachricht", unit: "ms", help: "Rechenzeit einer Nachricht; sie zählt nur mit simulierter Hardware.", kind: Fk::Dec },
        ],
    },
    Group {
        title: "Seltener gebraucht",
        rare: true,
        fields: &[
            Field { key: "jitter_ms", label: "Streuung der Wartezeit", unit: "ms", help: "Bis zu so viel kommt zufällig dazu.", kind: Fk::Dec },
            Field { key: "error_rate", label: "Fehlerrate", unit: "%", help: "Dieser Anteil geht unerledigt zurück in die Queue.", kind: Fk::Pct },
            Field { key: "wait_ms", label: "Warten auf Arbeit", unit: "ms", help: "So lange hält eine Abfrage die Verbindung, wenn nichts ansteht.", kind: Fk::Dec },
            Field { key: "idle_ms", label: "Pause nach leerer Abfrage", unit: "ms", help: "So lange wartet er, bevor er neu fragt.", kind: Fk::Dec },
            Field { key: "timeout_ms", label: "Zeitgrenze", unit: "ms", help: "So lange wartet er auf die Queue.", kind: Fk::Dec },
            MEM_FIELDS[0],
            MEM_FIELDS[1],
        ],
    },
];

/// the form of a kind of service
pub(crate) fn form_of(kind: &str) -> Option<&'static [Group]> {
    Some(match kind {
        "http-server" => &HTTP_SERVER,
        "http-load" => &HTTP_LOAD,
        "lb" => &LB,
        "kv" => &KV,
        "cache" => &CACHE,
        "mq" => &MQ,
        "worker" => &WORKER,
        "db" => &DB,
        _ => return None,
    })
}

/// the fields of a form in the order of the widgets
pub(crate) fn fields(kind: &str) -> impl Iterator<Item = &'static Field> {
    form_of(kind).unwrap_or(&[]).iter().flat_map(|g| g.fields.iter())
}

/// a row of the form's list: a backend (all four), a target or a node (`addr` only)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Row {
    pub name: String,
    pub addr: String,
    pub port: String,
    pub weight: String,
    /// a table's: its distribution and indexes
    pub dist: String,
    pub indexes: String,
    /// its place in the config it came from: what else it has stays
    pub src: Option<usize>,
}

/// what a form shows: a text per field, the rows of its list
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Vals {
    pub texts: Vec<String>,
    pub rows: Vec<Row>,
}

impl Vals {
    /// a field by its member (tests, the dialog)
    pub(crate) fn set(&mut self, kind: &str, key: &str, text: &str) -> bool {
        match fields(kind).position(|f| f.key == key) {
            Some(i) => {
                self.texts[i] = String::from(text);
                true
            }
            None => false,
        }
    }
}

/// a member by its path "health.path"
fn get_path<'a>(c: &'a Value, path: &str) -> Option<&'a Value> {
    match path.split_once('.') {
        Some((a, b)) => get_path(c.get(a)?, b),
        None => c.get(path),
    }
}

/// a member out by its path; an object left empty goes with it
fn remove_path(c: &mut Value, path: &str) {
    let Value::Object(o) = c else { return };
    match path.split_once('.') {
        Some((a, b)) => {
            let Some(i) = o.iter().position(|(k, _)| k == a) else { return };
            remove_path(&mut o[i].1, b);
            if o[i].1.as_object().is_some_and(|x| x.is_empty()) {
                o.remove(i);
            }
        }
        None => o.retain(|(k, _)| k != path),
    }
}

/// a number as a field shows it (the language's decimal separator)
fn shown_number(s: &str) -> String {
    if tr::en() { String::from(s) } else { s.replace('.', ",") }
}

/// a member as a field shows it ("": not there)
fn text_of(f: &Field, v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    match f.kind {
        Fk::Pct => rate_ppm(v).map_or(String::new(), |ppm| shown_number(&fixed(ppm as u64, 10_000))),
        Fk::Flag => v.as_bool().map_or(String::new(), |b| String::from(if b { "true" } else { "false" })),
        Fk::Words => v.as_array().map_or(String::new(), |a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")),
        Fk::Int | Fk::Dec => match v {
            Value::Number(n) => shown_number(n.as_str()),
            Value::String(s) => shown_number(s),
            _ => String::new(),
        },
        _ => v.as_str().map_or(String::new(), String::from),
    }
}

/// the rows of the form's list in a config
fn rows_of(kind: &str, c: &Value) -> Vec<Row> {
    let text = |v: &Value| Row { addr: String::from(v.as_str().unwrap_or("")), ..Row::default() };
    let num = |v: Option<&Value>| v.and_then(|x| x.as_u64()).map_or(String::new(), |x| x.to_string());
    let list = |k: &str| c.get(k).and_then(|x| x.as_array()).unwrap_or(&[]);
    match kind {
        "http-load" => c.get("target").into_iter().map(text).chain(list("targets").iter().map(text)).collect(),
        "kv" => list("nodes").iter().map(text).collect(),
        "lb" => list("backends")
            .iter()
            .enumerate()
            .map(|(i, b)| Row {
                name: String::from(b.get("name").and_then(|x| x.as_str()).unwrap_or("")),
                addr: String::from(b.get("addr").and_then(|x| x.as_str()).unwrap_or("")),
                port: num(b.get("port")),
                weight: num(b.get("weight")),
                src: Some(i),
                ..Row::default()
            })
            .collect(),
        // a table: name, rows, bytes per row, key, distribution, indexes
        "db" => list("tables")
            .iter()
            .enumerate()
            .map(|(i, b)| Row {
                name: String::from(b.get("name").and_then(|x| x.as_str()).unwrap_or("")),
                addr: num(b.get("rows")),
                port: num(b.get("row_bytes")),
                weight: String::from(b.get("key").and_then(|x| x.as_str()).unwrap_or("")),
                dist: String::from(b.get("dist").and_then(|x| x.as_str()).unwrap_or("")),
                indexes: b.get("indexes").and_then(|x| x.as_array()).map_or(String::new(), |a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")),
                src: Some(i),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// what a form shows for a config
pub(crate) fn fill(kind: &str, c: &Value) -> Vals {
    Vals { texts: fields(kind).map(|f| if f.kind == Fk::List { String::new() } else { text_of(f, get_path(c, f.key)) }).collect(), rows: rows_of(kind, c) }
}

/// the value of a field's text, or what is wrong with it
fn parsed(f: &Field, text: &str) -> Result<Value, String> {
    let number = |s: &str| -> Option<Number> {
        let s = s.replace(',', ".");
        if s.starts_with('-') { None } else { Number::parse(&s) }
    };
    match f.kind {
        Fk::Int => text.parse::<u64>().map(Value::u64).map_err(|_| tf("{}: das ist keine ganze Zahl.", &[t(f.label)])),
        Fk::Dec => number(text).map(Value::Number).ok_or_else(|| tf("{}: das ist keine Zahl.", &[t(f.label)])),
        Fk::Pct => number(text).map(|n| Value::string(format!("{}%", n.as_str()))).ok_or_else(|| tf("{}: das ist keine Zahl.", &[t(f.label)])),
        Fk::Flag => Ok(Value::Bool(text == "true")),
        Fk::Words => Ok(Value::Array(text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(Value::string).collect())),
        _ => Ok(Value::string(text)),
    }
}

/// the form's list into a config: what stands there beside the fields of a row stays
fn write_rows(kind: &str, c: &mut Value, base: &Value, rows: &[Row]) -> Result<(), String> {
    if kind == "db" {
        return write_tables(c, base, rows);
    }
    let used: Vec<&Row> = rows.iter().filter(|r| !r.addr.trim().is_empty()).collect();
    let key = match kind {
        "http-load" => "targets",
        "kv" => "nodes",
        _ => "backends",
    };
    if kind == "http-load" {
        remove_path(c, "target");
    }
    if used.is_empty() {
        remove_path(c, key);
        return Ok(());
    }
    if kind != "lb" {
        c.insert(key, Value::Array(used.iter().map(|r| Value::string(r.addr.trim())).collect()));
        return Ok(());
    }
    let olds = base.get("backends").and_then(|b| b.as_array()).unwrap_or(&[]);
    let mut out = Vec::new();
    for r in used {
        let mut b = r.src.and_then(|i| olds.get(i)).cloned().unwrap_or_else(empty_object);
        let mut put = |k: &str, text: &str, num: bool| -> Result<(), String> {
            let text = text.trim();
            if text.is_empty() {
                remove_path(&mut b, k);
            } else if num {
                let v = text.parse::<u64>().map_err(|_| tf("{}: das ist keine ganze Zahl.", &[text]))?;
                b.insert(k, Value::u64(v));
            } else {
                b.insert(k, Value::string(text));
            }
            Ok(())
        };
        put("name", &r.name, false)?;
        put("addr", &r.addr, false)?;
        put("port", &r.port, true)?;
        put("weight", &r.weight, true)?;
        out.push(b);
    }
    c.insert("backends", Value::Array(out));
    Ok(())
}

/// a database's tables into a config: what a table has beside its row's fields (its columns) stays
fn write_tables(c: &mut Value, base: &Value, rows: &[Row]) -> Result<(), String> {
    let used: Vec<&Row> = rows.iter().filter(|r| !r.name.trim().is_empty()).collect();
    if used.is_empty() {
        remove_path(c, "tables");
        return Ok(());
    }
    let olds = base.get("tables").and_then(|b| b.as_array()).unwrap_or(&[]);
    let mut out = Vec::new();
    for r in used {
        let mut b = r.src.and_then(|i| olds.get(i)).cloned().unwrap_or_else(empty_object);
        let mut put = |k: &str, text: &str, num: bool| -> Result<(), String> {
            let text = text.trim();
            if text.is_empty() {
                remove_path(&mut b, k);
            } else if num {
                let v = text.parse::<u64>().map_err(|_| tf("{}: das ist keine ganze Zahl.", &[text]))?;
                b.insert(k, Value::u64(v));
            } else {
                b.insert(k, Value::string(text));
            }
            Ok(())
        };
        put("name", &r.name, false)?;
        put("rows", &r.addr, true)?;
        put("row_bytes", &r.port, true)?;
        put("key", &r.weight, false)?;
        put("dist", &r.dist, false)?;
        let words: Vec<Value> = r.indexes.split(',').map(str::trim).filter(|s| !s.is_empty()).map(Value::string).collect();
        if words.is_empty() {
            remove_path(&mut b, "indexes");
        } else {
            b.insert("indexes", Value::Array(words));
        }
        out.push(b);
    }
    c.insert("tables", Value::Array(out));
    Ok(())
}

/// the config with what changed in the form: every other member stays as and where it is
pub(crate) fn merged(kind: &str, base: &Value, v: &Vals) -> Result<Value, String> {
    let was = fill(kind, base);
    let mut c = if base.as_object().is_some() { base.clone() } else { empty_object() };
    for (i, f) in fields(kind).enumerate() {
        let now = v.texts.get(i).map_or("", |s| s.trim());
        if f.kind == Fk::List || now == was.texts[i] {
            continue;
        }
        if now.is_empty() {
            remove_path(&mut c, f.key);
        } else {
            set_path(&mut c, f.key, parsed(f, now)?).map_err(|m| tr::msg(&m))?;
        }
    }
    if v.rows != was.rows {
        write_rows(kind, &mut c, base, &v.rows)?;
    }
    Ok(c)
}

/// the default of field i as words, asked of the service ("": it has none to tell)
pub(crate) fn default_text(kind: &str, i: usize, defs: &Value) -> String {
    let Some(f) = fields(kind).nth(i) else { return String::new() };
    let Some(v) = get_path(defs, f.key) else { return String::new() };
    match f.kind {
        Fk::Flag => String::from(t(if v.as_bool() == Some(true) { "an" } else { "aus" })),
        Fk::Choice(ch) => {
            let w = v.as_str().unwrap_or("");
            ch.iter().find(|c| c.word == w).map_or(String::from(w), |c| String::from(t(c.label)))
        }
        // (a share of the config's fraction shows as percent)
        Fk::Pct => rate_ppm(v).map_or(String::new(), |ppm| shown_number(&fixed(ppm as u64, 10_000))),
        _ => text_of(f, Some(v)),
    }
}

/// what applying a config would do
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Takes {
    /// it is what the service has
    Same,
    /// the running service takes it (Service::reconfigure)
    Live,
    /// the device starts again with it
    Restart,
}

/// what applying this config to the project's service j would do: the question doc/sim.rs asks
pub(crate) fn takes(doc: &Doc, dev: u32, j: usize, config: &Value) -> Takes {
    let Some(d) = doc.proj.device(dev) else { return Takes::Same };
    let Some(old) = d.services.get(j) else { return Takes::Same };
    if old.config == *config {
        return Takes::Same;
    }
    let mut new = d.services.clone();
    new[j].config = config.clone();
    let eng = doc.sim.engine();
    match live_steps(dev, &d.services, &new, &|b: &ServiceDef| eng.can_reconfigure(dev, &b.name, &b.config)) {
        Some(_) => Takes::Live,
        None => Takes::Restart,
    }
}

/// what the form says about the values in it
#[derive(Clone, Debug, PartialEq, Eq)]
enum Say {
    Same,
    Live,
    Restart,
    /// it lacks something: it is taken, and it does not start
    Lacks(String),
    /// it is refused, with the reason
    Bad(String),
}

/// a device of the project in a list of choices
struct Pick {
    id: u32,
    name: String,
    addr: Option<String>,
}

/// the open form
pub(crate) struct Form {
    pub vals: Vals,
    /// the config it was filled from: when the project's changes (undo, the JSON), it fills anew
    base: Value,
    /// the defaults of its kind (jfn_sim::svcform::defaults)
    defs: Value,
    /// the rarer settings show
    rare: i32,
    say: Say,
    say_for: Option<(Vals, u32)>,
    picks: Vec<Pick>,
    picks_for: (u32, u64),
    /// a device chosen that has no address yet
    note: String,
}

impl Form {
    pub(crate) fn of(def: &ServiceDef) -> Form {
        Form { vals: fill(&def.kind, &def.config), base: def.config.clone(), defs: defaults(&def.kind).unwrap_or_else(empty_object), rare: 0, say: Say::Same, say_for: None, picks: Vec::new(), picks_for: (0, u64::MAX), note: String::new() }
    }
}

/// the verdict of the form on these values
fn say_of(n: &Net, d: &Device, j: usize, vals: &Vals) -> Say {
    let Some(old) = d.services.get(j) else { return Say::Same };
    let c = match merged(&old.kind, &old.config, vals) {
        Ok(c) => c,
        Err(e) => return Say::Bad(e),
    };
    if c == old.config {
        return Say::Same;
    }
    if let Some(m) = missing(&old.kind, &c) {
        return Say::Lacks(tr::msg(&m));
    }
    let def = ServiceDef { config: c.clone(), ..old.clone() };
    if let Err(m) = svcedit::trial(d, &def) {
        return Say::Bad(tr::msg(&m));
    }
    match takes(&n.doc, d.id, j, &c) {
        Takes::Same => Say::Same,
        Takes::Live => Say::Live,
        Takes::Restart => Say::Restart,
    }
}

/// an http address on this device, with the port its service answers on
fn url_of(d: &Device, addr: &str, kinds: &[&str]) -> String {
    let port = kinds.iter().find_map(|k| d.services.iter().find(|s| s.kind == *k)).and_then(port_of).or_else(|| kinds.first().and_then(|k| defaults(k)?.get("port")?.as_u64()));
    match port {
        Some(p) if p != 80 => format!("http://{}:{}/", addr, p),
        _ => format!("http://{}/", addr),
    }
}

/// the choice in a list of devices: what the field's text becomes, a row's name and port
fn chosen(n: &Net, kind: &str, f: &Field, pick: &Pick, addr: &str) -> Row {
    let d = n.doc.proj.device(pick.id);
    match (kind, f.kind, d) {
        ("lb", _, Some(d)) => {
            let port = d.services.iter().find(|s| s.kind == "http-server").and_then(port_of).filter(|&p| p != 80).map_or(String::new(), |p| p.to_string());
            Row { name: pick.name.clone(), addr: String::from(addr), port, ..Row::default() }
        }
        ("kv", _, _) => Row { addr: String::from(addr), ..Row::default() },
        (_, Fk::Url(kinds), Some(d)) => Row { addr: url_of(d, addr, kinds), ..Row::default() },
        (_, _, Some(d)) => Row { addr: url_of(d, addr, HTTP_TARGET), ..Row::default() },
        _ => Row { addr: String::from(addr), ..Row::default() },
    }
}

// the devices to choose from, read again when the project changed and every second
fn refresh_picks(n: &mut Net, dev: u32) {
    let key = (n.doc.rev, n.doc.sim.engine().now() / jfn_sim::SEC);
    let eng = n.doc.sim.state();
    let Some(f) = n.devwin.svc.form.as_mut() else { return };
    if f.picks_for == key {
        return;
    }
    f.picks_for = key;
    f.picks = n.doc.proj.devices.iter().filter(|d| catalog_host(&d.model)).take(200).map(|d| Pick { id: d.id, name: d.name.clone(), addr: jfn_sim::experiment::addr_of(eng, d.id, Some(dev)) }).collect();
}

// a device that can answer: it has an operating system of its own
fn catalog_host(model: &str) -> bool {
    jfn_model::catalog::model(model).is_some_and(|m| m.has(jfn_model::catalog::F_HOST))
}

// a device chosen from the list: None when it has no address yet (the form says so)
fn pick_row(n: &mut Net, kind: &str, f: &Field, sel: i32) -> Option<Row> {
    let form = n.devwin.svc.form.as_ref()?;
    let p = form.picks.get(sel.checked_sub(1)? as usize)?;
    let Some(addr) = p.addr.clone() else {
        let note = tf("{} hat noch keine IPv4-Adresse: erst im Reiter Konfiguration eine vergeben.", &[&p.name]);
        if let Some(form) = n.devwin.svc.form.as_mut() {
            form.note = note;
        }
        return None;
    };
    let row = chosen(n, kind, f, p, &addr);
    if let Some(form) = n.devwin.svc.form.as_mut() {
        form.note.clear();
    }
    Some(row)
}

// the choice of a device: "Gerät wählen …", then the devices with their addresses
fn pick_box(n: &mut Net, ui: &mut Ui, id: u32, disabled: bool) -> i32 {
    let Some(form) = n.devwin.svc.form.as_ref() else { return 0 };
    let labels: Vec<String> = core::iter::once(String::from(t("Gerät wählen …"))).chain(form.picks.iter().map(|p| match &p.addr {
        Some(a) => format!("{} ({})", p.name, a),
        None => tf("{} (noch keine Adresse)", &[&p.name]),
    })).collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let mut sel = 0;
    if disabled {
        ui.next_flags(ST_DISABLED);
    }
    if ui.combobox(id, &mut sel, &refs) & EV_ACT != 0 { sel } else { 0 }
}

// the form's list of a database: a table per row, what it says of itself in two lines
fn table_rows(n: &mut Net, ui: &mut Ui, disabled: bool) {
    let rows = n.devwin.svc.form.as_ref().map_or(0, |x| x.vals.rows.len());
    let flags = if disabled { ST_DISABLED } else { 0 };
    let mut gone = None;
    for r in 0..rows.min(ROWS_MAX) {
        let id = ID_SF_ROW + r as u32 * 8;
        let Some(x) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.rows.get_mut(r)) else { continue };
        ui.box_begin(0, BOX_V, 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(-1, 0);
        ui.next_flags(flags);
        ui.textbox(id, Some(t("Name der Tabelle")), &mut x.name, 64);
        ui.next_size(110, 0);
        ui.next_flags(flags);
        ui.textbox(id + 1, Some(t("Zeilen")), &mut x.addr, 16);
        ui.next_size(90, 0);
        ui.next_flags(flags);
        ui.textbox(id + 3, Some(t("Bytes je Zeile")), &mut x.port, 8);
        ui.next_flags(flags | ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
        ui.next_icon(IC_DELETE);
        if ui.button(id + 2, t("Entfernen")) & EV_ACT != 0 {
            gone = Some(r);
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(90, 0);
        ui.next_flags(flags);
        ui.textbox(id + 4, Some(t("Schlüssel")), &mut x.weight, 32);
        let mut sel = CH_DB_DIST.iter().position(|c| c.word == x.dist).map_or(0, |k| k as i32 + 1);
        let labels: Vec<&str> = core::iter::once(t("Verteilung: gleichmäßig")).chain(CH_DB_DIST.iter().map(|c| t(c.label))).collect();
        ui.next_size(190, 0);
        ui.next_flags(flags);
        if ui.combobox(id + 5, &mut sel, &labels) & EV_ACT != 0 {
            x.dist = sel.checked_sub(1).and_then(|k| CH_DB_DIST.get(k as usize)).map_or(String::new(), |c| String::from(c.word));
            ui.repass();
        }
        ui.next_size(-1, 0);
        ui.next_flags(flags);
        ui.textbox(id + 6, Some(t("Indizes, durch Kommas getrennt")), &mut x.indexes, 200);
        ui.box_end();
        ui.box_end();
    }
    if let Some(r) = gone {
        if let Some(x) = n.devwin.svc.form.as_mut() {
            x.vals.rows.remove(r);
        }
        ui.repass();
    }
    ui.next_flags(flags | ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_ADD);
    if rows < ROWS_MAX && ui.button(ID_SF_ROW_ADD, t("Tabelle hinzufügen")) & EV_ACT != 0 {
        if let Some(x) = n.devwin.svc.form.as_mut() {
            x.vals.rows.push(Row::default());
        }
        ui.repass();
    }
}

// the form's list: a row each, with the choice of a device; a backend has a name, a port and a weight
fn list_rows(n: &mut Net, ui: &mut Ui, kind: &str, f: &Field, disabled: bool) {
    if kind == "db" {
        table_rows(n, ui, disabled);
        return;
    }
    let rows = n.devwin.svc.form.as_ref().map_or(0, |x| x.vals.rows.len());
    let mut gone = None;
    let lb = kind == "lb";
    let flags = if disabled { ST_DISABLED } else { 0 };
    for r in 0..rows.min(ROWS_MAX) {
        let id = ID_SF_ROW + r as u32 * 8;
        ui.box_begin(0, BOX_V, 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(170, 0);
        let sel = pick_box(n, ui, id, disabled);
        if sel > 0 {
            if let Some(row) = pick_row(n, kind, f, sel) {
                if let Some(x) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.rows.get_mut(r)) {
                    x.addr = row.addr;
                    if lb {
                        x.name = row.name;
                        x.port = row.port;
                    }
                }
            }
            ui.repass();
        }
        let Some(x) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.rows.get_mut(r)) else {
            ui.box_end();
            ui.box_end();
            continue;
        };
        ui.next_size(-1, 0);
        ui.next_flags(flags);
        let hint = if lb { t("Adresse") } else if kind == "kv" { t("Adresse") } else { t("http://Adresse/") };
        ui.textbox(id + 1, Some(hint), &mut x.addr, 200);
        ui.next_flags(flags | ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
        ui.next_icon(IC_DELETE);
        if ui.button(id + 2, t("Entfernen")) & EV_ACT != 0 {
            gone = Some(r);
        }
        ui.box_end();
        if lb {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.next_size(-1, 0);
            ui.next_flags(flags);
            ui.textbox(id + 3, Some(t("Name")), &mut x.name, 64);
            ui.next_size(80, 0);
            ui.next_flags(flags);
            ui.textbox(id + 4, Some(t("Port")), &mut x.port, 8);
            ui.next_size(80, 0);
            ui.next_flags(flags);
            ui.textbox(id + 5, Some(t("Gewicht")), &mut x.weight, 8);
            ui.box_end();
        }
        ui.box_end();
    }
    if let Some(r) = gone {
        if let Some(x) = n.devwin.svc.form.as_mut() {
            x.vals.rows.remove(r);
        }
        ui.repass();
    }
    ui.next_flags(flags | ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_ADD);
    if rows < ROWS_MAX && ui.button(ID_SF_ROW_ADD, t("Hinzufügen")) & EV_ACT != 0 {
        if let Some(x) = n.devwin.svc.form.as_mut() {
            x.vals.rows.push(Row::default());
        }
        ui.repass();
    }
}

// one setting: its label, the control, its unit, the line of help
fn field(n: &mut Net, ui: &mut Ui, kind: &str, i: usize, f: &'static Field, disabled: bool) {
    let id = ID_SF_FIELD + i as u32 * 2;
    let flags = if disabled { ST_DISABLED } else { 0 };
    ui.box_begin(0, BOX_V, 2);
    if f.kind == Fk::List {
        ui.text(TS_BODY | TX_SECONDARY, t(f.label));
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t(f.help));
        list_rows(n, ui, kind, f, disabled);
        ui.box_end();
        return;
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_BODY | TX_SECONDARY, t(f.label));
    let defs = n.devwin.svc.form.as_ref().map_or_else(empty_object, |x| x.defs.clone());
    let std = default_text(kind, i, &defs);
    match f.kind {
        Fk::Flag | Fk::Choice(_) => {
            let head = if std.is_empty() { String::from(t("Standard")) } else { tf("Standard ({})", &[&std]) };
            let (words, labels): (Vec<&str>, Vec<&str>) = match f.kind {
                Fk::Choice(ch) => ch.iter().map(|c| (c.word, t(c.label))).unzip(),
                _ => (alloc::vec!["true", "false"], alloc::vec![t("an"), t("aus")]),
            };
            let items: Vec<&str> = core::iter::once(head.as_str()).chain(labels.iter().copied()).collect();
            let Some(x) = n.devwin.svc.form.as_mut() else { return };
            let now = x.vals.texts.get(i).cloned().unwrap_or_default();
            let mut sel = words.iter().position(|w| *w == now).map_or(0, |k| k as i32 + 1);
            ui.next_size(-1, 0);
            ui.next_flags(flags);
            if ui.combobox(id, &mut sel, &items) & EV_ACT != 0 {
                if let Some(t) = x.vals.texts.get_mut(i) {
                    *t = sel.checked_sub(1).and_then(|k| words.get(k as usize)).map_or(String::new(), |w| String::from(*w));
                }
                ui.repass();
            }
        }
        Fk::Url(_) => {
            ui.next_size(170, 0);
            let sel = pick_box(n, ui, id + 1, disabled);
            if sel > 0 {
                if let Some(row) = pick_row(n, kind, f, sel) {
                    if let Some(t) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.texts.get_mut(i)) {
                        *t = row.addr;
                    }
                }
                ui.repass();
            }
            let Some(x) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.texts.get_mut(i)) else { return };
            ui.next_size(-1, 0);
            ui.next_flags(flags);
            ui.textbox(id, Some(t("http://Adresse/")), x, 200);
        }
        _ => {
            let hint = if std.is_empty() { String::new() } else { tf("Standard: {}", &[&std]) };
            let Some(x) = n.devwin.svc.form.as_mut().and_then(|x| x.vals.texts.get_mut(i)) else { return };
            ui.next_size(-1, 0);
            ui.next_flags(flags);
            ui.textbox(id, if hint.is_empty() { None } else { Some(&hint) }, x, 200);
            if !f.unit.is_empty() {
                ui.next_size(72, 0);
                ui.text(TS_BODY | TX_SECONDARY, t(f.unit));
            }
        }
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t(f.help));
    ui.box_end();
}

/// the form of the project's service j of device `dev` (below its card while it is open)
pub(crate) fn view(n: &mut Net, ui: &mut Ui, dev: u32, j: usize) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let Some(def) = d.services.get(j).cloned() else { return };
    let Some(groups) = form_of(&def.kind) else { return };
    let Some(f) = n.devwin.svc.form.as_mut() else { return };
    if f.base != def.config {
        *f = Form::of(&def);
    }
    let past = n.doc.sim.past().is_some();
    refresh_picks(n, dev);
    let mut json = false;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    let note = if past { t("Das ist eine Ansicht der Vergangenheit: hier ändert sich nichts. „Zurück zu live“ macht die Einstellungen wieder änderbar.") } else { t("Ein leeres Feld nimmt die Vorgabe des Dienstes.") };
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, note);
    ui.next_flags(ST_COMPACT);
    if ui.button(ID_SF_JSON, t("Als JSON bearbeiten")) & EV_ACT != 0 {
        json = true;
    }
    ui.box_end();
    let mut i = 0;
    for (g, group) in groups.iter().enumerate() {
        if group.rare {
            let mut open = n.devwin.svc.form.as_ref().map_or(0, |x| x.rare);
            let shown = ui.expander_begin(ID_SF_RARE + g as u32, t(group.title), &mut open) != 0;
            if let Some(x) = n.devwin.svc.form.as_mut() {
                x.rare = open;
            }
            if shown {
                ui.box_begin(0, BOX_V, 6);
                for fd in group.fields {
                    field(n, ui, &def.kind, i, fd, past);
                    i += 1;
                }
                ui.box_end();
            }
            ui.expander_end();
            if !shown {
                i += group.fields.len();
            }
        } else {
            ui.text(TS_BODY_STRONG, t(group.title));
            for fd in group.fields {
                field(n, ui, &def.kind, i, fd, past);
                i += 1;
            }
        }
    }
    // what Übernehmen would do, asked again when the fields or the project changed
    let key = n.devwin.svc.form.as_ref().map(|x| (x.vals.clone(), n.doc.rev));
    if n.devwin.svc.form.as_ref().is_some_and(|x| x.say_for != key) {
        if let Some((vals, rev)) = key {
            let say = say_of(n, &d, j, &vals);
            if let Some(x) = n.devwin.svc.form.as_mut() {
                x.say = say;
                x.say_for = Some((vals, rev));
            }
        }
    }
    let Some(x) = n.devwin.svc.form.as_ref() else { return };
    let (say, typed) = (x.say.clone(), x.note.clone());
    if !typed.is_empty() {
        crate::workspace::error_text(ui, &typed, 0);
    }
    ui.next_size(-1, 0);
    match &say {
        Say::Same => ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Noch nichts geändert.")),
        Say::Live => ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Gilt sofort: {} läuft weiter, ohne Neustart.", &[&def.name])),
        Say::Restart => ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("{} startet dafür neu.", &[&d.name])),
        Say::Lacks(m) => ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &format!("{} {}", m, tf("{} startet dafür neu.", &[&d.name]))),
        Say::Bad(m) => crate::workspace::error_text(ui, m, 0),
    }
    let mut cancel = false;
    let mut apply = false;
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_SF_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
        cancel = true;
    }
    if past || matches!(say, Say::Same | Say::Bad(_)) {
        ui.next_flags(ST_DISABLED);
    }
    if ui.accent_button(ID_SF_APPLY, t("Übernehmen")) & EV_ACT != 0 {
        apply = true;
    }
    ui.box_end();
    if cancel {
        n.devwin.svc.close();
        ui.repass();
    }
    if json {
        svcedit::to_json(n, &def);
        ui.repass();
    }
    if apply {
        take(n, &d, j);
        ui.repass();
    }
}

// Übernehmen: one undo step, the simulator follows as the form said
fn take(n: &mut Net, d: &Device, j: usize) {
    let Some(old) = d.services.get(j) else { return };
    let Some(vals) = n.devwin.svc.form.as_ref().map(|x| x.vals.clone()) else { return };
    let Ok(c) = merged(&old.kind, &old.config, &vals) else { return };
    n.devwin.svc.close();
    apply_config(n, d.id, j, c);
}

/// a new config of the project's service j: one undo step (svcedit.rs and the form)
pub(crate) fn apply_config(n: &mut Net, dev: u32, j: usize, c: Value) {
    let Some(d) = n.doc.proj.device(dev) else { return };
    let Some(old) = d.services.get(j) else { return };
    if old.config == c {
        return;
    }
    let label = format!("Dienst {} auf {} einstellen", old.name, d.name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.services[j].config = c) {
        n.doc.apply(e);
    }
}

// ---------------------------------------------------------------- Dienst hinzufügen

/// a kind of the dialog and what it does
struct AddKind {
    kind: &'static str,
    help: &'static str,
}

// i18n: help
const KINDS: [AddKind; 8] = [
    AddKind { kind: "http-server", help: "Antwortet auf HTTP-Anfragen, mit Rechenzeit, Wartezeit und Fehlerrate." },
    AddKind { kind: "http-load", help: "Erzeugt Last: Nutzer, die warten, oder eine feste Rate." },
    AddKind { kind: "lb", help: "Verteilt Anfragen oder Verbindungen auf mehrere Backends." },
    AddKind { kind: "kv", help: "Ein Schlüssel-Wert-Speicher über mehrere Knoten, mit Kopien und Quoren." },
    AddKind { kind: "cache", help: "Hält Antworten eines Backends und antwortet schneller." },
    AddKind { kind: "mq", help: "Hält Nachrichten, bis ein Worker sie abgeholt und bestätigt hat." },
    AddKind { kind: "worker", help: "Holt Nachrichten aus einer Queue und arbeitet sie ab." },
    AddKind { kind: "db", help: "Eine Datenbank mit Tabellen, Puffer, Sperren, Versionen und Log; sie nimmt SQL-artige Anweisungen über HTTP an." },
];

/// the usual name of a service of a kind
fn usual_name(kind: &str) -> &str {
    match kind {
        "http-server" => "web",
        "http-load" => "load",
        k => k,
    }
}

/// a name no service of the device has: the usual one, else with a number
pub(crate) fn free_name(d: &Device, kind: &str) -> String {
    let base = usual_name(kind);
    let free = |n: &str| !d.services.iter().any(|s| s.name.eq_ignore_ascii_case(n));
    if free(base) {
        return String::from(base);
    }
    (2..).map(|k| format!("{}{}", base, k)).find(|n| free(n)).unwrap_or_default()
}

/// the dialog's state
pub(crate) struct AddDlg {
    pub open: Shared<i32>,
    pub dev: u32,
    kind: i32,
    name: String,
    /// the name was typed: another kind keeps it
    typed: bool,
    err: String,
}

impl AddDlg {
    pub(crate) fn new() -> AddDlg {
        AddDlg { open: Shared::new(0), dev: 0, kind: 0, name: String::new(), typed: false, err: String::new() }
    }
}

/// "Dienst hinzufügen …" below the cards of the Dienste tab of window k
pub(crate) fn add_button(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if n.doc.sim.past().is_some() {
        ui.next_flags(ST_DISABLED);
    }
    ui.next_icon(IC_ADD);
    if ui.button(ID_SVC_ADD0 + k as u32, t("Dienst hinzufügen …")) & EV_ACT != 0 {
        let a = &mut n.devwin.svc.add;
        a.dev = dev;
        a.kind = 0;
        a.typed = false;
        a.err.clear();
        a.name = free_name(&d, KINDS[0].kind);
        a.open.set(1);
        ui.repass();
    }
    ui.box_end();
}

/// the dialog of device `dev` (from its tab)
pub(crate) fn add_dialog(n: &mut Net, ui: &mut Ui, dev: u32) {
    let open = n.devwin.svc.add.open.clone();
    if n.devwin.svc.add.dev != dev || open.get() == 0 {
        return;
    }
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    if ui.dialog_begin(ID_SA_DLG, t("Dienst hinzufügen"), &open) {
        ui.next_size(440, 0);
        ui.box_begin(0, BOX_V, 8);
        let names: Vec<&str> = KINDS.iter().map(|k| super::services::kind_name(k.kind)).collect();
        let a = &mut n.devwin.svc.add;
        ui.text(TS_BODY | TX_SECONDARY, t("Art"));
        ui.next_size(-1, 0);
        let mut sel = a.kind;
        if ui.combobox(ID_SA_KIND, &mut sel, &names) & EV_ACT != 0 {
            a.kind = sel.clamp(0, KINDS.len() as i32 - 1);
            if !a.typed {
                a.name = free_name(&d, KINDS[a.kind as usize].kind);
            }
            ui.repass();
        }
        let kind = &KINDS[a.kind.clamp(0, KINDS.len() as i32 - 1) as usize];
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t(kind.help));
        ui.text(TS_BODY | TX_SECONDARY, t("Name"));
        ui.next_size(-1, 0);
        if ui.textbox(ID_SA_NAME, None, &mut a.name, 64) & EV_ACT != 0 {
            a.typed = true;
        }
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("{} startet dafür neu. Danach folgt das Formular.", &[&d.name]));
        if !a.err.is_empty() {
            let e = a.err.clone();
            crate::workspace::error_text(ui, &e, 0);
        }
        ui.box_end();
        let b = ui.dialog_buttons(Some(t("Hinzufügen")), None, Some(t("Abbrechen")));
        if b == DLG_PRIMARY {
            add(n, ui, &d, kind.kind);
        }
    }
    ui.dialog_end();
}

// Hinzufügen: one undo step; the card has its form open
fn add(n: &mut Net, ui: &mut Ui, d: &Device, kind: &str) {
    let a = &mut n.devwin.svc.add;
    let name = String::from(a.name.trim());
    if name.is_empty() {
        a.err = String::from(t("Der Dienst braucht einen Namen."));
        a.open.set(1);
        return;
    }
    if d.services.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
        a.err = tf("{} hat schon einen Dienst „{}“.", &[&d.name, &name]);
        a.open.set(1);
        return;
    }
    let def = ServiceDef { kind: String::from(kind), name: name.clone(), disabled: false, config: empty_object() };
    let label = format!("Dienst {} auf {} hinzufügen", name, d.name);
    let new = def.clone();
    if let Some(e) = edits::device_edit(&n.doc.proj, d.id, &label, move |x| x.services.push(new)) {
        n.doc.apply(e);
    }
    n.devwin.svc.open_form(d.id, &def);
    ui.repass();
}
