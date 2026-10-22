// i18ntest -- self-checks of the languages, regional formats and messages (ui::i18n,
// plat::locale): tag normalisation, the system's languages from the environment, language
// negotiation, numbers / currency / sizes / dates of several locales, CLDR plural categories,
// messages with named, numbered, plural and select placeholders, collation by language
// (strengths, numbers, the rules of de, sv, fi, da, nb, es, cs, sk, pl, tr, Cyrillic, Greek),
// case mapping (ß, final sigma, Turkish i), the library's texts in all its languages (also the
// role names screen readers hear through plat::a11y).
// Console program; exit code 1 when a check fails.
//
//   python build_rs.py app i18ntest
//   build/win/i18ntest.exe [--only NAME]      (build_rs.py test runs it through tests/all.uis;
//                                             --script is ignored)
//
// No window and no ui::main!: the process starts with the defaults (German, "de-DE"), as a
// headless ui run does.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use datetime::Civil;
use plat::a11y::Role;
use ui::i18n::{
    self, Arg, COLL_IDENTICAL, COLL_PRIMARY, COLL_SECONDARY, Catalog, Collator, DATE_LONG, DATE_SHORT, LANG_DE, LANG_EN, Locale, Plural, msg,
    plural, plural_int,
};

struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

impl T {
    fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}\n", self.group, name).as_bytes());
        }
    }
    fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
}

const NB: char = '\u{A0}';
const NN: char = '\u{202F}';

fn s(x: &str) -> String {
    String::from(x)
}

// a test string with the no-break spaces written as '~' (U+00A0) and '^' (U+202F)
fn sp(x: &str) -> String {
    x.chars().map(|c| if c == '~' { NB } else if c == '^' { NN } else { c }).collect()
}

fn day() -> Civil {
    Civil { year: 2026, month: 10, day: 17, hour: 19, minute: 58, second: 7, nanos: 0 }
}

fn tags(t: &mut T) {
    let tag = |x: &str| plat::locale_tag(x);
    t.eq("de_DE.UTF-8@euro", tag("de_DE.UTF-8@euro"), Some(s("de-DE")));
    t.eq("en-us", tag("en-us"), Some(s("en-US")));
    t.eq("zh_hant_tw", tag("zh_hant_tw"), Some(s("zh-Hant-TW")));
    t.eq("sr_RS@latin", tag("sr_RS@latin"), Some(s("sr-RS")));
    t.eq("es-419", tag("es-419"), Some(s("es-419")));
    t.eq("C", tag("C"), None);
    t.eq("POSIX", tag("POSIX.UTF-8"), None);
    t.eq("empty", tag(""), None);
    t.eq("garbage", tag("1x-DE"), None);
}

// the environment of Linux, the browser and Android (Windows asks the system)
fn system(t: &mut T) {
    #[cfg(not(windows))]
    {
        let set = |n: &str, v: Option<&str>| {
            plat::setenv(n, v);
        };
        for n in ["LC_ALL", "LC_MESSAGES", "LC_TIME", "LC_NUMERIC", "LANG", "LANGUAGE"] {
            set(n, None);
        }
        set("LANG", Some("de_DE.UTF-8"));
        set("LANGUAGE", Some("fr_CA:en"));
        t.eq("LANGUAGE before LANG", plat::system_languages(), alloc::vec![s("fr-CA"), s("en"), s("de-DE")]);
        t.eq("format from LANG", plat::system_locale(), Some(s("de-DE")));
        set("LC_TIME", Some("en_GB.UTF-8"));
        t.eq("format from LC_TIME", plat::system_locale(), Some(s("en-GB")));
        set("LANG", Some("C"));
        set("LC_TIME", None);
        t.eq("LANGUAGE ignored under C", plat::system_languages(), alloc::vec::Vec::<String>::new());
        t.eq("no format under C", plat::system_locale(), None);
        set("LANG", None);
        set("LANGUAGE", Some("pt-BR,en"));
        t.eq("host list (browser)", plat::system_languages(), alloc::vec![s("pt-BR"), s("en")]);
        t.eq("format from the first language", plat::system_locale(), Some(s("pt-BR")));
        set("LANGUAGE", None);
    }
    #[cfg(windows)]
    {
        let l = plat::system_languages();
        t.check("Windows: valid tags", l.iter().all(|x| plat::locale_tag(x).as_deref() == Some(x.as_str())));
        t.check("Windows: a regional format", plat::system_locale().is_some());
        plat::write_stdout(format!("  system: {:?}, format {:?}\n", l, plat::system_locale()).as_bytes());
    }
}

// languages: negotiation, the catalogue column, library texts (before any set_language)
fn languages(t: &mut T) {
    t.eq("defaults", (i18n::language(), i18n::language_code(), i18n::user_languages()), (LANG_DE, s("de"), alloc::vec![s("de-DE")]));
    t.eq("negotiate: region", i18n::negotiate(&["de", "en", "fr"], &["fr-CA", "en"]), Some(2));
    t.eq("negotiate: order", i18n::negotiate(&["de", "en", "fr"], &["it", "en-GB", "fr"]), Some(1));
    t.eq("negotiate: case", i18n::negotiate(&["de", "en-US"], &["EN_us"]), Some(1));
    t.eq("negotiate: none", i18n::negotiate(&["de", "en"], &["it"]), None);
    i18n::set_user_languages(&["fr-CA", "en"]);
    i18n::set_catalog(Catalog::from_tsv("de\ten\tfr\nÖffnen?\tOpen?\tOuvrir ?\n"));
    t.eq("catalogue follows the user", (i18n::language(), i18n::language_code()), (2, s("fr")));
    t.eq("app text", i18n::t("Öffnen?"), "Ouvrir ?");
    t.eq("library text in French", i18n::tr("Öffnen"), "Ouvrir");
    t.eq("month of the UI language", i18n::month_name(10), "octobre");
    i18n::set_user_languages(&["it"]);
    t.eq("no fit: English", (i18n::language(), i18n::tr("Öffnen")), (LANG_EN, "Open"));
    i18n::set_language(LANG_DE);
    t.eq("explicit", (i18n::tr("Öffnen"), i18n::month_name(3)), ("Öffnen", "März"));
    i18n::set_user_languages(&["fr"]);
    t.eq("explicit stays", i18n::language(), LANG_DE);
    t.eq("set_language_code", (i18n::set_language_code("fr-BE"), i18n::language_code()), (2, s("fr")));
    i18n::set_catalog(Catalog::from_tsv("de\tru\tfr\n"));
    t.eq("the code is matched again", (i18n::language(), i18n::language_code()), (2, s("fr")));
    i18n::set_catalog(Catalog::from_tsv("de\tfr\n"));
    t.eq("... in every catalogue", (i18n::language(), i18n::language_code()), (1, s("fr")));
    i18n::set_language(1);
    i18n::set_catalog(Catalog::from_tsv("de\tru\n"));
    t.eq("set_language keeps the column", i18n::language_code(), s("ru"));
    // written right to left: ui mirrors its windows then
    t.check("Russian is LTR", !i18n::rtl_language());
    i18n::set_catalog(Catalog::from_tsv("de\tar\the\n"));
    t.check("Arabic is RTL", i18n::rtl_language());
    i18n::set_language_code("he-IL");
    t.check("Hebrew is RTL", i18n::rtl_language() && i18n::language_code() == "he");
    i18n::set_language(LANG_DE);
    t.check("German is LTR", !i18n::rtl_language());
    // jfgui_LANG: languages and format
    plat::setenv("jfgui_LANG", Some("fr-CA:en"));
    i18n::use_system();
    t.eq("jfgui_LANG", (i18n::user_languages(), i18n::locale_tag(), i18n::locale().tag()), (alloc::vec![s("fr-CA"), s("en")], s("fr-CA"), "fr-CA"));
    plat::setenv("jfgui_LANG", None);
    i18n::set_locale("de-DE");
    i18n::set_catalog(Catalog::from_tsv("de\ten\n"));
    i18n::set_language(LANG_DE);
}

fn lookup(t: &mut T) {
    for (q, want) in [
        ("de-AT", "de-AT"),
        ("de-LU", "de-DE"),
        ("de", "de-DE"),
        ("en", "en-US"),
        ("en-ZA", "en-001"),
        ("en_GB.UTF-8", "en-GB"),
        ("zh-Hant", "zh-TW"),
        ("zh-HK", "zh-TW"),
        ("zh", "zh-CN"),
        ("no", "nb-NO"),
        ("nn-NO", "nb-NO"),
        ("pt", "pt-BR"),
        ("fr_CA", "fr-CA"),
        ("iw", "he-IL"),
        ("xx", "en-001"),
        ("C", "en-001"),
    ] {
        t.eq(q, Locale::get(q).tag(), want);
    }
    t.eq("first weekday", [Locale::get("de").first_weekday(), Locale::get("en-US").first_weekday(), Locale::get("ar").first_weekday()], [0, 6, 5]);
    t.eq("rtl", [Locale::get("he").rtl(), Locale::get("ar-SA").rtl(), Locale::get("de").rtl()], [true, true, false]);
    t.eq("currency", [Locale::get("de-CH").currency(), Locale::get("ja").currency()], ["CHF", "JPY"]);
}

fn numbers(t: &mut T) {
    let (de, en, fr, es, hi, ch) =
        (Locale::get("de"), Locale::get("en"), Locale::get("fr"), Locale::get("es"), Locale::get("hi"), Locale::get("de-CH"));
    t.check("signs", de.decimal() == ',' && de.group() == '.' && en.decimal() == '.' && fr.group() == NN);
    t.eq("de int", de.fmt_int(1234567), s("1.234.567"));
    t.eq("en int", en.fmt_int(-1234567), s("-1,234,567"));
    t.eq("hi int (lakh)", hi.fmt_int(1234567), s("12,34,567"));
    t.eq("en small", en.fmt_int(999), s("999"));
    t.eq("i64::MIN", en.fmt_int(i64::MIN), s("-9,223,372,036,854,775,808"));
    t.eq("es 4 digits", es.fmt_int(1234), s("1234"));
    t.eq("es 5 digits", es.fmt_int(12345), s("12.345"));
    t.eq("fr num", fr.fmt_num(1234567.891, 2), sp("1^234^567,89"));
    t.eq("de-CH num", ch.fmt_num(1234.5, 1), s("1’234.5"));
    t.eq("rounding", de.fmt_num(2.675, 1), s("2,7"));
    t.eq("negative zero", de.fmt_num(-0.004, 2), s("0,00"));
    t.eq("negative", de.fmt_num(-1.5, 1), s("-1,5"));
    t.eq("NaN", de.fmt_num(f64::NAN, 2), s("NaN"));
    t.eq("de percent", de.fmt_percent(0.256, 1), sp("25,6~%"));
    t.eq("en percent", en.fmt_percent(0.256, 0), s("26%"));
    t.eq("fr percent", fr.fmt_percent(0.5, 0), sp("50^%"));
    t.eq("tr percent", Locale::get("tr").fmt_percent(0.5, 0), s("%50"));
    t.eq("de EUR", de.fmt_currency(1234.5, "EUR"), sp("1.234,50~€"));
    t.eq("en USD", en.fmt_currency(1234.5, "usd"), s("$1,234.50"));
    t.eq("en EUR", en.fmt_currency(-5.0, "EUR"), s("-€5.00"));
    t.eq("de USD", de.fmt_currency(5.0, "USD"), sp("5,00~US$"));
    t.eq("de-CH money", ch.fmt_money(1234.5), sp("CHF~1’234.50"));
    t.eq("en CHF", en.fmt_currency(1.0, "CHF"), sp("CHF~1.00"));
    t.eq("ja money", Locale::get("ja").fmt_money(1234.6), s("￥1,235"));
    t.eq("unknown code", de.fmt_currency(5.0, "XYZ"), sp("5,00~XYZ"));
    t.eq("size B", de.fmt_size(512), sp("512~B"));
    t.eq("size KB", de.fmt_size(1536), sp("1,5~KB"));
    t.eq("size MB", en.fmt_size(10 << 20), sp("10~MB"));
    t.eq("size fr", fr.fmt_size(1536), sp("1,5~ko"));
    t.eq("free functions use locale()", i18n::fmt_int(1234), s("1.234"));
}

fn dates(t: &mut T) {
    let c = day();
    let f = |tag: &str, st: u32| Locale::get(tag).fmt_date(&c, st);
    t.eq("de short", f("de", DATE_SHORT), s("17.10.2026"));
    t.eq("de long", f("de", DATE_LONG), s("17. Oktober 2026"));
    t.eq("en-US short", f("en-US", DATE_SHORT), s("10/17/2026"));
    t.eq("en-US long", f("en-US", DATE_LONG), s("October 17, 2026"));
    t.eq("en-GB", f("en-GB", DATE_LONG), s("17 October 2026"));
    t.eq("en-CA", f("en-CA", DATE_SHORT), s("2026-10-17"));
    t.eq("es long", f("es", DATE_LONG), s("17 de octubre de 2026"));
    t.eq("pl long (genitive)", f("pl", DATE_LONG), s("17 października 2026"));
    t.eq("ru long", f("ru", DATE_LONG), s("17 октября 2026 г."));
    t.eq("ja long", f("ja", DATE_LONG), s("2026年10月17日"));
    t.eq("ko short", f("ko", DATE_SHORT), s("2026. 10. 17."));
    t.eq("hu long", f("hu", DATE_LONG), s("2026. október 17."));
    t.eq("he long", f("he", DATE_LONG), s("17 באוקטובר 2026"));
    t.eq("de time", Locale::get("de").fmt_time(&c), s("19:58"));
    t.eq("en time", Locale::get("en").fmt_time(&c), s("7:58 PM"));
    t.eq("fr-CA time", Locale::get("fr-CA").fmt_time(&c), s("19 h 58"));
    let mid = Civil { hour: 0, minute: 5, ..c };
    let noon = Civil { hour: 12, minute: 0, ..c };
    t.eq("midnight / noon", (Locale::get("en").fmt_time(&mid), Locale::get("en").fmt_time(&noon)), (s("12:05 AM"), s("12:00 PM")));
    t.eq("de date time", Locale::get("de").fmt_date_time(&c), s("17.10.2026, 19:58"));
    t.eq("en date time", Locale::get("en").fmt_date_time(&c), s("10/17/2026, 7:58 PM"));
}

fn plurals(t: &mut T) {
    use Plural::*;
    let cases: &[(&str, i64, Plural)] = &[
        ("de", 1, One),
        ("de", 0, Other),
        ("en", 2, Other),
        ("fr", 0, One),
        ("fr", 1, One),
        ("fr", 2, Other),
        ("ru", 1, One),
        ("ru", 2, Few),
        ("ru", 5, Many),
        ("ru", 11, Many),
        ("ru", 21, One),
        ("ru", 22, Few),
        ("ru-RU", 112, Many),
        ("pl", 1, One),
        ("pl", 22, Few),
        ("pl", 12, Many),
        ("pl", 5, Many),
        ("cs", 3, Few),
        ("cs", 5, Other),
        ("ar", 0, Zero),
        ("ar", 2, Two),
        ("ar", 3, Few),
        ("ar", 11, Many),
        ("ar", 100, Other),
        ("he", 2, Two),
        ("ja", 1, Other),
        ("ro", 101, Few),
        ("ro", 20, Other),
        ("sl", 102, Two),
        ("lt", 21, One),
        ("lv", 10, Zero),
    ];
    for &(l, n, want) in cases {
        t.eq(&format!("{l} {n}"), plural_int(l, n), want);
    }
    t.eq("de 1,5", plural("de", 1, 1, 5), Other);
    t.eq("de 1,0", plural("de", 1, 1, 0), Other);
    t.eq("fr 1,5", plural("fr", 1, 1, 5), One);
    t.eq("ru 1,5", plural("ru", 1, 1, 5), Other);
    t.eq("cs 1,5", plural("cs", 1, 1, 5), Many);
    t.eq("hi 1,5", plural("hi", 1, 1, 5), Other);
    t.eq("names", [Zero.name(), Few.name(), Other.name()], ["zero", "few", "other"]);
}

fn messages(t: &mut T) {
    let c = day();
    t.eq("named", msg("{n} Dateien in {dir}", &[("n", 3.into()), ("dir", "Bilder".into())]), s("3 Dateien in Bilder"));
    t.eq("numbered", msg("{1} vor {0}", &[("a", "x".into()), ("b", "y".into())]), s("y vor x"));
    t.eq("in order", msg("{} und {}", &[("a", "x".into()), ("b", "y".into())]), s("x und y"));
    let files = "{n, plural, =0 {keine Dateien} one {# Datei} other {# Dateien}}";
    t.eq("plural =0", msg(files, &[("n", 0.into())]), s("keine Dateien"));
    t.eq("plural one", msg(files, &[("n", 1.into())]), s("1 Datei"));
    t.eq("plural other, grouped #", msg(files, &[("n", 1234.into())]), s("1.234 Dateien"));
    t.eq("plural of a fraction", msg(files, &[("n", 1.5.into())]), s("1,5 Dateien"));
    let guests = "{n, plural, offset:1 =0 {niemand} =1 {nur {name}} one {{name} und # weitere Person} other {{name} und # weitere}}";
    let g = |n: i64| msg(guests, &[("n", n.into()), ("name", "Anna".into())]);
    t.eq("offset", [g(0), g(1), g(2), g(3)], [s("niemand"), s("nur Anna"), s("Anna und 1 weitere Person"), s("Anna und 2 weitere")]);
    let sel = "{g, select, female {Sie} male {Er} other {Es}} kommt";
    t.eq("select", (msg(sel, &[("g", "female".into())]), msg(sel, &[("g", "x".into())])), (s("Sie kommt"), s("Es kommt")));
    let nested = "{g, select, female {{n, plural, one {eine Frau} other {# Frauen}}} other {{n} Leute}}";
    t.eq("nested", msg(nested, &[("g", "female".into()), ("n", 2.into())]), s("2 Frauen"));
    t.eq("quotes", msg("geht's '{'n'}' = {n}, It''s", &[("n", 5.into())]), s("geht's {n} = 5, It's"));
    t.eq("quoted #", msg("{n, plural, other {'#' #}}", &[("n", 5.into())]), s("# 5"));
    t.eq("plain numbers", msg("{a} {b} {c}", &[("a", 1.5.into()), ("b", 2.0.into()), ("c", 1234.5.into())]), s("1,5 2 1234,5"));
    t.eq("number", msg("{n, number}", &[("n", 1234.5.into())]), s("1.234,5"));
    t.eq("integer", msg("{n, number, integer}", &[("n", 2.6.into())]), s("3"));
    t.eq("percent", msg("{p, number, percent}", &[("p", 0.25.into())]), sp("25~%"));
    t.eq("currency", msg("{a, number, ::currency/EUR}", &[("a", 1234.5.into())]), sp("1.234,50~€"));
    t.eq("size", msg("{s, number, size}", &[("s", 1536u64.into())]), sp("1,5~KB"));
    t.eq("dates", msg("{d, date} | {d, date, long} | {d, time} | {d, datetime}", &[("d", c.into())]),
        s("17.10.2026 | 17. Oktober 2026 | 19:58 | 17.10.2026, 19:58"));
    t.eq("missing", msg("{x} da", &[]), s("{x} da"));
    t.eq("broken", msg("a {n, plural, one {x", &[("n", 1.into())]), s("a {n, plural, one {x"));
    t.eq("stray brace", msg("a } b", &[]), s("a } b"));
    // the UI language decides the plural rules, the format the numbers
    i18n::set_language(LANG_EN);
    t.eq("English rules", msg("{n, plural, one {# file} other {# files}}", &[("n", 1.into())]), s("1 file"));
    i18n::set_catalog(Catalog::from_tsv("de\tru\n"));
    i18n::set_language_code("ru");
    let ru = "{n, plural, one {# файл} few {# файла} many {# файлов} other {# файла}}";
    let r = |n: Arg| msg(ru, &[("n", n)]);
    t.eq("Russian rules", [r(1.into()), r(3.into()), r(5.into()), r(21.into()), r(1.5.into())],
        [s("1 файл"), s("3 файла"), s("5 файлов"), s("21 файл"), s("1,5 файла")]);
    i18n::set_locale("en-US");
    t.eq("format of the locale", msg("{n, plural, many {# файлов} other {#}}", &[("n", 1235.into())]), s("1,235 файлов"));
    i18n::set_locale("de-DE");
    i18n::set_catalog(Catalog::from_tsv("de\ten\n"));
    i18n::set_language(LANG_DE);
}

// sort a scrambled copy of `want` with the collator: it must come out as `want`
fn order(t: &mut T, name: &str, c: &Collator, want: &[&str]) {
    let mut v: Vec<&str> = want.iter().rev().copied().collect();
    v.rotate_left(want.len() / 2);
    c.sort(&mut v, |x| *x);
    t.eq(name, v, want.to_vec());
}

fn collate(t: &mut T) {
    use core::cmp::Ordering::*;
    let c = Collator::new;
    order(t, "root: letters, accents, case", &c("en"), &["a", "A", "á", "Á", "ä", "b"]);
    order(t, "spaces, punctuation, digits, letters", &c("en"), &[" ", "-", "_", "1", "a"]);
    order(t, "de: ä like a", &c("de"), &["Ärger", "Arzt", "Asche", "Zebra"]);
    order(t, "de: ß like ss", &c("de"), &["Strasse", "Straße", "Strassen", "Strasser"]);
    order(t, "de", &c("de"), &["Mueller", "Muff", "Müller"]);
    order(t, "de phone book", &c("de").phonebook(true), &["Mueller", "Müller", "Muff"]);
    order(t, "-u-co-phonebk", &c("de-DE-u-co-phonebk"), &["Mueller", "Müller", "Muff"]);
    order(t, "sv: å ä ö after z, ü like y", &c("sv"), &["über", "yxa", "zebra", "åsna", "ärlig", "öra"]);
    order(t, "fi", &c("fi-FI"), &["zeta", "Åke", "äiti", "öljy"]);
    order(t, "da: æ ø å after z, aa = å", &c("da"), &["zebra", "Æble", "ørn", "Ålborg", "Aalborg"]);
    order(t, "nb", &c("nb-NO"), &["zoo", "ærlig", "øl", "år"]);
    order(t, "no -> nb", &c("no"), &["zoo", "ærlig", "øl", "år"]);
    order(t, "es: ñ after n", &c("es"), &["nube", "ñu", "oso"]);
    order(t, "root: ñ is n", &c("en"), &["ñu", "nube", "oso"]);
    order(t, "cs: ch after h", &c("cs"), &["hrad", "chata", "integrál"]);
    order(t, "cs: č ř š ž", &c("cs"), &["cukr", "čaj", "dům", "rybník", "řeka", "sůl", "šaty", "zima", "žena"]);
    order(t, "root: ch is c h", &c("en"), &["chata", "hrad"]);
    order(t, "sk: ä ô", &c("sk"), &["a", "ä", "b", "o", "ô", "p"]);
    order(t, "pl: ł ź ż", &c("pl"), &["luk", "łuk", "mama", "zebra", "źle", "żaba"]);
    order(t, "pl: ą ć ę ń ó ś", &c("pl"), &["a", "ą", "b", "c", "ć", "d", "e", "ę", "f", "n", "ń", "o", "ó", "p", "s", "ś", "t"]);
    order(t, "tr: ı before i, I its capital", &c("tr"), &["ılık", "Irmak", "incir", "İzmir"]);
    order(t, "tr: ç ğ ö ş ü", &c("tr"), &["c", "ç", "d", "g", "ğ", "h", "o", "ö", "p", "s", "ş", "t", "u", "ü", "v"]);
    order(t, "root: I is i", &c("en"), &["incir", "Irmak", "ılık"]);
    order(t, "ru: ё like е", &c("ru"), &["ёж", "ель", "жук"]);
    order(t, "uk: ґ є і ї й", &c("uk"), &["г", "ґ", "д", "е", "є", "ж", "и", "і", "ї", "й", "к"]);
    order(t, "sr: ј љ њ ћ џ", &c("sr"), &["й", "ј", "к", "л", "љ", "м", "н", "њ", "о", "т", "ћ", "у", "ч", "џ", "ш"]);
    order(t, "el: accents, final sigma", &c("el"), &["αλφα", "άλφα", "βήτα", "σοφοσ", "σοφος"]);
    let p = |tag: &str| Collator::new(tag).strength(COLL_PRIMARY);
    t.eq("he: final forms", p("he").compare("מם", "ממ"), Equal);
    t.eq("ar: alef with hamza", p("ar").compare("أحمد", "احمد"), Equal);
    t.eq("ja: katakana as hiragana, voiced", (p("ja").compare("かな", "カナ"), c("ja").compare("か", "が")), (Equal, Less));
    // numbers
    let n = c("de").numeric(true);
    order(t, "numeric", &n, &["Datei 2", "Datei 10", "Datei 100"]);
    order(t, "not numeric", &c("de"), &["Datei 10", "Datei 100", "Datei 2"]);
    order(t, "numeric versions", &n, &["v1.9", "v1.10", "v2"]);
    t.eq("leading zeros", (n.compare("Bild 7", "Bild 007"), n.strength(COLL_SECONDARY).compare("Bild 7", "Bild 007")), (Less, Equal));
    t.eq("Arabic-Indic digits", n.compare("٣", "10"), Less);
    t.eq("-u-kn", (c("en-u-kn").compare("2", "10"), c("en").compare("2", "10")), (Less, Greater));
    // strengths
    let de = c("de");
    t.eq("primary", [p("de").compare("a", "Ä"), p("de").compare("Straße", "STRASSE"), c("de-u-ks-level1").compare("a", "Ä")], [Equal, Equal, Equal]);
    let s2 = de.strength(COLL_SECONDARY);
    t.eq("secondary: without case", [s2.compare("a", "A"), s2.compare("a", "ä"), s2.compare("Ä", "a")], [Equal, Less, Greater]);
    t.eq("tertiary", [de.compare("a", "A"), de.compare("A", "ä")], [Less, Less]);
    t.eq("composed = decomposed", de.compare("e\u{301}", "é"), Equal);
    t.check("identical", de.strength(COLL_IDENTICAL).compare("e\u{301}", "é") != Equal);
    t.eq("soft hyphen ignored", de.compare("Ab\u{AD}c", "Abc"), Equal);
    t.check("keys compare like texts", de.key("Apfel") < de.key("Äpfel") && de.key("Äpfel") < de.key("Birne"));
    // the user's locale, a region
    t.eq("collate (de-DE)", i18n::collate("Öl", "Zebra"), Less);
    i18n::set_locale("sv-SE");
    t.eq("collate (sv-SE)", (i18n::collate("Öl", "Zebra"), i18n::collator().strength(COLL_SECONDARY).compare("Öl", "öl")), (Greater, Equal));
    i18n::set_locale("de-DE");
    t.eq("Locale::collate", [Locale::get("sv").collate("ö", "z"), Locale::get("de").collate("ö", "z")], [Greater, Less]);
    t.eq("Windows sort name", (c("de-DE_phoneb").compare("Müller", "Muff"), c("de-DE").compare("Müller", "Muff")), (Less, Greater));
}

fn case(t: &mut T) {
    use i18n::{fold_in, to_lower_in, to_upper_in};
    t.eq("ß -> SS", to_upper_in("de", "Straße"), s("STRASSE"));
    t.eq("final sigma", to_lower_in("el", "ΟΔΟΣ ΣΟΦΟΣ."), s("οδος σοφος."));
    t.eq("Turkish upper i", to_upper_in("tr", "istanbul ırmak"), s("İSTANBUL IRMAK"));
    t.eq("English upper i", to_upper_in("en", "istanbul"), s("ISTANBUL"));
    t.eq("Turkish lower I", to_lower_in("tr-TR", "IŞIK İZMİR"), s("ışık izmir"));
    t.eq("Azeri", to_lower_in("az", "I"), s("ı"));
    t.eq("Turkish I + dot", to_lower_in("tr", "I\u{307}"), s("i"));
    t.eq("English İ", (to_lower_in("en", "İ"), to_lower_in("en", "IŞIK")), (s("i\u{307}"), s("işik")));
    t.eq("title case digraph", (to_upper_in("hr", "ǆ"), to_lower_in("hr", "ǅ")), (s("Ǆ"), s("ǆ")));
    t.eq("ligature", to_upper_in("en", "ﬁx"), s("FIX"));
    t.eq("fold ß, ẞ", (fold_in("de", "Straße"), fold_in("de", "STRAẞE")), (s("strasse"), s("strasse")));
    t.eq("fold sigma", (fold_in("el", "ΣΟΦΟΣ"), fold_in("el", "σοφος")), (s("σοφοσ"), s("σοφοσ")));
    t.eq("fold Turkish", (fold_in("tr", "IİıI"), fold_in("en", "Iİı")), (s("ıiıı"), s("ii\u{307}ı")));
    t.eq("fold Cherokee", fold_in("en", "\u{AB70}\u{13A0}"), s("\u{13A0}\u{13A0}"));
    t.eq("fold ligature", fold_in("en", "ﬁ"), s("fi"));
    t.check("eq_ignore_case", i18n::eq_ignore_case("STRASSE", "Straße") && i18n::eq_ignore_case("É", "e\u{301}") && !i18n::eq_ignore_case("a", "ä"));
    // the user's locale
    t.eq("to_upper (de-DE)", i18n::to_upper("istanbul"), s("ISTANBUL"));
    i18n::set_locale("tr-TR");
    t.eq("tr-TR", (i18n::to_upper("istanbul"), i18n::to_lower("ISPARTA"), i18n::fold("I")), (s("İSTANBUL"), s("ısparta"), s("ı")));
    t.check("eq_ignore_case (tr-TR)", i18n::eq_ignore_case("IRMAK", "ırmak") && !i18n::eq_ignore_case("IRMAK", "irmak"));
    i18n::set_locale("de-DE");
}

fn distinct(v: &[&str]) -> bool {
    v.iter().enumerate().all(|(i, a)| !v[..i].contains(a))
}

// the library's own texts: every language has every text, the widgets' texts are keys
fn library(t: &mut T) {
    let langs = i18n::library_languages();
    t.eq("languages", langs, &["de", "en", "fr", "es", "it", "nl", "pl", "ar", "he"][..]);
    let keys: Vec<&str> = i18n::library_keys().collect();
    t.check("keys distinct", distinct(&keys));
    for key in [
        "Öffnen", "Speichern", "Speichern unter", "Abbrechen", "Dateiname:", "Übergeordneter Ordner", "Startordner", "Der Pfad ist zu lang.",
        "Die Datei wurde nicht gefunden.", "Schließen", "Zurücksetzen", "Weitere Befehle", "Vorherige Register", "Weitere Register",
        "Aufklappen", "Zuklappen", "Heute", "Vorheriger Monat", "Nächster Monat", "Strg", "Umschalt", "Alt", "Entf", "Einfg", "Eingabe",
        "Esc", "Rücktaste", "Leertaste", "Pos1", "Ende", "Bild auf", "Bild ab", "Nach-links", "Nach-rechts", "Nach-oben", "Nach-unten",
    ] {
        t.check(&format!("widget text {key}"), keys.contains(&key));
    }
    // the key names (from "Strg" on) may stay as they are
    let names = keys.iter().position(|k| *k == "Strg").unwrap_or(keys.len());
    i18n::set_catalog(Catalog::from_tsv("de\ten\tfr\tes\tit\tnl\tpl\tar\the\tru\n"));
    for lang in langs {
        i18n::set_language_code(lang);
        t.eq(&format!("{lang}: selected"), i18n::language_code(), s(lang));
        for (k, key) in keys.iter().enumerate() {
            let v = i18n::tr(key);
            t.check(&format!("{lang}: {key}"), !v.is_empty() && v.trim() == v);
            if *lang != "de" && k < names {
                t.check(&format!("{lang}: {key} translated"), v != *key);
            }
            if k < names && (*lang == "ar" || *lang == "he") {
                let script = if *lang == "ar" { '\u{600}'..='\u{6FF}' } else { '\u{590}'..='\u{5FF}' };
                t.check(&format!("{lang}: {key} in its script"), v.chars().any(|c| script.contains(&c)));
            }
        }
        let months: Vec<&str> = (1..=12).map(i18n::month_name).collect();
        let short: Vec<&str> = (1..=12).map(i18n::month_short).collect();
        let days: Vec<&str> = (0..7).map(i18n::weekday_short).collect();
        let abbr: Vec<&str> = (0..7).map(i18n::weekday_abbr).collect();
        for (what, v) in [("months", &months), ("short months", &short), ("weekdays", &days), ("weekday abbreviations", &abbr)] {
            t.check(&format!("{lang}: {what}"), v.iter().all(|x| !x.is_empty()) && distinct(v));
        }
        t.eq(&format!("{lang}: rtl"), i18n::rtl_language(), *lang == "ar" || *lang == "he");
    }
    for (lang, want) in [
        ("fr", ("Aujourd’hui", "octobre", "lu", "Ouvrir")),
        ("es", ("Hoy", "octubre", "lu", "Abrir")),
        ("it", ("Oggi", "ottobre", "lu", "Apri")),
        ("nl", ("Vandaag", "oktober", "ma", "Openen")),
        ("pl", ("Dzisiaj", "październik", "pn", "Otwórz")),
        ("ar", ("اليوم", "أكتوبر", "ن", "فتح")),
        ("he", ("היום", "אוקטובר", "ב׳", "פתיחה")),
    ] {
        i18n::set_language_code(lang);
        t.eq(lang, (i18n::tr("Heute"), i18n::month_name(10), i18n::weekday_short(0), i18n::tr("Öffnen")), want);
    }
    // the role names screen readers hear (plat::a11y::ROLE_TEXTS): library texts, in the UI
    // language once ui hands the bridges its translation (they follow a change of it)
    t.check("role names are library texts", plat::a11y::ROLE_TEXTS.iter().all(|k| keys.contains(k)));
    plat::a11y::set_translator(i18n::tr);
    let role = |r| plat::a11y::role_name(r);
    for (lang, want) in [
        ("de", ("Umschalter", "Überschrift", "Dialog")),
        ("en", ("toggle switch", "heading", "dialog")),
        ("fr", ("interrupteur", "titre", "boîte de dialogue")),
        ("he", ("מתג", "כותרת", "תיבת דו-שיח")),
    ] {
        i18n::set_language_code(lang);
        t.eq(&format!("{lang}: role names"), (role(Role::Switch), role(Role::Heading), role(Role::Dialog)), want);
    }
    i18n::set_language_code("ru");
    t.eq("other languages: English", (i18n::language_code(), i18n::tr("Öffnen"), i18n::month_name(1)), (s("ru"), "Open", "January"));
    t.eq("not a library text: the source", i18n::tr("Kein Bibliothekstext"), "Kein Bibliothekstext");
    i18n::set_catalog(Catalog::from_tsv("de\ten\n"));
    i18n::set_language(LANG_DE);
}

fn run() -> i32 {
    let only = plat::arg_value("--only").map(String::from);
    let groups: &[(&str, fn(&mut T))] = &[
        ("tags", tags),
        ("system", system),
        ("languages", languages),
        ("lookup", lookup),
        ("numbers", numbers),
        ("dates", dates),
        ("plurals", plurals),
        ("messages", messages),
        ("collate", collate),
        ("case", case),
        ("library", library),
    ];
    let (mut failed, mut total) = (0, 0);
    for (name, f) in groups {
        if only.as_deref().is_some_and(|o| o != *name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed).as_bytes());
        failed += t.failed;
        total += t.ok + t.failed;
    }
    plat::write_stdout(format!("i18ntest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
