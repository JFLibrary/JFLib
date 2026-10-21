// i18n_locale -- regional formats (a compact subset of CLDR) and plural categories
//
//   Locale::get(tag) -> Locale          the best match of a BCP 47 tag: the tag itself, the
//                                       language with the region, the language's default
//                                       ("de" -> de-DE, "en" -> en-US), else "en-001"
//   locale() / set_locale(tag) / locale_tag()   the user's format: ui boots it from
//                                       jfgui_LANG, the system (window mode) or "de-DE"
//                                       (headless runs, so shots match on every machine)
//   loc.fmt_int(n)                      "1.234.567" (de), "1,234,567" (en), "12,34,567" (hi)
//   loc.fmt_num(v, decimals)            fixed decimals, rounded; grouped
//   loc.fmt_percent(fraction, decimals) 0.25 -> "25 %" (de), "25%" (en), "%25" (tr)
//   loc.fmt_currency(v, "USD") / fmt_money(v)   "1.234,56 $" (de), "$1,234.56" (en-US);
//                                       fmt_money in the locale's own currency
//   loc.fmt_size(bytes)                 "1,5 MB" (1024 based; French "Mo")
//   loc.fmt_date(&civil, DATE_SHORT | DATE_LONG) / fmt_time(&civil) / fmt_date_time(&civil)
//                                       "17.10.2026" / "17. Oktober 2026" / "19:58";
//                                       en-US "10/17/2026" / "October 17, 2026" / "7:58 PM"
//   loc.first_weekday() (0 = Monday .. 6 = Sunday), rtl(), decimal(), group(), currency(), tag()
//   fmt_int / fmt_num / fmt_percent / fmt_currency / fmt_money / fmt_size / fmt_date /
//   fmt_time / fmt_date_time            the same with locale()
//   plural(lang, i, v, f) / plural_int(lang, n) -> Plural   CLDR cardinal categories (i the
//                                       integer digits, v the number of visible fraction
//                                       digits, f those digits as an integer)
//
// Numbers use ASCII digits everywhere (also Arabic and Hindi); spaces inside numbers and
// before units are no-break spaces (U+00A0, French U+202F).  Long dates use the format
// (genitive) month names of the locale's language.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use datetime::Civil;

const NBSP: char = '\u{A0}';
const NNBSP: char = '\u{202F}';

/// fmt_date: "17.10.2026" / "10/17/2026"
pub const DATE_SHORT: u32 = 0;
/// fmt_date: "17. Oktober 2026" / "October 17, 2026"
pub const DATE_LONG: u32 = 1;

// currency pattern
const CUR_PRE: u8 = 0; // "$1.00"
const CUR_PRE_SP: u8 = 1; // "€ 1,00"
const CUR_SUF: u8 = 2; // "1,00 €"
// percent pattern
const PCT_SUF: u8 = 0; // "5%"
const PCT_SUF_SP: u8 = 1; // "5 %" (U+00A0)
const PCT_SUF_NN: u8 = 2; // "5 %" (U+202F)
const PCT_PRE: u8 = 3; // "%5"

#[derive(Clone, Copy)]
struct Data {
    tag: &'static str,
    dec: char,
    grp: char,
    /// the size of the groups after the first (3; India 2)
    grp2: u8,
    /// 2: four-digit numbers stay ungrouped ("1234", "12 345")
    min_grp: u8,
    date: &'static str,
    date_long: &'static str,
    time: &'static str,
    dt_sep: &'static str,
    months: &'static [&'static str; 12],
    am_pm: [&'static str; 2],
    /// 0 = Monday .. 6 = Sunday
    first_day: u8,
    cur: &'static str,
    cur_pat: u8,
    pct: u8,
    rtl: bool,
}

const M_EN: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const M_DE: [&str; 12] =
    ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const M_FR: [&str; 12] =
    ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
const M_ES: [&str; 12] =
    ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"];
const M_IT: [&str; 12] =
    ["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"];
const M_NL: [&str; 12] =
    ["januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november", "december"];
const M_PT: [&str; 12] =
    ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];
const M_PL: [&str; 12] = [
    "stycznia", "lutego", "marca", "kwietnia", "maja", "czerwca", "lipca", "sierpnia", "września", "października", "listopada", "grudnia",
];
const M_CS: [&str; 12] =
    ["ledna", "února", "března", "dubna", "května", "června", "července", "srpna", "září", "října", "listopadu", "prosince"];
const M_SK: [&str; 12] =
    ["januára", "februára", "marca", "apríla", "mája", "júna", "júla", "augusta", "septembra", "októbra", "novembra", "decembra"];
const M_SV: [&str; 12] =
    ["januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september", "oktober", "november", "december"];
const M_DA: [&str; 12] =
    ["januar", "februar", "marts", "april", "maj", "juni", "juli", "august", "september", "oktober", "november", "december"];
const M_NB: [&str; 12] =
    ["januar", "februar", "mars", "april", "mai", "juni", "juli", "august", "september", "oktober", "november", "desember"];
const M_FI: [&str; 12] = [
    "tammikuuta", "helmikuuta", "maaliskuuta", "huhtikuuta", "toukokuuta", "kesäkuuta", "heinäkuuta", "elokuuta", "syyskuuta",
    "lokakuuta", "marraskuuta", "joulukuuta",
];
const M_RU: [&str; 12] =
    ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
const M_UK: [&str; 12] =
    ["січня", "лютого", "березня", "квітня", "травня", "червня", "липня", "серпня", "вересня", "жовтня", "листопада", "грудня"];
const M_TR: [&str; 12] = ["Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık"];
const M_EL: [&str; 12] = [
    "Ιανουαρίου", "Φεβρουαρίου", "Μαρτίου", "Απριλίου", "Μαΐου", "Ιουνίου", "Ιουλίου", "Αυγούστου", "Σεπτεμβρίου", "Οκτωβρίου",
    "Νοεμβρίου", "Δεκεμβρίου",
];
const M_HU: [&str; 12] =
    ["január", "február", "március", "április", "május", "június", "július", "augusztus", "szeptember", "október", "november", "december"];
const M_RO: [&str; 12] = [
    "ianuarie", "februarie", "martie", "aprilie", "mai", "iunie", "iulie", "august", "septembrie", "octombrie", "noiembrie", "decembrie",
];
const M_ID: [&str; 12] =
    ["Januari", "Februari", "Maret", "April", "Mei", "Juni", "Juli", "Agustus", "September", "Oktober", "November", "Desember"];
const M_AR: [&str; 12] = ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"];
const M_HE: [&str; 12] = ["ינואר", "פברואר", "מרץ", "אפריל", "מאי", "יוני", "יולי", "אוגוסט", "ספטמבר", "אוקטובר", "נובמבר", "דצמבר"];
const M_HI: [&str; 12] = ["जनवरी", "फ़रवरी", "मार्च", "अप्रैल", "मई", "जून", "जुलाई", "अगस्त", "सितंबर", "अक्तूबर", "नवंबर", "दिसंबर"];

const AM_PM: [&str; 2] = ["AM", "PM"];

const DE: Data = Data {
    tag: "de-DE",
    dec: ',',
    grp: '.',
    grp2: 3,
    min_grp: 1,
    date: "dd.MM.yyyy",
    date_long: "d. MMMM yyyy",
    time: "HH:mm",
    dt_sep: ", ",
    months: &M_DE,
    am_pm: AM_PM,
    first_day: 0,
    cur: "EUR",
    cur_pat: CUR_SUF,
    pct: PCT_SUF_SP,
    rtl: false,
};
const EN: Data = Data {
    tag: "en-US",
    dec: '.',
    grp: ',',
    date: "M/d/yyyy",
    date_long: "MMMM d, yyyy",
    time: "h:mm a",
    months: &M_EN,
    first_day: 6,
    cur: "USD",
    cur_pat: CUR_PRE,
    pct: PCT_SUF,
    ..DE
};
// English outside the table's regions (CLDR "en-001")
const EN_001: Data = Data { tag: "en-001", date: "dd/MM/yyyy", date_long: "d MMMM yyyy", time: "HH:mm", first_day: 0, ..EN };
const EN_GB: Data = Data { tag: "en-GB", cur: "GBP", ..EN_001 };
const FR: Data = Data {
    tag: "fr-FR",
    grp: NNBSP,
    date: "dd/MM/yyyy",
    date_long: "d MMMM yyyy",
    dt_sep: " ",
    months: &M_FR,
    pct: PCT_SUF_NN,
    ..DE
};
const ES: Data =
    Data { tag: "es-ES", min_grp: 2, date: "dd/MM/yyyy", date_long: "d 'de' MMMM 'de' yyyy", time: "H:mm", months: &M_ES, ..DE };
const IT: Data = Data { tag: "it-IT", date: "dd/MM/yyyy", date_long: "d MMMM yyyy", months: &M_IT, pct: PCT_SUF, ..DE };
const NL: Data = Data {
    tag: "nl-NL",
    date: "dd-MM-yyyy",
    date_long: "d MMMM yyyy",
    dt_sep: " ",
    months: &M_NL,
    cur_pat: CUR_PRE_SP,
    pct: PCT_SUF,
    ..DE
};
const PT: Data = Data {
    tag: "pt-BR",
    date: "dd/MM/yyyy",
    date_long: "d 'de' MMMM 'de' yyyy",
    dt_sep: " ",
    months: &M_PT,
    first_day: 6,
    cur: "BRL",
    cur_pat: CUR_PRE_SP,
    pct: PCT_SUF,
    ..DE
};
const PL: Data = Data { tag: "pl-PL", grp: NBSP, min_grp: 2, date_long: "d MMMM yyyy", months: &M_PL, cur: "PLN", pct: PCT_SUF, ..DE };
const CS: Data =
    Data { tag: "cs-CZ", grp: NBSP, date: "d. M. yyyy", time: "H:mm", dt_sep: " ", months: &M_CS, cur: "CZK", ..DE };
const SV: Data = Data {
    tag: "sv-SE",
    grp: NBSP,
    date: "yyyy-MM-dd",
    date_long: "d MMMM yyyy",
    dt_sep: " ",
    months: &M_SV,
    cur: "SEK",
    ..DE
};
const RU: Data =
    Data { tag: "ru-RU", grp: NBSP, date_long: "d MMMM yyyy 'г.'", months: &M_RU, cur: "RUB", ..DE };

static TABLE: &[Data] = &[
    // the first entry of a language is its default
    DE,
    Data { tag: "de-AT", grp: NBSP, cur_pat: CUR_PRE_SP, ..DE },
    Data { tag: "de-CH", dec: '.', grp: '’', cur: "CHF", cur_pat: CUR_PRE_SP, pct: PCT_SUF, ..DE },
    Data { tag: "de-LI", dec: '.', grp: '’', cur: "CHF", cur_pat: CUR_PRE_SP, pct: PCT_SUF, ..DE },
    EN,
    EN_GB,
    Data { tag: "en-IE", cur: "EUR", ..EN_001 },
    Data { tag: "en-AU", time: "h:mm a", cur: "AUD", ..EN_001 },
    Data { tag: "en-NZ", date: "d/MM/yyyy", time: "h:mm a", cur: "NZD", ..EN_001 },
    Data { tag: "en-CA", date: "yyyy-MM-dd", time: "h:mm a", first_day: 6, cur: "CAD", ..EN },
    Data { tag: "en-IN", grp2: 2, date: "dd/MM/yyyy", date_long: "d MMMM yyyy", first_day: 6, cur: "INR", ..EN },
    EN_001,
    FR,
    Data { tag: "fr-BE", ..FR },
    Data { tag: "fr-CA", grp: NBSP, date: "yyyy-MM-dd", time: "HH 'h' mm", first_day: 6, cur: "CAD", ..FR },
    Data { tag: "fr-CH", date: "dd.MM.yyyy", cur: "CHF", ..FR },
    ES,
    Data { tag: "es-MX", dec: '.', grp: ',', min_grp: 1, first_day: 6, cur: "MXN", cur_pat: CUR_PRE, pct: PCT_SUF, ..ES },
    Data { tag: "es-US", dec: '.', grp: ',', min_grp: 1, time: "h:mm a", first_day: 6, cur: "USD", cur_pat: CUR_PRE, pct: PCT_SUF, ..ES },
    IT,
    Data { tag: "it-CH", dec: '.', grp: '’', date: "dd.MM.yyyy", cur: "CHF", cur_pat: CUR_PRE_SP, ..IT },
    NL,
    Data { tag: "nl-BE", date: "d/MM/yyyy", cur_pat: CUR_SUF, ..NL },
    PT,
    Data { tag: "pt-PT", grp: NBSP, min_grp: 2, first_day: 0, cur: "EUR", cur_pat: CUR_SUF, ..PT },
    PL,
    CS,
    Data { tag: "sk-SK", months: &M_SK, cur: "EUR", ..CS },
    SV,
    Data { tag: "da-DK", date: "dd.MM.yyyy", date_long: "d. MMMM yyyy", time: "HH.mm", months: &M_DA, cur: "DKK", grp: '.', ..SV },
    Data { tag: "nb-NO", date: "dd.MM.yyyy", date_long: "d. MMMM yyyy", dt_sep: ", ", months: &M_NB, cur: "NOK", ..SV },
    Data { tag: "fi-FI", date: "d.M.yyyy", date_long: "d. MMMM yyyy", time: "H.mm", months: &M_FI, cur: "EUR", ..SV },
    RU,
    Data { tag: "uk-UA", date_long: "d MMMM yyyy 'р.'", months: &M_UK, cur: "UAH", pct: PCT_SUF, ..RU },
    Data { tag: "tr-TR", date_long: "d MMMM yyyy", dt_sep: " ", months: &M_TR, cur: "TRY", cur_pat: CUR_PRE, pct: PCT_PRE, ..DE },
    Data { tag: "el-GR", date: "d/M/yyyy", date_long: "d MMMM yyyy", months: &M_EL, pct: PCT_SUF, ..DE },
    Data {
        tag: "hu-HU",
        grp: NBSP,
        date: "yyyy. MM. dd.",
        date_long: "yyyy. MMMM d.",
        time: "H:mm",
        dt_sep: " ",
        months: &M_HU,
        cur: "HUF",
        pct: PCT_SUF,
        ..DE
    },
    Data { tag: "ro-RO", date_long: "d MMMM yyyy", months: &M_RO, cur: "RON", ..DE },
    Data { tag: "ja-JP", date: "yyyy/MM/dd", date_long: "yyyy年M月d日", time: "H:mm", dt_sep: " ", first_day: 6, cur: "JPY", ..EN },
    Data { tag: "zh-CN", date: "yyyy/M/d", date_long: "yyyy年M月d日", time: "HH:mm", dt_sep: " ", first_day: 0, cur: "CNY", ..EN },
    Data { tag: "zh-TW", date: "yyyy/M/d", date_long: "yyyy年M月d日", time: "HH:mm", dt_sep: " ", cur: "TWD", ..EN },
    Data { tag: "ko-KR", date: "yyyy. M. d.", date_long: "yyyy년 M월 d일", time: "HH:mm", dt_sep: " ", cur: "KRW", ..EN },
    Data {
        tag: "ar-EG",
        date: "d/M/yyyy",
        date_long: "d MMMM yyyy",
        time: "HH:mm",
        dt_sep: "، ",
        months: &M_AR,
        first_day: 5,
        cur: "EGP",
        cur_pat: CUR_SUF,
        rtl: true,
        ..EN
    },
    Data { tag: "ar-SA", date: "d/M/yyyy", date_long: "d MMMM yyyy", time: "HH:mm", dt_sep: "، ", months: &M_AR, cur: "SAR", cur_pat: CUR_SUF, rtl: true, ..EN },
    Data {
        tag: "he-IL",
        date: "d.M.yyyy",
        date_long: "d בMMMM yyyy",
        time: "H:mm",
        months: &M_HE,
        cur: "ILS",
        cur_pat: CUR_SUF,
        rtl: true,
        ..EN
    },
    Data { tag: "hi-IN", grp2: 2, date: "d/M/yyyy", date_long: "d MMMM yyyy", months: &M_HI, am_pm: ["am", "pm"], cur: "INR", ..EN },
    Data {
        tag: "id-ID",
        dec: ',',
        grp: '.',
        date: "dd/MM/yyyy",
        date_long: "d MMMM yyyy",
        time: "HH.mm",
        months: &M_ID,
        cur: "IDR",
        ..EN
    },
];

// currencies: code, symbol at home, symbol elsewhere, decimals
static CURRENCIES: &[(&str, &str, &str, u8)] = &[
    ("EUR", "€", "€", 2),
    ("USD", "$", "US$", 2),
    ("GBP", "£", "£", 2),
    ("CHF", "CHF", "CHF", 2),
    ("JPY", "￥", "¥", 0),
    ("CNY", "¥", "CN¥", 2),
    ("CAD", "$", "CA$", 2),
    ("AUD", "$", "A$", 2),
    ("NZD", "$", "NZ$", 2),
    ("INR", "₹", "₹", 2),
    ("BRL", "R$", "R$", 2),
    ("MXN", "$", "MX$", 2),
    ("PLN", "zł", "PLN", 2),
    ("CZK", "Kč", "CZK", 2),
    ("SEK", "kr", "SEK", 2),
    ("DKK", "kr.", "DKK", 2),
    ("NOK", "kr", "NOK", 2),
    ("RUB", "₽", "RUB", 2),
    ("UAH", "₴", "UAH", 2),
    ("TRY", "₺", "TRY", 2),
    ("HUF", "Ft", "HUF", 2),
    ("RON", "RON", "RON", 2),
    ("KRW", "₩", "₩", 0),
    ("TWD", "$", "NT$", 2),
    ("ILS", "₪", "₪", 2),
    ("EGP", "ج.م.", "EGP", 2),
    ("SAR", "ر.س.", "SAR", 2),
    ("IDR", "Rp", "IDR", 2),
];

/// the formats of one region (copy it freely)
#[derive(Clone, Copy)]
pub struct Locale {
    d: &'static Data,
}

impl Locale {
    /// the best match of a BCP 47 tag or a POSIX locale name (see the module comment)
    pub fn get(tag: &str) -> Locale {
        Locale { d: lookup(tag) }
    }

    /// the tag of the matched data ("de-DE", "en-001")
    pub fn tag(&self) -> &'static str {
        self.d.tag
    }
    /// the decimal sign
    pub fn decimal(&self) -> char {
        self.d.dec
    }
    /// the thousands separator
    pub fn group(&self) -> char {
        self.d.grp
    }
    /// the first day of the week: 0 = Monday .. 6 = Sunday
    pub fn first_weekday(&self) -> u32 {
        self.d.first_day as u32
    }
    /// right-to-left script (Arabic, Hebrew)
    pub fn rtl(&self) -> bool {
        self.d.rtl
    }
    /// the ISO 4217 code of the region's currency
    pub fn currency(&self) -> &'static str {
        self.d.cur
    }
    /// the long (format) name of month m (1..12) in the locale's language
    pub fn month_name(&self, m: u32) -> &'static str {
        self.d.months[((m.max(1) - 1) % 12) as usize]
    }

    // integer digits with the group separators
    fn group_digits(&self, digits: &str, out: &mut String) {
        let n = digits.len();
        if n < 3 + self.d.min_grp as usize {
            out.push_str(digits);
            return;
        }
        let mut cuts: Vec<usize> = Vec::new();
        let mut p = n as isize - 3;
        while p > 0 {
            cuts.push(p as usize);
            p -= self.d.grp2.max(1) as isize;
        }
        for (k, c) in digits.chars().enumerate() {
            if cuts.contains(&k) {
                out.push(self.d.grp);
            }
            out.push(c);
        }
    }

    /// an integer with group separators
    pub fn fmt_int(&self, n: i64) -> String {
        let mut out = String::new();
        if n < 0 {
            out.push('-');
        }
        self.group_digits(&format!("{}", n.unsigned_abs()), &mut out);
        out
    }

    /// a number with exactly `decimals` fraction digits (rounded) and group separators
    pub fn fmt_num(&self, v: f64, decimals: u32) -> String {
        self.num(v, decimals, true)
    }

    pub(crate) fn num(&self, v: f64, decimals: u32, grouped: bool) -> String {
        if v.is_nan() {
            return String::from("NaN");
        }
        if v.is_infinite() {
            return String::from(if v < 0.0 { "-∞" } else { "∞" });
        }
        let s = format!("{:.*}", decimals.min(17) as usize, if v < 0.0 { -v } else { v });
        let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
        let mut out = String::new();
        if v < 0.0 && s.bytes().any(|b| (b'1'..=b'9').contains(&b)) {
            out.push('-');
        }
        if grouped {
            self.group_digits(int, &mut out);
        } else {
            out.push_str(int);
        }
        if !frac.is_empty() {
            out.push(self.d.dec);
            out.push_str(frac);
        }
        out
    }

    /// a number as short as it is: up to 6 fraction digits, trailing zeros dropped, no groups
    pub(crate) fn num_plain(&self, v: f64) -> String {
        let mut s = self.num(v, 6, false);
        if s.contains(self.d.dec) {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with(self.d.dec) {
                s.pop();
            }
        }
        s
    }

    /// a fraction as percent: 0.25 -> "25 %" (de) / "25%" (en)
    pub fn fmt_percent(&self, fraction: f64, decimals: u32) -> String {
        let n = self.fmt_num(fraction * 100.0, decimals);
        match self.d.pct {
            PCT_SUF_SP => format!("{n}{NBSP}%"),
            PCT_SUF_NN => format!("{n}{NNBSP}%"),
            PCT_PRE => format!("%{n}"),
            _ => format!("{n}%"),
        }
    }

    /// an amount of the ISO 4217 currency `code` ("EUR", "USD"; unknown codes are shown as is)
    pub fn fmt_currency(&self, v: f64, code: &str) -> String {
        let c = CURRENCIES.iter().find(|c| c.0.eq_ignore_ascii_case(code));
        let (sym, dec) = match c {
            Some(c) if c.0 == self.d.cur => (c.1, c.3),
            Some(c) => (c.2, c.3),
            None => (code, 2),
        };
        let n = self.fmt_num(if v < 0.0 { -v } else { v }, dec as u32);
        let minus = if v < 0.0 && n.bytes().any(|b| (b'1'..=b'9').contains(&b)) { "-" } else { "" };
        let letter = sym.chars().last().is_some_and(char::is_alphabetic);
        match self.d.cur_pat {
            CUR_SUF => format!("{minus}{n}{NBSP}{sym}"),
            CUR_PRE_SP => format!("{minus}{sym}{NBSP}{n}"),
            _ if letter => format!("{minus}{sym}{NBSP}{n}"),
            _ => format!("{minus}{sym}{n}"),
        }
    }

    /// an amount in the region's own currency
    pub fn fmt_money(&self, v: f64) -> String {
        self.fmt_currency(v, self.d.cur)
    }

    /// a byte count: "512 B", "1,5 KB", "23 MB" (1024 based, one decimal below 10)
    pub fn fmt_size(&self, bytes: u64) -> String {
        let units: [&str; 6] =
            if self.d.tag.starts_with("fr") { ["o", "ko", "Mo", "Go", "To", "Po"] } else { ["B", "KB", "MB", "GB", "TB", "PB"] };
        if bytes < 1024 {
            return format!("{bytes}{NBSP}{}", units[0]);
        }
        let mut v = bytes as f64;
        let mut k = 0;
        while v >= 1024.0 && k < 5 {
            v /= 1024.0;
            k += 1;
        }
        format!("{}{NBSP}{}", self.fmt_num(v, if v < 9.95 { 1 } else { 0 }), units[k])
    }

    /// a date: DATE_SHORT "17.10.2026" / DATE_LONG "17. Oktober 2026"
    pub fn fmt_date(&self, c: &Civil, style: u32) -> String {
        pattern(self.d, if style == DATE_LONG { self.d.date_long } else { self.d.date }, c)
    }

    /// a time of day: "19:58" / "7:58 PM"
    pub fn fmt_time(&self, c: &Civil) -> String {
        pattern(self.d, self.d.time, c)
    }

    /// the short date and the time: "17.10.2026, 19:58" / "10/17/2026, 7:58 PM"
    pub fn fmt_date_time(&self, c: &Civil) -> String {
        let mut s = self.fmt_date(c, DATE_SHORT);
        s.push_str(self.d.dt_sep);
        s.push_str(&self.fmt_time(c));
        s
    }
}

// a date pattern: d dd M MM MMMM yy yyyy H HH h hh mm ss a, 'quoted text', anything else as is
fn pattern(d: &Data, pat: &str, c: &Civil) -> String {
    let mut out = String::new();
    let b: Vec<char> = pat.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let ch = b[i];
        if ch == '\'' {
            let end = b[i + 1..].iter().position(|&x| x == '\'').map_or(b.len(), |p| i + 1 + p);
            if end == i + 1 {
                out.push('\''); // '' -> '
            }
            out.extend(&b[i + 1..end]);
            i = end + 1;
            continue;
        }
        if !ch.is_ascii_alphabetic() {
            out.push(ch);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < b.len() && b[i + n] == ch {
            n += 1;
        }
        i += n;
        let h12 = match c.hour % 12 {
            0 => 12,
            h => h,
        };
        match (ch, n) {
            ('d', 1) => out.push_str(&format!("{}", c.day)),
            ('d', _) => out.push_str(&format!("{:02}", c.day)),
            ('M', 1) => out.push_str(&format!("{}", c.month)),
            ('M', 2) => out.push_str(&format!("{:02}", c.month)),
            ('M', _) => out.push_str(d.months[((c.month.max(1) - 1) % 12) as usize]),
            ('y', 2) => out.push_str(&format!("{:02}", c.year.rem_euclid(100))),
            ('y', _) => out.push_str(&format!("{}", c.year)),
            ('H', 1) => out.push_str(&format!("{}", c.hour)),
            ('H', _) => out.push_str(&format!("{:02}", c.hour)),
            ('h', 1) => out.push_str(&format!("{}", h12)),
            ('h', _) => out.push_str(&format!("{:02}", h12)),
            ('m', _) => out.push_str(&format!("{:02}", c.minute)),
            ('s', _) => out.push_str(&format!("{:02}", c.second)),
            ('a', _) => out.push_str(d.am_pm[(c.hour >= 12) as usize]),
            _ => {}
        }
    }
    out
}

fn lookup(tag: &str) -> &'static Data {
    let Some(t) = plat::locale_tag(tag) else { return &TABLE[index_of("en-001")] };
    if let Some(d) = TABLE.iter().find(|d| d.tag.eq_ignore_ascii_case(&t)) {
        return d;
    }
    let mut parts = t.split('-');
    let mut lang = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    let script = rest.iter().find(|p| p.len() == 4).copied();
    let mut region = rest.iter().find(|p| p.len() == 2 || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))).copied();
    // old and macro language codes, Chinese scripts
    lang = match lang {
        "no" | "nn" => "nb",
        "iw" => "he",
        "in" => "id",
        l => l,
    };
    if lang == "zh" {
        region = match (script, region) {
            (Some("Hant"), None) | (_, Some("HK" | "MO")) => Some("TW"),
            (_, r) => r,
        };
    }
    if let Some(r) = region {
        let want = format!("{lang}-{r}");
        if let Some(d) = TABLE.iter().find(|d| d.tag.eq_ignore_ascii_case(&want)) {
            return d;
        }
        // English of other regions: the international formats
        if lang == "en" {
            return &TABLE[index_of("en-001")];
        }
    }
    TABLE.iter().find(|d| d.tag.split('-').next() == Some(lang)).unwrap_or(&TABLE[index_of("en-001")])
}

fn index_of(tag: &str) -> usize {
    TABLE.iter().position(|d| d.tag == tag).unwrap_or(0)
}

// ---------------------------------------------------------------- the user's locale

static CURRENT: rt::sync::SpinLock<Option<(Locale, String)>> = rt::sync::SpinLock::new(None);

/// the user's regional format (default "de-DE")
pub fn locale() -> Locale {
    CURRENT.lock().as_ref().map_or(Locale { d: &TABLE[0] }, |c| c.0)
}

/// the tag the format was asked for ("de-LU" may use the data of "de-DE")
pub fn locale_tag() -> String {
    CURRENT.lock().as_ref().map_or_else(|| String::from("de-DE"), |c| c.1.clone())
}

/// set the user's regional format (a BCP 47 tag or a POSIX locale name)
pub fn set_locale(tag: &str) {
    let t = plat::locale_tag(tag).unwrap_or_else(|| String::from(tag));
    *CURRENT.lock() = Some((Locale::get(&t), t));
}

/// Locale::fmt_int in the user's format
pub fn fmt_int(n: i64) -> String {
    locale().fmt_int(n)
}
/// Locale::fmt_num in the user's format
pub fn fmt_num(v: f64, decimals: u32) -> String {
    locale().fmt_num(v, decimals)
}
/// Locale::fmt_percent in the user's format
pub fn fmt_percent(fraction: f64, decimals: u32) -> String {
    locale().fmt_percent(fraction, decimals)
}
/// Locale::fmt_currency in the user's format
pub fn fmt_currency(v: f64, code: &str) -> String {
    locale().fmt_currency(v, code)
}
/// Locale::fmt_money in the user's format
pub fn fmt_money(v: f64) -> String {
    locale().fmt_money(v)
}
/// Locale::fmt_size in the user's format
pub fn fmt_size(bytes: u64) -> String {
    locale().fmt_size(bytes)
}
/// Locale::fmt_date in the user's format
pub fn fmt_date(c: &Civil, style: u32) -> String {
    locale().fmt_date(c, style)
}
/// Locale::fmt_time in the user's format
pub fn fmt_time(c: &Civil) -> String {
    locale().fmt_time(c)
}
/// Locale::fmt_date_time in the user's format
pub fn fmt_date_time(c: &Civil) -> String {
    locale().fmt_date_time(c)
}

// ---------------------------------------------------------------- plural categories

/// a CLDR plural category
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plural {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Plural {
    /// the CLDR keyword: "zero", "one", "two", "few", "many", "other"
    pub fn name(self) -> &'static str {
        match self {
            Plural::Zero => "zero",
            Plural::One => "one",
            Plural::Two => "two",
            Plural::Few => "few",
            Plural::Many => "many",
            Plural::Other => "other",
        }
    }
}

/// the plural category of an integer in a language ("de", "ru-RU", ...)
pub fn plural_int(lang: &str, n: i64) -> Plural {
    plural(lang, n.unsigned_abs(), 0, 0)
}

/// the plural category of a number: i = its integer digits, v = the number of visible fraction
/// digits, f = those digits as an integer ("1,50": i 1, v 2, f 50)
pub fn plural(lang: &str, i: u64, v: u32, f: u64) -> Plural {
    use Plural::*;
    let l = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    let int = v == 0; // an integer (n = i)
    let (i10, i100) = (i % 10, i % 100);
    let (f10, f100) = (f % 10, f % 100);
    match l.as_str() {
        "ja" | "zh" | "ko" | "vi" | "th" | "id" | "ms" | "lo" | "km" | "my" | "yue" => Other,
        "fr" | "pt" => {
            if i <= 1 { One } else { Other }
        }
        "hi" | "bn" | "gu" | "kn" | "mr" | "fa" | "am" | "zu" => {
            if i == 0 || (i == 1 && f == 0) { One } else { Other }
        }
        "ru" | "uk" | "be" => {
            if !int {
                Other
            } else if i10 == 1 && i100 != 11 {
                One
            } else if (2..=4).contains(&i10) && !(12..=14).contains(&i100) {
                Few
            } else {
                Many
            }
        }
        "pl" => {
            if !int {
                Other
            } else if i == 1 {
                One
            } else if (2..=4).contains(&i10) && !(12..=14).contains(&i100) {
                Few
            } else {
                Many
            }
        }
        "cs" | "sk" => {
            if !int {
                Many
            } else if i == 1 {
                One
            } else if (2..=4).contains(&i) {
                Few
            } else {
                Other
            }
        }
        "hr" | "sr" | "bs" | "sh" => {
            if (int && i10 == 1 && i100 != 11) || (f10 == 1 && f100 != 11) {
                One
            } else if (int && (2..=4).contains(&i10) && !(12..=14).contains(&i100)) || ((2..=4).contains(&f10) && !(12..=14).contains(&f100)) {
                Few
            } else {
                Other
            }
        }
        "sl" => {
            if !int {
                Few
            } else if i100 == 1 {
                One
            } else if i100 == 2 {
                Two
            } else if (3..=4).contains(&i100) {
                Few
            } else {
                Other
            }
        }
        "ro" => {
            if int && i == 1 {
                One
            } else if !int || i == 0 || (1..=19).contains(&i100) {
                Few
            } else {
                Other
            }
        }
        "lt" => {
            if !int {
                Many
            } else if i10 == 1 && !(11..=19).contains(&i100) {
                One
            } else if i10 >= 2 && !(11..=19).contains(&i100) {
                Few
            } else {
                Other
            }
        }
        "lv" => {
            if int && (i10 == 0 || (11..=19).contains(&i100)) {
                Zero
            } else if int && i10 == 1 && i100 != 11 {
                One
            } else {
                Other
            }
        }
        "ar" => {
            if !int {
                Other
            } else if i == 0 {
                Zero
            } else if i == 1 {
                One
            } else if i == 2 {
                Two
            } else if (3..=10).contains(&i100) {
                Few
            } else if (11..=99).contains(&i100) {
                Many
            } else {
                Other
            }
        }
        "he" | "iw" => {
            if (int && i == 1) || (i == 0 && !int) {
                One
            } else if int && i == 2 {
                Two
            } else {
                Other
            }
        }
        // de, en, nl, sv, da, nb, fi, et, it, es, ca, el, hu, tr, bg, ... : one = 1 without fraction
        _ => {
            if int && i == 1 { One } else { Other }
        }
    }
}
