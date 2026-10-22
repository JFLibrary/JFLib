// i18n -- languages, translation catalogues, regional formats and messages
//
//   set_language(LANG_*) / language()   the UI language: the catalogue column (process wide;
//                                       default German, the source column)
//   set_language_code(code) -> index    the best column for a BCP 47 code ("fr-CA" -> "fr",
//                                       else English, else the source)
//   language_code() -> "de" / "en" / ...   the code of the current column
//   user_languages() / set_user_languages(list)   the languages the user wants, in order (ui
//                                       boots them from --lang or jfgui_LANG, the system in
//                                       window mode, "de-DE" in headless runs);
//                                       negotiate(available, wanted)
//   use_system()                        user languages and format from --lang, jfgui_LANG or
//                                       the system (programs without ui::main!)
//   tr(de) -> text                      a text of the library itself (file dialog, command
//                                       bar, tree, toasts, shortcuts, date picker) in the UI
//                                       language: German, English, French, Spanish, Italian,
//                                       Dutch, Polish, Arabic, Hebrew (library_languages), else
//                                       English; the German source text is the key
//                                       (library_keys)
//   Catalog::from_tsv(text)             an app's catalogue: a TSV file whose first line names
//                                       the languages ("de\ten\tfr"), then one row per text,
//                                       the source text first; catalog.tr(s) -> the text in
//                                       the current language (the source when missing)
//   month_name(m) / month_short(m) / weekday_short(d)   calendar names of the UI language
//                                       (m 1..12, d 0 = Mon)
//   set_catalog(catalog) / t(s)         the app's catalogue for the whole process: t(s) is the
//                                       text in the current language (German source when
//                                       missing).  Unless set_language came first, the column
//                                       is negotiated with the user's languages
//   fill(text, args)                    "{}" placeholders replaced in order
//   msg(text, args)                     named / numbered placeholders, plurals, select,
//                                       numbers and dates in the user's format (i18n_msg.rs)
//   locale(), Locale, fmt_*, plural     regional formats and plural rules (i18n_locale.rs)
//   collate(a, b), Collator, to_upper / to_lower / fold, eq_ignore_case   sorting by language
//                                       and case mapping (i18n_collate.rs)
//
// The language index is the catalogue column: LANG_DE = 0 (the source column), LANG_EN = 1.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[path = "i18n_locale.rs"]
mod regional;
pub use regional::*;
#[path = "i18n_msg.rs"]
mod message;
pub use message::{Arg, msg};
#[path = "i18n_collate.rs"]
mod collation;
pub use collation::*;

pub const LANG_DE: u32 = 0;
pub const LANG_EN: u32 = 1;

static LANG: AtomicU32 = AtomicU32::new(LANG_DE);
// the language of the library's texts: an index of LIB_LANGS (0 German, 1 English, ...)
static LIB: AtomicU32 = AtomicU32::new(0);
// the UI language is written right to left
static LIB_RTL: AtomicBool = AtomicBool::new(false);
const RTL_LANGS: [&str; 10] = ["ar", "he", "iw", "fa", "ur", "ps", "sd", "ug", "yi", "dv"];
// set_language / set_language_code was called: set_catalog keeps the column (or the code)
static EXPLICIT: AtomicBool = AtomicBool::new(false);
static EXPLICIT_CODE: rt::sync::SpinLock<Option<String>> = rt::sync::SpinLock::new(None);
// the user's languages (empty: "de-DE")
static USER: rt::sync::SpinLock<Vec<String>> = rt::sync::SpinLock::new(Vec::new());

/// set the UI language: the catalogue column (LANG_DE, LANG_EN, ...)
pub fn set_language(lang: u32) {
    EXPLICIT.store(true, Ordering::Relaxed);
    *EXPLICIT_CODE.lock() = None;
    select(lang);
}

fn select(lang: u32) {
    LANG.store(lang, Ordering::Relaxed);
    let code = language_code();
    let lib = if code == "iw" { "he" } else { code.as_str() };
    LIB.store(LIB_LANGS.iter().position(|l| *l == lib).unwrap_or(1) as u32, Ordering::Relaxed);
    LIB_RTL.store(RTL_LANGS.contains(&code.as_str()), Ordering::Relaxed);
}

// the language codes of the columns: the catalogue's, else the library's own
fn columns() -> Vec<String> {
    match *APP.lock() {
        Some(c) if !c.langs.is_empty() => c.langs.clone(),
        _ => alloc::vec![String::from("de"), String::from("en")],
    }
}

/// the code of the current UI language ("de", "en", "fr"; the catalogue's header, lower case)
pub fn language_code() -> String {
    let cols = columns();
    let code = cols.get(language() as usize).map_or("en", |c| c.as_str());
    String::from(code.split(['-', '_']).next().unwrap_or(code)).to_ascii_lowercase()
}

/// select the column that fits a BCP 47 code best (the language, else English, else the
/// source) -> its index; a later catalogue is matched with the code again
pub fn set_language_code(code: &str) -> u32 {
    EXPLICIT.store(true, Ordering::Relaxed);
    *EXPLICIT_CODE.lock() = Some(String::from(code));
    let k = pick(&[code]);
    select(k);
    k
}

// the best column for the wanted languages
fn pick<W: AsRef<str>>(wanted: &[W]) -> u32 {
    let cols = columns();
    let k = negotiate(&cols, wanted).or_else(|| negotiate(&cols, &["en"])).unwrap_or(0);
    k as u32
}

/// the first of `available` that fits the `wanted` languages (in their order): the same tag,
/// else the same language ("fr-CA" takes "fr" or "fr-FR")
pub fn negotiate<A: AsRef<str>, W: AsRef<str>>(available: &[A], wanted: &[W]) -> Option<usize> {
    fn primary(t: &str) -> &str {
        t.split(['-', '_']).next().unwrap_or(t)
    }
    for w in wanted {
        let w = w.as_ref();
        if let Some(k) = available.iter().position(|a| a.as_ref().replace('_', "-").eq_ignore_ascii_case(&w.replace('_', "-"))) {
            return Some(k);
        }
        if let Some(k) = available.iter().position(|a| primary(a.as_ref()).eq_ignore_ascii_case(primary(w))) {
            return Some(k);
        }
    }
    None
}

/// the languages the user wants, most wanted first (BCP 47)
pub fn user_languages() -> Vec<String> {
    let u = USER.lock().clone();
    if u.is_empty() { alloc::vec![String::from("de-DE")] } else { u }
}

/// set the user's languages; an installed catalogue follows unless set_language came first
pub fn set_user_languages<W: AsRef<str>>(list: &[W]) {
    *USER.lock() = list.iter().filter_map(|w| plat::locale_tag(w.as_ref())).collect();
    if APP.lock().is_some() && !EXPLICIT.load(Ordering::Relaxed) {
        select(pick(&user_languages()));
    }
}

/// the user's languages and format from jfgui_LANG ("fr-CA" or a ':' list), else the system
pub fn use_system() {
    if !use_env() {
        set_user_languages(&plat::system_languages());
        if let Some(t) = plat::system_locale() {
            set_locale(&t);
        }
    }
}

// --lang (test scripts: "#!args --lang ar"), else jfgui_LANG: languages and format -> whether
// one was set
fn use_env() -> bool {
    let Some(v) = plat::arg_value("--lang").or_else(|| plat::env("jfgui_LANG")).filter(|v| !v.trim().is_empty()) else { return false };
    let list: Vec<&str> = v.split([':', ',']).map(str::trim).filter(|t| !t.is_empty()).collect();
    set_user_languages(&list);
    if let Some(first) = list.first() {
        set_locale(first);
    }
    true
}

/// ui boot: jfgui_LANG, else the system's languages in window mode; headless runs keep German
/// so screenshots match on every machine
pub(crate) fn boot(headless: bool) {
    if headless {
        use_env();
    } else {
        use_system();
    }
}

/// whether the UI language is written right to left (Arabic, Hebrew, Persian, Urdu, ...):
/// ui mirrors its windows then (Ui::set_rtl)
pub fn rtl_language() -> bool {
    LIB_RTL.load(Ordering::Relaxed)
}

/// the current UI language (LANG_*)
pub fn language() -> u32 {
    LANG.load(Ordering::Relaxed)
}

// the languages of the library's own texts: the columns of LIB_TEXTS and the calendar names
const LIB_LANGS: [&str; 9] = ["de", "en", "fr", "es", "it", "nl", "pl", "ar", "he"];

// the library's own texts: the German source, then the other languages of LIB_LANGS (short,
// as the systems' dialogs say it; key names as the keyboards print them)
static LIB_TEXTS: &[[&str; 9]] = &[
    ["Öffnen", "Open", "Ouvrir", "Abrir", "Apri", "Openen", "Otwórz", "فتح", "פתיחה"],
    ["Speichern", "Save", "Enregistrer", "Guardar", "Salva", "Opslaan", "Zapisz", "حفظ", "שמירה"],
    ["Speichern unter", "Save as", "Enregistrer sous", "Guardar como", "Salva con nome", "Opslaan als", "Zapisz jako", "حفظ باسم", "שמירה בשם"],
    ["Abbrechen", "Cancel", "Annuler", "Cancelar", "Annulla", "Annuleren", "Anuluj", "إلغاء", "ביטול"],
    ["Schließen", "Close", "Fermer", "Cerrar", "Chiudi", "Sluiten", "Zamknij", "إغلاق", "סגירה"],
    ["Zurücksetzen", "Reset", "Réinitialiser", "Restablecer", "Reimposta", "Opnieuw instellen", "Resetuj", "إعادة تعيين", "איפוס"],
    ["Dateiname:", "File name:", "Nom du fichier\u{A0}:", "Nombre de archivo:", "Nome file:", "Bestandsnaam:", "Nazwa pliku:", "اسم الملف:", "שם קובץ:"],
    ["Dateityp:", "File type:", "Type de fichier\u{A0}:", "Tipo de archivo:", "Tipo file:", "Bestandstype:", "Typ pliku:", "نوع الملف:", "סוג קובץ:"],
    ["Ordner auswählen", "Select folder", "Sélectionner un dossier", "Seleccionar carpeta", "Seleziona cartella", "Map selecteren", "Wybierz folder", "تحديد مجلد", "בחירת תיקייה"],
    ["Neuer Ordner", "New folder", "Nouveau dossier", "Nueva carpeta", "Nuova cartella", "Nieuwe map", "Nowy folder", "مجلد جديد", "תיקייה חדשה"],
    ["Übergeordneter Ordner", "Parent folder", "Dossier parent", "Carpeta superior", "Cartella superiore", "Bovenliggende map", "Folder nadrzędny", "المجلد الأصل", "תיקיית אב"],
    ["Startordner", "Home folder", "Dossier personnel", "Carpeta personal", "Cartella personale", "Persoonlijke map", "Folder domowy", "المجلد الرئيسي", "תיקיית הבית"],
    [
        "Der Pfad ist zu lang.",
        "The path is too long.",
        "Le chemin d’accès est trop long.",
        "La ruta es demasiado larga.",
        "Il percorso è troppo lungo.",
        "Het pad is te lang.",
        "Ścieżka jest za długa.",
        "المسار طويل جدًا.",
        "הנתיב ארוך מדי.",
    ],
    [
        "Die Datei wurde nicht gefunden.",
        "The file was not found.",
        "Le fichier est introuvable.",
        "No se encontró el archivo.",
        "Impossibile trovare il file.",
        "Het bestand is niet gevonden.",
        "Nie znaleziono pliku.",
        "تعذر العثور على الملف.",
        "הקובץ לא נמצא.",
    ],
    ["Aufwärts", "Up", "Monter", "Subir", "Su", "Omhoog", "W górę", "لأعلى", "למעלה"],
    ["Zurück", "Back", "Précédent", "Atrás", "Indietro", "Vorige", "Wstecz", "السابق", "הקודם"],
    ["Weiter", "Next", "Suivant", "Siguiente", "Avanti", "Volgende", "Dalej", "التالي", "הבא"],
    ["Fertigstellen", "Finish", "Terminer", "Finalizar", "Fine", "Voltooien", "Zakończ", "إنهاء", "סיום"],
    ["Weitere Befehle", "More commands", "Plus de commandes", "Más comandos", "Altri comandi", "Meer opdrachten", "Więcej poleceń", "مزيد من الأوامر", "פקודות נוספות"],
    [
        "Vorherige Register",
        "Previous tabs",
        "Onglets précédents",
        "Pestañas anteriores",
        "Schede precedenti",
        "Vorige tabbladen",
        "Poprzednie karty",
        "علامات التبويب السابقة",
        "כרטיסיות קודמות",
    ],
    ["Weitere Register", "More tabs", "Plus d’onglets", "Más pestañas", "Altre schede", "Meer tabbladen", "Więcej kart", "مزيد من علامات التبويب", "כרטיסיות נוספות"],
    ["Aufklappen", "Expand", "Développer", "Expandir", "Espandi", "Uitvouwen", "Rozwiń", "توسيع", "הרחבה"],
    ["Zuklappen", "Collapse", "Réduire", "Contraer", "Comprimi", "Samenvouwen", "Zwiń", "طي", "כיווץ"],
    [
        "Navigation umschalten",
        "Toggle navigation",
        "Afficher/masquer la navigation",
        "Mostrar u ocultar navegación",
        "Mostra/nascondi navigazione",
        "Navigatie weergeven/verbergen",
        "Pokaż/ukryj nawigację",
        "إظهار التنقل أو إخفاؤه",
        "הצגה/הסתרה של הניווט",
    ],
    ["Heute", "Today", "Aujourd’hui", "Hoy", "Oggi", "Vandaag", "Dzisiaj", "اليوم", "היום"],
    ["Vorheriger Monat", "Previous month", "Mois précédent", "Mes anterior", "Mese precedente", "Vorige maand", "Poprzedni miesiąc", "الشهر السابق", "החודש הקודם"],
    ["Nächster Monat", "Next month", "Mois suivant", "Mes siguiente", "Mese successivo", "Volgende maand", "Następny miesiąc", "الشهر التالي", "החודש הבא"],
    // key names of the shortcuts (Arabic and Hebrew keyboards print the English ones)
    ["Strg", "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Ctrl", "Ctrl"],
    ["Umschalt", "Shift", "Maj", "Mayús", "Maiusc", "Shift", "Shift", "Shift", "Shift"],
    ["Alt", "Alt", "Alt", "Alt", "Alt", "Alt", "Alt", "Alt", "Alt"],
    ["Entf", "Del", "Suppr", "Supr", "Canc", "Del", "Del", "Del", "Del"],
    ["Einfg", "Ins", "Inser", "Insert", "Ins", "Ins", "Ins", "Ins", "Ins"],
    ["Pos1", "Home", "Origine", "Inicio", "Home", "Home", "Home", "Home", "Home"],
    ["Ende", "End", "Fin", "Fin", "Fine", "End", "End", "End", "End"],
    ["Bild auf", "PgUp", "Pg préc", "Re Pág", "PgSu", "PgUp", "PgUp", "PgUp", "PgUp"],
    ["Bild ab", "PgDn", "Pg suiv", "Av Pág", "PgGiù", "PgDn", "PgDn", "PgDn", "PgDn"],
    ["Leertaste", "Space", "Espace", "Espacio", "Spazio", "Spatie", "Spacja", "Space", "Space"],
    ["Eingabe", "Enter", "Entrée", "Entrar", "Invio", "Enter", "Enter", "Enter", "Enter"],
    ["Rücktaste", "Backspace", "Retour arrière", "Retroceso", "Backspace", "Backspace", "Backspace", "Backspace", "Backspace"],
    ["Esc", "Esc", "Échap", "Esc", "Esc", "Esc", "Esc", "Esc", "Esc"],
    ["Nach-links", "Left", "Gauche", "Izquierda", "Sinistra", "Links", "W lewo", "Left", "Left"],
    ["Nach-rechts", "Right", "Droite", "Derecha", "Destra", "Rechts", "W prawo", "Right", "Right"],
    ["Nach-oben", "Up", "Haut", "Arriba", "Su", "Omhoog", "W górę", "Up", "Up"],
    ["Nach-unten", "Down", "Bas", "Abajo", "Giù", "Omlaag", "W dół", "Down", "Down"],
    // the settings page (settings_page.rs)
    ["Allgemein", "General", "Général", "General", "Generale", "Algemeen", "Ogólne", "عام", "כללי"],
    [
        "Wird nach einem Neustart wirksam.",
        "Takes effect after a restart.",
        "Prend effet après un redémarrage.",
        "Surte efecto después de reiniciar.",
        "Ha effetto dopo il riavvio.",
        "Wordt van kracht na een herstart.",
        "Zacznie obowiązywać po ponownym uruchomieniu.",
        "يسري مفعوله بعد إعادة التشغيل.",
        "ייכנס לתוקף לאחר הפעלה מחדש.",
    ],
    // fonts (fontpick.rs)
    ["OK", "OK", "OK", "Aceptar", "OK", "OK", "OK", "موافق", "אישור"],
    ["Suchen", "Search", "Rechercher", "Buscar", "Cerca", "Zoeken", "Szukaj", "بحث", "חיפוש"],
    ["Schriftart", "Font", "Police", "Fuente", "Carattere", "Lettertype", "Czcionka", "الخط", "גופן"],
    [
        "Schriftschnitt",
        "Font style",
        "Style de police",
        "Estilo de fuente",
        "Stile carattere",
        "Tekenstijl",
        "Styl czcionki",
        "نمط الخط",
        "סגנון גופן",
    ],
    ["Größe", "Size", "Taille", "Tamaño", "Dimensione", "Grootte", "Rozmiar", "الحجم", "גודל"],
    ["Vorschau", "Preview", "Aperçu", "Vista previa", "Anteprima", "Voorbeeld", "Podgląd", "معاينة", "תצוגה מקדימה"],
    ["Standard", "Regular", "Normal", "Normal", "Normale", "Normaal", "Normalny", "عادي", "רגיל"],
    ["Kursiv", "Italic", "Italique", "Cursiva", "Corsivo", "Cursief", "Kursywa", "مائل", "נטוי"],
    ["Fett", "Bold", "Gras", "Negrita", "Grassetto", "Vet", "Pogrubienie", "غامق", "מודגש"],
    [
        "Fett Kursiv",
        "Bold Italic",
        "Gras italique",
        "Negrita cursiva",
        "Grassetto corsivo",
        "Vet cursief",
        "Pogrubiona kursywa",
        "غامق مائل",
        "מודגש נטוי",
    ],
    [
        "Keine passende Schrift installiert",
        "No matching font installed",
        "Aucune police correspondante installée",
        "No hay ninguna fuente adecuada instalada",
        "Nessun carattere adatto installato",
        "Geen passend lettertype geïnstalleerd",
        "Brak zainstalowanej pasującej czcionki",
        "لا يوجد خط مطابق مثبت",
        "לא מותקן גופן מתאים",
    ],
    [
        "Angezeigt in:",
        "Shown in:",
        "Affiché avec\u{A0}:",
        "Se muestra en:",
        "Visualizzato in:",
        "Weergegeven in:",
        "Wyświetlane czcionką:",
        "معروض بخط:",
        "מוצג בגופן:",
    ],
    // docking (dock.rs)
    [
        "Weitere Optionen",
        "More options",
        "Plus d’options",
        "Más opciones",
        "Altre opzioni",
        "Meer opties",
        "Więcej opcji",
        "مزيد من الخيارات",
        "אפשרויות נוספות",
    ],
    ["Einklappen", "Collapse", "Réduire", "Contraer", "Comprimi", "Samenvouwen", "Zwiń", "طي", "כיווץ"],
    ["Ausblenden", "Hide", "Masquer", "Ocultar", "Nascondi", "Verbergen", "Ukryj", "إخفاء", "הסתרה"],
    ["Lösen", "Float", "Détacher", "Desacoplar", "Sgancia", "Losmaken", "Odepnij", "فصل", "ניתוק"],
    ["Andocken", "Dock", "Ancrer", "Acoplar", "Ancora", "Vastzetten", "Zadokuj", "إرساء", "עגינה"],
    [
        "Links andocken",
        "Dock left",
        "Ancrer à gauche",
        "Acoplar a la izquierda",
        "Ancora a sinistra",
        "Links vastzetten",
        "Zadokuj po lewej",
        "إرساء إلى اليسار",
        "עגינה לשמאל",
    ],
    [
        "Rechts andocken",
        "Dock right",
        "Ancrer à droite",
        "Acoplar a la derecha",
        "Ancora a destra",
        "Rechts vastzetten",
        "Zadokuj po prawej",
        "إرساء إلى اليمين",
        "עגינה לימין",
    ],
    [
        "Unten andocken",
        "Dock at the bottom",
        "Ancrer en bas",
        "Acoplar abajo",
        "Ancora in basso",
        "Onderaan vastzetten",
        "Zadokuj na dole",
        "إرساء في الأسفل",
        "עגינה למטה",
    ],
    ["Als Register", "As a tab", "Comme onglet", "Como pestaña", "Come scheda", "Als tabblad", "Jako karta", "كعلامة تبويب", "ככרטיסייה"],
    ["Darüber", "Above", "Au-dessus", "Encima", "Sopra", "Erboven", "Powyżej", "فوق", "מעל"],
    ["Darunter", "Below", "En dessous", "Debajo", "Sotto", "Eronder", "Poniżej", "تحت", "מתחת"],
    ["Davor", "Before", "Avant", "Antes", "Prima", "Ervoor", "Przed", "قبل", "לפני"],
    ["Dahinter", "After", "Après", "Después", "Dopo", "Erna", "Za", "بعد", "אחרי"],
    // spell checking (edit_spell.rs)
    ["Keine Vorschläge", "No suggestions", "Aucune suggestion", "No hay sugerencias", "Nessun suggerimento", "Geen suggesties", "Brak sugestii", "لا توجد اقتراحات", "אין הצעות"],
    [
        "Vorschläge werden gesucht …",
        "Looking for suggestions…",
        "Recherche de suggestions…",
        "Buscando sugerencias…",
        "Ricerca dei suggerimenti…",
        "Suggesties zoeken…",
        "Wyszukiwanie sugestii…",
        "جارٍ البحث عن اقتراحات…",
        "חיפוש הצעות…",
    ],
    [
        "Zum Wörterbuch hinzufügen",
        "Add to dictionary",
        "Ajouter au dictionnaire",
        "Agregar al diccionario",
        "Aggiungi al dizionario",
        "Toevoegen aan woordenlijst",
        "Dodaj do słownika",
        "إضافة إلى القاموس",
        "הוספה למילון",
    ],
    ["Ignorieren", "Ignore", "Ignorer", "Ignorar", "Ignora", "Negeren", "Ignoruj", "تجاهل", "התעלמות"],
    // permission questions and the permissions page (perm.rs)
    ["Zugriff erlauben?", "Allow access?", "Autoriser l’accès\u{A0}?", "¿Permitir el acceso?", "Consentire l’accesso?", "Toegang toestaan?", "Zezwolić na dostęp?", "السماح بالوصول؟", "לאפשר גישה?"],
    ["Dateien lesen", "Read files", "Lire des fichiers", "Leer archivos", "Leggere file", "Bestanden lezen", "Odczytywać pliki", "قراءة الملفات", "קריאת קבצים"],
    ["Dateien schreiben", "Write files", "Écrire des fichiers", "Escribir archivos", "Scrivere file", "Bestanden schrijven", "Zapisywać pliki", "كتابة الملفات", "כתיבת קבצים"],
    ["Ein Programm starten", "Start a program", "Lancer un programme", "Iniciar un programa", "Avviare un programma", "Een programma starten", "Uruchomić program", "تشغيل برنامج", "הפעלת תוכנית"],
    ["Eine Adresse öffnen", "Open an address", "Ouvrir une adresse", "Abrir una dirección", "Aprire un indirizzo", "Een adres openen", "Otworzyć adres", "فتح عنوان", "פתיחת כתובת"],
    ["Ein Plugin laden", "Load a plug-in", "Charger un plug-in", "Cargar un complemento", "Caricare un plug-in", "Een plug-in laden", "Wczytać wtyczkę", "تحميل مكوّن إضافي", "טעינת תוסף"],
    ["Drucken", "Print", "Imprimer", "Imprimir", "Stampare", "Afdrukken", "Drukować", "طباعة", "הדפסה"],
    [
        "Gespeicherte Kennwörter verwenden",
        "Use saved passwords",
        "Utiliser les mots de passe enregistrés",
        "Usar contraseñas guardadas",
        "Usare le password salvate",
        "Opgeslagen wachtwoorden gebruiken",
        "Używać zapisanych haseł",
        "استخدام كلمات المرور المحفوظة",
        "שימוש בסיסמאות שמורות",
    ],
    [
        "Beim Anmelden starten",
        "Start at sign-in",
        "Démarrer à l’ouverture de session",
        "Iniciar al iniciar sesión",
        "Avviare all’accesso",
        "Starten bij aanmelden",
        "Uruchamiać przy logowaniu",
        "التشغيل عند تسجيل الدخول",
        "הפעלה בעת הכניסה",
    ],
    [
        "Eine Netzwerkverbindung aufbauen",
        "Connect to the network",
        "Se connecter au réseau",
        "Conectarse a la red",
        "Connettersi alla rete",
        "Verbinding maken met het netwerk",
        "Łączyć się z siecią",
        "الاتصال بالشبكة",
        "התחברות לרשת",
    ],
    [
        "Netzwerkverbindungen annehmen",
        "Accept network connections",
        "Accepter des connexions réseau",
        "Aceptar conexiones de red",
        "Accettare connessioni di rete",
        "Netwerkverbindingen accepteren",
        "Przyjmować połączenia sieciowe",
        "قبول اتصالات الشبكة",
        "קבלת חיבורי רשת",
    ],
    [
        "Eine Aktion für den Agenten ausführen",
        "Run an action for the agent",
        "Exécuter une action pour l’agent",
        "Ejecutar una acción para el agente",
        "Eseguire un’azione per l’agente",
        "Een actie voor de agent uitvoeren",
        "Wykonać akcję dla agenta",
        "تنفيذ إجراء للوكيل",
        "ביצוע פעולה עבור הסוכן",
    ],
    ["Für das Plugin: {}", "For the plug-in: {}", "Pour le plug-in\u{A0}: {}", "Para el complemento: {}", "Per il plug-in: {}", "Voor de plug-in: {}", "Dla wtyczki: {}", "للمكوّن الإضافي: {}", "עבור התוסף: {}"],
    ["Grund: {}", "Reason: {}", "Motif\u{A0}: {}", "Motivo: {}", "Motivo: {}", "Reden: {}", "Powód: {}", "السبب: {}", "סיבה: {}"],
    ["Regel: {}", "Rule: {}", "Règle\u{A0}: {}", "Regla: {}", "Regola: {}", "Regel: {}", "Reguła: {}", "القاعدة: {}", "כלל: {}"],
    ["Einmal erlauben", "Allow once", "Autoriser une fois", "Permitir una vez", "Consenti una volta", "Eenmaal toestaan", "Zezwól raz", "السماح مرة واحدة", "אפשר פעם אחת"],
    ["Immer erlauben", "Always allow", "Toujours autoriser", "Permitir siempre", "Consenti sempre", "Altijd toestaan", "Zawsze zezwalaj", "السماح دائمًا", "אפשר תמיד"],
    ["Ablehnen", "Deny", "Refuser", "Denegar", "Nega", "Weigeren", "Odmów", "رفض", "דחה"],
    ["Nie erlauben", "Never allow", "Ne jamais autoriser", "No permitir nunca", "Non consentire mai", "Nooit toestaan", "Nigdy nie zezwalaj", "عدم السماح أبدًا", "לעולם אל תאפשר"],
    [
        "Wird beim nächsten Zugriff gefragt",
        "Asked on the next access",
        "Demandé au prochain accès",
        "Se preguntará en el próximo acceso",
        "Richiesto al prossimo accesso",
        "Wordt bij de volgende toegang gevraagd",
        "Pytanie przy następnym dostępie",
        "سيُطلب عند الوصول التالي",
        "תוצג שאלה בגישה הבאה",
    ],
    [
        "Bis zum Programmende erlaubt",
        "Allowed until the program ends",
        "Autorisé jusqu’à la fermeture du programme",
        "Permitido hasta que se cierre el programa",
        "Consentito fino alla chiusura del programma",
        "Toegestaan tot het programma sluit",
        "Dozwolone do zamknięcia programu",
        "مسموح حتى إغلاق البرنامج",
        "מותר עד לסגירת התוכנית",
    ],
    ["Immer erlaubt", "Always allowed", "Toujours autorisé", "Siempre permitido", "Sempre consentito", "Altijd toegestaan", "Zawsze dozwolone", "مسموح دائمًا", "מותר תמיד"],
    [
        "Bis zum Programmende abgelehnt",
        "Denied until the program ends",
        "Refusé jusqu’à la fermeture du programme",
        "Denegado hasta que se cierre el programa",
        "Negato fino alla chiusura del programma",
        "Geweigerd tot het programma sluit",
        "Odmówiono do zamknięcia programu",
        "مرفوض حتى إغلاق البرنامج",
        "נדחה עד לסגירת התוכנית",
    ],
    ["Nie erlaubt", "Never allowed", "Jamais autorisé", "Nunca permitido", "Mai consentito", "Nooit toegestaan", "Nigdy niedozwolone", "غير مسموح أبدًا", "אסור תמיד"],
    [
        "Keine Rückfragen",
        "No permission questions",
        "Aucune demande d’autorisation",
        "No hay solicitudes de permiso",
        "Nessuna richiesta di autorizzazione",
        "Geen toestemmingsvragen",
        "Brak pytań o uprawnienia",
        "لا توجد طلبات أذونات",
        "אין בקשות הרשאה",
    ],
    ["Alle zurücksetzen", "Reset all", "Tout réinitialiser", "Restablecer todo", "Reimposta tutto", "Alles opnieuw instellen", "Resetuj wszystko", "إعادة تعيين الكل", "איפוס הכול"],
    // the parts of the colour picker the keyboard reaches (color.rs)
    ["Sättigung und Helligkeit", "Saturation and brightness", "Saturation et luminosité", "Saturación y brillo", "Saturazione e luminosità", "Verzadiging en helderheid", "Nasycenie i jasność", "التشبع والسطوع", "רוויה ובהירות"],
    ["Farbton", "Hue", "Teinte", "Matiz", "Tonalità", "Tint", "Odcień", "تدرج اللون", "גוון"],
    ["Deckkraft", "Opacity", "Opacité", "Opacidad", "Opacità", "Dekking", "Krycie", "العتامة", "אטימות"],
    // a splitter grip (shell.rs split views, dock.rs): F8 focuses it, the arrows move it
    ["Teiler", "Splitter", "Séparateur", "Divisor", "Divisore", "Splitser", "Rozdzielacz", "فاصل", "מפריד"],
    // (ROADMAP §6) the rich-text and code editors (rich_edit.rs, rich_doc.rs, code_edit.rs, code_doc.rs)
    ["Einfügen", "Paste", "Coller", "Pegar", "Incolla", "Plakken", "Wklej", "لصق", "הדבקה"],
    ["Löschen", "Delete", "Supprimer", "Eliminar", "Elimina", "Verwijderen", "Usuń", "حذف", "מחיקה"],
    ["Liste", "List", "Liste", "Lista", "Elenco", "Lijst", "Lista", "قائمة", "רשימה"],
    ["Absatz", "Paragraph", "Paragraphe", "Párrafo", "Paragrafo", "Alinea", "Akapit", "فقرة", "פסקה"],
    ["Unterstrichen", "Underline", "Souligné", "Subrayado", "Sottolineato", "Onderstrepen", "Podkreślenie", "تسطير", "קו תחתון"],
    ["Durchgestrichen", "Strikethrough", "Barré", "Tachado", "Barrato", "Doorhalen", "Przekreślenie", "يتوسطه خط", "קו חוצה"],
    ["Format", "Format", "Format", "Formato", "Formato", "Opmaak", "Formatowanie", "تنسيق", "עיצוב"],
    ["Textfarbe", "Text color", "Couleur du texte", "Color de texto", "Colore testo", "Tekstkleur", "Kolor tekstu", "لون النص", "צבע טקסט"],
    ["Hervorhebung", "Highlight", "Surlignage", "Resaltado", "Evidenziazione", "Markering", "Wyróżnienie", "تمييز", "הדגשה"],
    ["Schriftgröße", "Font size", "Taille de police", "Tamaño de fuente", "Dimensione carattere", "Tekengrootte", "Rozmiar czcionki", "حجم الخط", "גודל גופן"],
    ["Link", "Link", "Lien", "Enlace", "Collegamento", "Koppeling", "Link", "ارتباط", "קישור"],
    ["Format löschen", "Clear formatting", "Effacer la mise en forme", "Borrar formato", "Cancella formattazione", "Opmaak wissen", "Wyczyść formatowanie", "مسح التنسيق", "ניקוי עיצוב"],
    ["Ausrichtung", "Alignment", "Alignement", "Alineación", "Allineamento", "Uitlijning", "Wyrównanie", "محاذاة", "יישור"],
    ["Einzug", "Indent", "Retrait", "Sangría", "Rientro", "Inspringing", "Wcięcie", "مسافة بادئة", "כניסה"],
    ["Formatvorlage", "Style", "Style", "Estilo", "Stile", "Stijl", "Styl", "نمط", "סגנון"],
    ["Ersetzen", "Replace", "Remplacer", "Reemplazar", "Sostituisci", "Vervangen", "Zamień", "استبدال", "החלפה"],
    ["Link-Adresse", "Link address", "Adresse du lien", "Dirección del enlace", "Indirizzo collegamento", "Koppelingsadres", "Adres linku", "عنوان الارتباط", "כתובת קישור"],
    ["Link entfernen", "Remove link", "Supprimer le lien", "Quitar enlace", "Rimuovi collegamento", "Koppeling verwijderen", "Usuń link", "إزالة الارتباط", "הסרת קישור"],
    ["Rückgängig", "Undo", "Annuler", "Deshacer", "Annulla", "Ongedaan maken", "Cofnij", "تراجع", "ביטול פעולה"],
    ["Wiederholen", "Redo", "Rétablir", "Rehacer", "Ripeti", "Opnieuw", "Ponów", "إعادة", "ביצוע חוזר"],
    ["Überschrift 1", "Heading 1", "Titre 1", "Título 1", "Titolo 1", "Kop 1", "Nagłówek 1", "عنوان 1", "כותרת 1"],
    ["Überschrift 2", "Heading 2", "Titre 2", "Título 2", "Titolo 2", "Kop 2", "Nagłówek 2", "عنوان 2", "כותרת 2"],
    ["Überschrift 3", "Heading 3", "Titre 3", "Título 3", "Titolo 3", "Kop 3", "Nagłówek 3", "عنوان 3", "כותרת 3"],
    ["Zitat", "Quote", "Citation", "Cita", "Citazione", "Citaat", "Cytat", "اقتباس", "ציטוט"],
    ["Keine", "None", "Aucune", "Ninguno", "Nessuno", "Geen", "Brak", "بلا", "ללא"],
    ["Automatisch", "Automatic", "Automatique", "Automático", "Automatico", "Automatisch", "Automatyczny", "تلقائي", "אוטומטי"],
    ["Linksbündig", "Align left", "Aligner à gauche", "Alinear a la izquierda", "Allinea a sinistra", "Links uitlijnen", "Wyrównaj do lewej", "محاذاة لليسار", "יישור לשמאל"],
    ["Zentriert", "Center", "Centrer", "Centrar", "Centra", "Centreren", "Wyśrodkuj", "توسيط", "מרכוז"],
    ["Rechtsbündig", "Align right", "Aligner à droite", "Alinear a la derecha", "Allinea a destra", "Rechts uitlijnen", "Wyrównaj do prawej", "محاذاة لليمين", "יישור לימין"],
    ["Blocksatz", "Justify", "Justifier", "Justificar", "Giustifica", "Uitvullen", "Wyjustuj", "ضبط", "יישור לשני הצדדים"],
    ["Aufzählung", "Bullets", "Puces", "Viñetas", "Elenco puntato", "Opsommingstekens", "Punktory", "تعداد نقطي", "תבליטים"],
    ["Nummerierung", "Numbering", "Numérotation", "Numeración", "Elenco numerato", "Nummering", "Numerowanie", "ترقيم", "מספור"],
    ["Einzug verkleinern", "Decrease indent", "Diminuer le retrait", "Disminuir sangría", "Riduci rientro", "Inspringing verkleinen", "Zmniejsz wcięcie", "إنقاص المسافة البادئة", "הקטנת כניסה"],
    ["Einzug vergrößern", "Increase indent", "Augmenter le retrait", "Aumentar sangría", "Aumenta rientro", "Inspringing vergroten", "Zwiększ wcięcie", "زيادة المسافة البادئة", "הגדלת כניסה"],
    ["Zeilenumbruch", "Line break", "Saut de ligne", "Salto de línea", "Interruzione di riga", "Regeleinde", "Podział wiersza", "فاصل أسطر", "מעבר שורה"],
    ["Kommentar", "Comment", "Commentaire", "Comentario", "Commento", "Opmerking", "Komentarz", "تعليق", "הערה"],
    ["Zeilen verschieben", "Move lines", "Déplacer les lignes", "Mover líneas", "Sposta righe", "Regels verplaatsen", "Przenieś wiersze", "نقل الأسطر", "הזזת שורות"],
    ["Alle ersetzen", "Replace all", "Remplacer tout", "Reemplazar todo", "Sostituisci tutto", "Alles vervangen", "Zamień wszystko", "استبدال الكل", "החלפת הכול"],
    ["Groß-/Kleinschreibung", "Match case", "Respecter la casse", "Coincidir mayúsculas y minúsculas", "Maiuscole/minuscole", "Hoofdlettergevoelig", "Uwzględnij wielkość liter", "مطابقة حالة الأحرف", "התאמת רישיות"],
    ["Ganzes Wort", "Whole word", "Mot entier", "Palabra completa", "Parola intera", "Heel woord", "Całe wyrazy", "الكلمة بأكملها", "מילה שלמה"],
    ["Vorheriger Treffer", "Previous match", "Résultat précédent", "Coincidencia anterior", "Risultato precedente", "Vorige overeenkomst", "Poprzednie wystąpienie", "التطابق السابق", "ההתאמה הקודמת"],
    ["Nächster Treffer", "Next match", "Résultat suivant", "Coincidencia siguiente", "Risultato successivo", "Volgende overeenkomst", "Następne wystąpienie", "التطابق التالي", "ההתאמה הבאה"],
    ["Keine Treffer", "No results", "Aucun résultat", "Sin resultados", "Nessun risultato", "Geen resultaten", "Brak wyników", "لا توجد نتائج", "אין תוצאות"],
    // (L6) the severities of the code editor's diagnostics (tooltips, the record at the caret)
    ["Fehler", "Error", "Erreur", "Error", "Errore", "Fout", "Błąd", "خطأ", "שגיאה"],
    ["Warnung", "Warning", "Avertissement", "Advertencia", "Avviso", "Waarschuwing", "Ostrzeżenie", "تحذير", "אזהרה"],
    ["Hinweis", "Info", "Info", "Información", "Informazione", "Info", "Informacja", "معلومة", "מידע"],
    ["Schreiben", "Typing", "Saisie", "Escritura", "Digitazione", "Typen", "Pisanie", "كتابة", "הקלדה"],
    ["Fließtext", "Normal text", "Texte normal", "Texto normal", "Testo normale", "Normale tekst", "Zwykły tekst", "نص عادي", "טקסט רגיל"],
    ["Standardschrift", "Default font", "Police par défaut", "Fuente predeterminada", "Carattere predefinito", "Standaardlettertype", "Domyślna czcionka", "الخط الافتراضي", "גופן ברירת מחדל"],
    // (ROADMAP §6) the Markdown / HTML viewer's name for screen readers (md_view.rs)
    ["Dokument", "Document", "Document", "Documento", "Documento", "Document", "Dokument", "مستند", "מסמך"],
    // (ROADMAP §12) the live inspector's tool window (inspect.rs)
    ["Inspektor", "Inspector", "Inspecteur", "Inspector", "Ispettore", "Inspector", "Inspektor", "أداة الفحص", "כלי בדיקה"],
    ["Auswählen", "Pick", "Choisir", "Elegir", "Scegli", "Kiezen", "Wskaż", "انتقاء", "בחירה"],
    ["Umrisse", "Outlines", "Contours", "Contornos", "Contorni", "Omtrekken", "Obrysy", "الحدود", "קווי מתאר"],
    ["Tab-Reihenfolge", "Tab order", "Ordre de tabulation", "Orden de tabulación", "Ordine di tabulazione", "Tabvolgorde", "Kolejność tabulacji", "ترتيب التنقل", "סדר מעבר"],
    ["Neu gezeichnet", "Repaints", "Zones redessinées", "Zonas repintadas", "Aree ridisegnate", "Hertekend", "Odświeżone obszary", "المناطق المعاد رسمها", "אזורים שצוירו מחדש"],
    ["Widgets", "Widgets", "Widgets", "Widgets", "Widget", "Widgets", "Widżety", "عناصر الواجهة", "רכיבים"],
    ["Bildaufbau", "Frame", "Image", "Fotograma", "Fotogramma", "Frame", "Klatka", "الإطار", "פריים"],
    ["Farben", "Colors", "Couleurs", "Colores", "Colori", "Kleuren", "Kolory", "الألوان", "צבעים"],
    ["Nichts ausgewählt", "Nothing selected", "Aucune sélection", "Nada seleccionado", "Nessuna selezione", "Niets geselecteerd", "Nic nie zaznaczono", "لم يُحدَّد شيء", "לא נבחר דבר"],
    [
        "Klicken Sie auf ein Widget (Esc beendet)",
        "Click a widget (Esc ends)",
        "Cliquez sur un widget (Échap pour terminer)",
        "Haga clic en un widget (Esc para terminar)",
        "Fai clic su un widget (Esc per terminare)",
        "Klik op een widget (Esc stopt)",
        "Kliknij widżet (Esc kończy)",
        "انقر عنصرًا (Esc للإنهاء)",
        "לחצו על רכיב (Esc לסיום)",
    ],
    ["Kopieren", "Copy", "Copier", "Copiar", "Copia", "Kopiëren", "Kopiuj", "نسخ", "העתקה"],
    ["Art", "Kind", "Type", "Tipo", "Tipo", "Soort", "Rodzaj", "النوع", "סוג"],
    ["Kennung", "ID", "Identifiant", "Identificador", "Identificatore", "ID", "Identyfikator", "المعرّف", "מזהה"],
    ["Beschriftung", "Label", "Libellé", "Etiqueta", "Etichetta", "Label", "Etykieta", "التسمية", "תווית"],
    ["Rechteck", "Rectangle", "Rectangle", "Rectángulo", "Rettangolo", "Rechthoek", "Prostokąt", "المستطيل", "מלבן"],
    ["Wert", "Value", "Valeur", "Valor", "Valore", "Waarde", "Wartość", "القيمة", "ערך"],
    ["Zustand", "State", "État", "Estado", "Stato", "Status", "Stan", "الحالة", "מצב"],
    ["Ebene", "Layer", "Calque", "Capa", "Livello", "Laag", "Warstwa", "الطبقة", "שכבה"],
    ["Fenster", "Window", "Fenêtre", "Ventana", "Finestra", "Venster", "Okno", "النافذة", "חלון"],
    ["Selektor", "Selector", "Sélecteur", "Selector", "Selettore", "Selector", "Selektor", "المحدِّد", "בורר"],
    ["Durchgänge", "Passes", "Passes", "Pasadas", "Passate", "Passes", "Przebiegi", "التمريرات", "מעברים"],
    ["Aufbau", "Build", "Construction", "Construcción", "Costruzione", "Opbouw", "Budowanie", "البناء", "בנייה"],
    ["Zeichnen", "Render", "Rendu", "Renderizado", "Rendering", "Weergave", "Renderowanie", "الرسم", "ציור"],
    ["Neu gezeichnete Fläche", "Repainted area", "Surface redessinée", "Área repintada", "Area ridisegnata", "Hertekend gebied", "Odświeżony obszar", "المساحة المعاد رسمها", "שטח שצויר מחדש"],
    ["Zeichenbefehle", "Draw commands", "Commandes de dessin", "Comandos de dibujo", "Comandi di disegno", "Tekenopdrachten", "Polecenia rysowania", "أوامر الرسم", "פקודות ציור"],
    ["Glyphen-Cache", "Glyph cache", "Cache des glyphes", "Caché de glifos", "Cache dei glifi", "Glyphcache", "Pamięć podręczna glifów", "ذاكرة الحروف المؤقتة", "מטמון גליפים"],
    ["Hauptfenster", "Main window", "Fenêtre principale", "Ventana principal", "Finestra principale", "Hoofdvenster", "Okno główne", "النافذة الرئيسية", "חלון ראשי"],
    // (ROADMAP §6) the image viewer (imgview.rs): the accessible name of ui.image_view
    ["Bild", "Image", "Image", "Imagen", "Immagine", "Afbeelding", "Obraz", "صورة", "תמונה"],
    // (L5) the line chart (chart.rs): its empty state and the words of its summary
    ["Keine Daten", "No data", "Aucune donnée", "Sin datos", "Nessun dato", "Geen gegevens", "Brak danych", "لا توجد بيانات", "אין נתונים"],
    ["aktuell", "current", "actuel", "actual", "attuale", "huidig", "bieżąca", "الحالية", "נוכחי"],
    ["min.", "min", "min", "mín.", "min", "min.", "min.", "الأدنى", "מינימום"],
    ["max.", "max", "max", "máx.", "max", "max.", "maks.", "الأقصى", "מקסימום"],
    // the names of the roles screen readers hear (plat::a11y::ROLE_TEXTS: UIA
    // LocalizedControlType, AT-SPI GetLocalizedRoleName), English as AT-SPI names them
    ["Bereich", "panel", "panneau", "panel", "pannello", "paneel", "panel", "لوحة", "חלונית"],
    ["Bezeichnung", "label", "étiquette", "etiqueta", "etichetta", "label", "etykieta", "تسمية", "תווית"],
    ["Schaltfläche", "push button", "bouton", "botón", "pulsante", "knop", "przycisk", "زر", "לחצן"],
    ["Kontrollkästchen", "check box", "case à cocher", "casilla", "casella di controllo", "selectievakje", "pole wyboru", "خانة اختيار", "תיבת סימון"],
    ["Optionsfeld", "radio button", "bouton radio", "botón de opción", "pulsante di opzione", "keuzerondje", "przycisk opcji", "زر اختيار", "לחצן אפשרות"],
    ["Umschalter", "toggle switch", "interrupteur", "interruptor", "interruttore", "schakelaar", "przełącznik", "مفتاح تبديل", "מתג"],
    ["Schieberegler", "slider", "curseur", "control deslizante", "dispositivo di scorrimento", "schuifregelaar", "suwak", "شريط تمرير", "מחוון"],
    ["Fortschrittsanzeige", "progress bar", "barre de progression", "barra de progreso", "barra di avanzamento", "voortgangsbalk", "pasek postępu", "شريط التقدم", "סרגל התקדמות"],
    ["Eingabefeld", "entry", "zone de saisie", "campo de texto", "campo di testo", "invoerveld", "pole tekstowe", "حقل إدخال", "שדה קלט"],
    ["Verweis", "link", "lien", "enlace", "collegamento", "koppeling", "łącze", "ارتباط", "קישור"],
    ["Listenfeld", "list box", "zone de liste", "cuadro de lista", "casella di riepilogo", "keuzelijst", "pole listy", "مربع قائمة", "תיבת רשימה"],
    ["Listeneintrag", "list item", "élément de liste", "elemento de lista", "elemento dell'elenco", "lijstitem", "element listy", "عنصر قائمة", "פריט ברשימה"],
    ["Menüleiste", "menu bar", "barre de menus", "barra de menús", "barra dei menu", "menubalk", "pasek menu", "شريط القوائم", "סרגל תפריטים"],
    ["Menü", "menu", "menu", "menú", "menu", "menu", "menu", "قائمة", "תפריט"],
    ["Menüeintrag", "menu item", "élément de menu", "elemento de menú", "voce di menu", "menu-item", "element menu", "بند قائمة", "פריט תפריט"],
    ["Registerleiste", "page tab list", "liste d'onglets", "lista de pestañas", "elenco schede", "tabbladenlijst", "lista kart", "قائمة علامات التبويب", "רשימת כרטיסיות"],
    ["Registerkarte", "page tab", "onglet", "pestaña", "scheda", "tabblad", "karta", "علامة تبويب", "כרטיסייה"],
    ["Kombinationsfeld", "combo box", "zone de liste déroulante", "cuadro combinado", "casella combinata", "keuzelijst met invoervak", "pole kombi", "مربع تحرير وسرد", "תיבה משולבת"],
    ["Grafik", "image", "image", "imagen", "immagine", "afbeelding", "obraz", "صورة", "תמונה"],
    ["Zeichenfläche", "canvas", "zone de dessin", "lienzo", "area di disegno", "tekenvlak", "obszar rysowania", "لوحة رسم", "משטח ציור"],
    ["Dialog", "dialog", "boîte de dialogue", "cuadro de diálogo", "finestra di dialogo", "dialoogvenster", "okno dialogowe", "مربع حوار", "תיבת דו-שיח"],
    ["Überschrift", "heading", "titre", "encabezado", "intestazione", "kop", "nagłówek", "عنوان", "כותרת"],
    ["Rahmen", "frame", "cadre", "marco", "cornice", "kader", "ramka", "إطار", "מסגרת"],
    ["Anwendung", "application", "application", "aplicación", "applicazione", "toepassing", "aplikacja", "تطبيق", "יישום"],
    ["unbekannt", "unknown", "inconnu", "desconocido", "sconosciuto", "onbekend", "nieznany", "غير معروف", "לא ידוע"],
];

/// a library text in the current language: the UI language when the library has it
/// (library_languages), else English; the German source when the text is not in the table
pub fn tr(de: &'static str) -> &'static str {
    let l = li();
    if l == 0 {
        return de;
    }
    LIB_TEXTS.iter().find(|r| r[0] == de).map_or(de, |r| r[l])
}

/// the languages of the library's own texts (tr, month_name, weekday_short): "de", "en", "fr",
/// "es", "it", "nl", "pl", "ar", "he"
pub fn library_languages() -> &'static [&'static str] {
    &LIB_LANGS
}

/// the German source texts of the library's own texts (the keys of tr)
pub fn library_keys() -> impl Iterator<Item = &'static str> {
    LIB_TEXTS.iter().map(|r| r[0])
}

// calendar names: stand-alone forms (the date picker's title "Oktober 2026"), in the order of
// LIB_LANGS
static MONTHS: [[&str; 12]; 9] = [
    ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"],
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"],
    ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
    ["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"],
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"],
    ["styczeń", "luty", "marzec", "kwiecień", "maj", "czerwiec", "lipiec", "sierpień", "wrzesień", "październik", "listopad", "grudzień"],
    ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"],
    ["ינואר", "פברואר", "מרץ", "אפריל", "מאי", "יוני", "יולי", "אוגוסט", "ספטמבר", "אוקטובר", "נובמבר", "דצמבר"],
];
static MONTHS_SHORT: [[&str; 12]; 9] = [
    ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"],
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
    ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"],
    ["gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic"],
    ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"],
    ["sty", "lut", "mar", "kwi", "maj", "cze", "lip", "sie", "wrz", "paź", "lis", "gru"],
    ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"],
    ["ינו׳", "פבר׳", "מרץ", "אפר׳", "מאי", "יוני", "יולי", "אוג׳", "ספט׳", "אוק׳", "נוב׳", "דצמ׳"],
];
// the calendar's column heads (Arabic: the one-letter forms of its calendars)
static WEEKDAYS_SHORT: [[&str; 7]; 9] = [
    ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"],
    ["lu", "ma", "me", "je", "ve", "sa", "di"],
    ["lu", "ma", "mi", "ju", "vi", "sá", "do"],
    ["lu", "ma", "me", "gi", "ve", "sa", "do"],
    ["ma", "di", "wo", "do", "vr", "za", "zo"],
    ["pn", "wt", "śr", "cz", "pt", "so", "nd"],
    ["ن", "ث", "ر", "خ", "ج", "س", "ح"],
    ["ב׳", "ג׳", "ד׳", "ה׳", "ו׳", "ש׳", "א׳"],
];
static WEEKDAYS_ABBR: [[&str; 7]; 9] = [
    ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
    ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
    ["lun", "mar", "mer", "gio", "ven", "sab", "dom"],
    ["ma", "di", "wo", "do", "vr", "za", "zo"],
    ["pon.", "wt.", "śr.", "czw.", "pt.", "sob.", "niedz."],
    ["الاثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة", "السبت", "الأحد"],
    ["יום ב׳", "יום ג׳", "יום ד׳", "יום ה׳", "יום ו׳", "שבת", "יום א׳"],
];

fn li() -> usize {
    LIB.load(Ordering::Relaxed) as usize
}

/// the name of month m (1..12) in the current language
pub fn month_name(m: u32) -> &'static str {
    MONTHS[li()][((m.max(1) - 1) % 12) as usize]
}

/// the short name of month m (1..12)
pub fn month_short(m: u32) -> &'static str {
    MONTHS_SHORT[li()][((m.max(1) - 1) % 12) as usize]
}

/// the two-letter name of weekday d (0 = Monday .. 6 = Sunday)
pub fn weekday_short(d: u32) -> &'static str {
    WEEKDAYS_SHORT[li()][(d % 7) as usize]
}

/// the usual abbreviation of weekday d (0 = Monday) in running text: "Sa" / "Sat"
pub fn weekday_abbr(d: u32) -> &'static str {
    WEEKDAYS_ABBR[li()][(d % 7) as usize]
}

/// a translation catalogue of an app (see the module comment)
pub struct Catalog {
    langs: Vec<String>,
    rows: BTreeMap<&'static str, Vec<&'static str>>,
}

impl Catalog {
    /// parse a TSV catalogue (first line: the language codes); "\n" and "\t" escapes in the
    /// cells are not interpreted
    pub fn from_tsv(text: &'static str) -> Catalog {
        let mut lines = text.lines();
        let langs: Vec<String> = lines.next().map(|h| h.split('\t').map(|s| String::from(s.trim())).collect()).unwrap_or_default();
        let mut rows = BTreeMap::new();
        for line in lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split('\t');
            let Some(key) = cols.next() else { continue };
            let rest: Vec<&'static str> = cols.collect();
            rows.insert(key, rest);
        }
        Catalog { langs, rows }
    }

    /// the language codes of the columns ("de", "en", ...)
    pub fn languages(&self) -> &[String] {
        &self.langs
    }

    /// the text in the current language (the source text when the cell is missing or empty)
    pub fn tr<'a>(&'a self, s: &'a str) -> &'a str {
        let l = language() as usize;
        if l == 0 {
            return s;
        }
        match self.rows.get(s).and_then(|r| r.get(l - 1)) {
            Some(t) if !t.is_empty() => t,
            _ => s,
        }
    }

    /// whether the catalogue has a row for this source text
    pub fn has(&self, s: &str) -> bool {
        self.rows.contains_key(s)
    }
}

static APP: rt::sync::SpinLock<Option<&'static Catalog>> = rt::sync::SpinLock::new(None);

/// install the app's catalogue for the whole process (t() reads it); its column follows the
/// user's languages unless set_language (the same column) or set_language_code (the best
/// column for that code) came first
pub fn set_catalog(c: Catalog) {
    let r: &'static Catalog = Box::leak(Box::new(c));
    *APP.lock() = Some(r);
    let code = EXPLICIT_CODE.lock().clone();
    if let Some(code) = code {
        select(pick(&[code]));
    } else if EXPLICIT.load(Ordering::Relaxed) {
        select(language());
    } else {
        select(pick(&user_languages()));
    }
}

/// the app's text in the current language: the installed catalogue, then the library's own
/// texts, then the German source
pub fn t(s: &'static str) -> &'static str {
    if language() == LANG_DE {
        return s;
    }
    let cat = *APP.lock();
    match cat.and_then(|c| c.get_static(s)) {
        Some(x) => x,
        None => tr(s),
    }
}

/// (perm.rs) a text that is not 'static (a manifest's reason): the app's catalogue, else the
/// text itself
pub(crate) fn t_text(s: &str) -> String {
    let cat = if language() == LANG_DE { None } else { *APP.lock() };
    String::from(cat.and_then(|c| c.get_static(s)).unwrap_or(s))
}

/// replace the "{}" placeholders of a text in order
pub fn fill(text: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    let mut i = 0;
    while let Some(p) = rest.find("{}") {
        out.push_str(&rest[..p]);
        out.push_str(args.get(i).copied().unwrap_or(""));
        i += 1;
        rest = &rest[p + 2..];
    }
    out.push_str(rest);
    out
}

impl Catalog {
    /// the translation of a source text in the current language, when the catalogue has one
    pub fn get_static(&self, s: &str) -> Option<&'static str> {
        let l = language() as usize;
        if l == 0 {
            return None;
        }
        self.rows.get(s).and_then(|r| r.get(l - 1)).copied().filter(|t| !t.is_empty())
    }
}
