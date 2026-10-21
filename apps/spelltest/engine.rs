// engine -- spelltest --test engine: self-checks of the spell crate (lib/spell) on the test
// dictionaries of app://hunspell (de_TEST, en_TEST) and on small dictionaries written here:
// SET encodings, FLAG long / num / UTF-8, AF aliases, conditions, homonyms, NEEDAFFIX on an
// affix, compound options, COMPOUNDRULE, the hidden UPPER case forms of mixed case words,
// forbidden affixed forms, CIRCUMFIX, where compound flags come from, prefixes inside
// compounds, compounds of three parts, CHECKCOMPOUNDPATTERN with flags, CHECKCOMPOUNDREP, word
// pairs, IGNORE / ICONV, BREAK, the tokenizer, suggestions, the personal word list, finding
// dictionaries.  Every word expectation agrees with Hunspell 1.7
// (checked with libhunspell on the same dictionaries).  The group "hyphen": Liang's algorithm
// (spell::Hyphenator) on patterns written here and on app://hyphen/hyph_de_TEST.dic -- TeX
// files, libhyphen's .dic format (character sets, the minimums, NEXTLEVEL, NOHYPHEN,
// non-standard patterns), exceptions, edge cases; the expectations of the .dic tests agree
// with libhyphen 2.8 on the same files.  Prints FAIL lines and a summary; the exit code is the
// number of failures.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use spell::{find_hyphen, hyphen_languages, tokens, Dictionary, Hyphenator, Source, Speller, SRC_APP};

struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

impl T {
    fn check(&mut self, cond: bool, name: &str) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}\n", self.group, name));
        }
    }

    // every word correct / every word wrong
    fn good(&mut self, d: &Dictionary, words: &[&str]) {
        for w in words {
            self.check(d.check(w), &format!("{:?} is correct", w));
        }
    }

    fn bad(&mut self, d: &Dictionary, words: &[&str]) {
        for w in words {
            self.check(!d.check(w), &format!("{:?} is wrong", w));
        }
    }

    // the suggestions for `word` contain `want` (first: at position 0)
    fn sug(&mut self, d: &Dictionary, word: &str, want: &str, first: bool) {
        let s = d.suggest(word);
        let ok = if first { s.first().is_some_and(|x| x == want) } else { s.iter().any(|x| x == want) };
        self.check(ok, &format!("suggest({:?}) {} {:?} (got {:?})", word, if first { "starts with" } else { "has" }, want, s));
    }
}

fn de() -> Option<Dictionary> {
    Dictionary::load("app://hunspell/de_TEST.aff", "app://hunspell/de_TEST.dic")
}

// ------------------------------------------------------------------ de_TEST

fn affixes(t: &mut T) {
    let Some(d) = de() else {
        t.check(false, "app://hunspell/de_TEST loads");
        return;
    };
    t.check(d.words() >= 60, "de_TEST has its words");
    t.check(d.unsupported().is_empty(), "de_TEST uses nothing unsupported");
    t.good(&d, &["Haus", "Türen", "Aufgaben", "Schulen", "Tage", "Kinder", "Schiffe", "Arbeiten", "und"]);
    t.bad(&d, &["Haused", "Tüen", "Aufgabeen", "Kindere"]);
    // conditions: N adds "n" after e, "en" after anything else
    t.bad(&d, &["Türn", "Aufgabeen"]);
    // strip and condition: dunkel -> dunkle / dunklen / dunkler
    t.good(&d, &["dunkel", "dunkle", "dunklen", "dunkler"]);
    t.bad(&d, &["dunkelen", "dunkl"]);
    // cross product prefix + suffix
    t.good(&d, &["schön", "schöne", "unschön", "unschöne", "unschönem"]);
    // two suffixes: comparative + ending
    t.good(&d, &["schöner", "schönere", "schöneren"]);
    t.good(&d, &["schönerer", "guter", "besseren"]);
    t.bad(&d, &["schönenen", "schönerst", "guterer"]);
    // NEEDAFFIX stems
    t.bad(&d, &["lauf", "spiel"]);
    t.good(&d, &["laufen", "laufe", "spielst", "spielt"]);
    // CIRCUMFIX: ge- only with the participle ending
    t.good(&d, &["gelaufen", "gespielt"]);
    t.bad(&d, &["gelauf", "gespiel", "gelauft", "gespielen", "gelaufst"]);
    // FORBIDDENWORD (a compound that would be derived)
    t.bad(&d, &["Zimmerhaus"]);
    // KEEPCASE
    t.good(&d, &["JFLib"]);
    t.bad(&d, &["JFLIB", "jflib", "Jflib"]);
    // case variants
    t.good(&d, &["HAUS", "Haus", "Und", "UND", "PARIS", "Paris"]);
    t.bad(&d, &["haus", "paris", "hAUS"]);
    // CHECKSHARPS: SS in upper case words
    t.good(&d, &["Straße", "STRASSE", "STRAßE", "Straßen", "STRASSEN"]);
    t.bad(&d, &["Strasse", "strasse"]);
    // abbreviations (WORDCHARS ".") and numbers
    t.good(&d, &["z.B.", "usw.", "1.000,50", "12-15", "Haus."]);
    t.bad(&d, &["z.B", "usw"]);
    // BREAK (default: hyphens)
    t.good(&d, &["E-Mail", "E-Mail-Adresse", "Haus-", "-Tür"]);
    t.bad(&d, &["E-Mial-Adresse", "Haus-Tüür"]);
}

fn compounds(t: &mut T) {
    let Some(d) = de() else { return };
    t.good(&d, &["Haustür", "Gartenhaus", "Hausaufgabe", "Hausaufgaben", "Wörterbuch", "Rechtschreibung", "Schifffahrt", "Arbeitstag"]);
    // three parts, a Fugen-s in the middle
    t.good(&d, &["Arbeitszimmer", "Hausarbeitszimmer", "Prüfungsaufgabe", "Prüfungsaufgaben"]);
    // the Fugen-s is needed, alone it is no word
    t.bad(&d, &["Arbeitzimmer", "Prüfungaufgabe", "Arbeits", "Prüfungs"]);
    // nouns inside are lower case, the compound starts upper case
    t.bad(&d, &["HausTür", "haustür", "Haustüre", "tür", "haus"]);
    t.good(&d, &["HAUSTÜR"]);
    // parts shorter than COMPOUNDMIN, a part that is not a word
    t.bad(&d, &["Hausxy", "Hausxyztür", "Huastür"]);
}

fn suggestions(t: &mut T) {
    let Some(d) = de() else { return };
    t.sug(&d, "Hasu", "Haus", true); // swap
    t.sug(&d, "Strasse", "Straße", true); // REP ss -> ß
    t.sug(&d, "Hauser", "Häuser", false); // MAP a / ä
    t.sug(&d, "Tsg", "Tag", false); // KEY neighbour
    t.sug(&d, "Arbit", "Arbeit", true); // a missing letter (TRY)
    t.sug(&d, "Arbeiit", "Arbeit", true); // a letter too many
    t.sug(&d, "undoder", "und oder", false); // two words
    t.sug(&d, "haus", "Haus", true); // capitalised
    t.sug(&d, "HASU", "HAUS", true); // upper case
    t.sug(&d, "Gartenhuas", "Gartenhaus", false); // a compound
    t.sug(&d, "Haustüü", "Haustür", false);
    t.sug(&d, "Schreibunk", "Schreibung", false);
    // NOSUGGEST and forbidden words never come back
    let s = d.suggest("Misst");
    t.check(!s.iter().any(|x| x == "Mist"), &format!("suggest(\"Misst\") without the NOSUGGEST word (got {:?})", s));
    let s = d.suggest("Zimmerhuas");
    t.check(!s.iter().any(|x| x == "Zimmerhaus"), &format!("suggest(\"Zimmerhuas\") without the forbidden word (got {:?})", s));
    // every suggestion is a correct word, at most 8, the word itself never
    for w in ["Hasu", "Arbit", "Fehlr", "Schuel", "geleufen", "Wortebuch"] {
        let s = d.suggest(w);
        t.check(s.len() <= 8 && s.iter().all(|x| x.split(' ').all(|p| d.check(p))) && !s.iter().any(|x| x == w), &format!("suggest({:?}) gives correct words only ({:?})", w, s));
    }
    // the edit distance search: two edits away
    t.sug(&d, "Pürfnug", "Prüfung", false);
}

fn speller(t: &mut T) {
    let Some(d) = de() else { return };
    let mut sp = Speller::with_dictionary(d, "de-TEST");
    t.check(sp.language() == "de-TEST" && sp.backend() == "hunspell", "Speller: language and backend");
    let r0 = sp.revision();
    t.check(!sp.check("Klaus"), "Klaus is unknown");
    t.check(sp.cached("Klaus") == Some(false), "the result is cached");
    sp.add("Klaus");
    t.check(sp.revision() != r0 && sp.cached("Klaus").is_none(), "add: new revision, cache cleared");
    t.check(sp.check("Klaus") && sp.check("KLAUS") && !sp.check("klaus"), "added Klaus: Klaus, KLAUS, not klaus");
    sp.add("jflibx");
    t.check(sp.check("jflibx") && sp.check("Jflibx") && sp.check("JFLIBX"), "added in lower case: every case");
    sp.ignore("Qwz");
    t.check(sp.check("Qwz") && sp.personal() == ["Klaus", "jflibx"] && sp.ignored() == ["Qwz"], "ignore: accepted, not in the personal list");
    sp.remove("Klaus");
    t.check(!sp.check("Klaus"), "remove: Klaus unknown again");
    let s = sp.suggest("Jflibc", 5);
    t.check(s.iter().any(|x| x == "jflibx" || x == "Jflibx"), &format!("suggest offers personal words ({:?})", s));
    // misspelled() and check_token(): the abbreviation with its dot
    let text = "Heute ist z.B. die Hasu-Tür und usw. Arbit, 3D www.x.de";
    let bad: Vec<&str> = sp.misspelled(text).iter().map(|&(a, b)| &text[a..b]).collect();
    t.check(bad == ["Hasu", "Arbit"], &format!("misspelled() = {:?}", bad));
    let text = "Tüür-Haus-Tag E-Mial E-Mail-Adr.";
    let bad: Vec<&str> = sp.misspelled(text).iter().map(|&(a, b)| &text[a..b]).collect();
    t.check(bad == ["Tüür", "E-Mial", "Adr"], &format!("misspelled() of hyphenated words = {:?}", bad));
}

// ------------------------------------------------------------------ small dictionaries

fn formats(t: &mut T) {
    // ISO8859-1
    let d = Dictionary::from_bytes(b"SET ISO8859-1\nTRY \xe4\xf6\xfc\n", b"2\nT\xfcr\nSch\xfcler\n");
    t.good(&d, &["Tür", "Schüler"]);
    t.bad(&d, &["Tuer", "TÜRE"]);
    t.check(d.suggest("Tor").iter().any(|x| x == "Tür"), "ISO8859-1: TRY letters decoded");
    // ISO8859-2 and KOI8-R
    let d = Dictionary::from_bytes(b"SET ISO8859-2\n", b"1\n\xb3za\n");
    t.good(&d, &["łza", "Łza"]);
    let d = Dictionary::from_bytes(b"SET KOI8-R\n", b"1\n\xcd\xc9\xd2\n");
    t.good(&d, &["мир", "Мир", "МИР"]);
    // a UTF-8 file with a byte order mark
    let d = Dictionary::from_bytes(b"\xef\xbb\xbfSET UTF-8\n", b"\xef\xbb\xbf1\nk\xc3\xa4se\n");
    t.good(&d, &["käse"]);
    // FLAG long
    let d = Dictionary::from_bytes(b"FLAG long\nSFX Aa Y 1\nSFX Aa 0 s .\nSFX Ab Y 1\nSFX Ab 0 ed .\n", b"2\nword/Aa\nwalk/AaAb\n");
    t.good(&d, &["word", "words", "walks", "walked"]);
    t.bad(&d, &["worded"]);
    // FLAG num with a prefix (cross product)
    let d = Dictionary::from_bytes(b"FLAG num\nSFX 101 Y 1\nSFX 101 0 s .\nPFX 7 Y 1\nPFX 7 0 re .\n", b"1\nmake/101,7\n");
    t.good(&d, &["make", "makes", "remake", "remakes"]);
    t.bad(&d, &["remade", "rere"]);
    // FLAG UTF-8
    let d = Dictionary::from_bytes("SET UTF-8\nFLAG UTF-8\nSFX ü Y 1\nSFX ü 0 en .\n".as_bytes(), "1\nhaus/ü\n".as_bytes());
    t.good(&d, &["hausen"]);
    // AF aliases
    let d = Dictionary::from_bytes(b"AF 2\nAF AB\nAF A\nSFX A Y 1\nSFX A 0 s .\nSFX B Y 1\nSFX B 0 ed .\n", b"2\nwalk/1\ntalk/2\n");
    t.good(&d, &["walks", "walked", "talks"]);
    t.bad(&d, &["talked"]);
    // conditions with a set and a strip
    let d = Dictionary::from_bytes(b"SFX X Y 2\nSFX X y ies [^aeiou]y\nSFX X 0 s [aeiou]y\n", b"2\nfly/X\nboy/X\n");
    t.good(&d, &["flies", "boys"]);
    t.bad(&d, &["flys", "boies"]);
    // homonyms: both flag sets count
    let d = Dictionary::from_bytes(b"SFX A Y 1\nSFX A 0 en .\nSFX B Y 1\nSFX B 0 e .\n", b"2\nbank/A\nbank/B\n");
    t.good(&d, &["banken", "banke"]);
    // NEEDAFFIX on an affix: another affix must follow
    let d = Dictionary::from_bytes(b"NEEDAFFIX n\nSFX A Y 1\nSFX A 0 er/nB .\nSFX B Y 1\nSFX B 0 s .\n", b"1\nwalk/A\n");
    t.good(&d, &["walk", "walkers"]);
    t.bad(&d, &["walker"]);
    // morphological fields and an escaped slash
    let d = Dictionary::from_bytes(b"", b"3\nand/or\tpo:conj\nhome st:home po:noun\nhe\\/she\n");
    t.good(&d, &["home", "he/she"]);
    // IGNORE and ICONV
    let d = Dictionary::from_bytes("SET UTF-8\nIGNORE \u{AD}\nICONV 1\nICONV ’ '\n".as_bytes(), "2\ndon't\nword\n".as_bytes());
    t.good(&d, &["don’t", "don't", "wo\u{AD}rd"]);
    // BREAK of its own (the default hyphen no longer breaks)
    let d = Dictionary::from_bytes(b"BREAK 1\nBREAK +\n", b"2\nfoo\nbar\n");
    t.good(&d, &["foo+bar"]);
    t.bad(&d, &["foo-bar"]);
}

fn compound_options(t: &mut T) {
    let aff = b"COMPOUNDFLAG X\nCOMPOUNDMIN 2\nCHECKCOMPOUNDDUP\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE\nCHECKCOMPOUNDCASE\n\
COMPOUNDWORDMAX 3\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN oo ab\nFORCEUCASE U\nREP 1\nREP f g\nCHECKCOMPOUNDREP\n\
COMPOUNDFORBIDFLAG F\nSFX S Y 1\nSFX S 0 s/F .\nSFX P Y 1\nSFX P 0 x .\nONLYINCOMPOUND O\n";
    let dic = b"11\nfoo/XSP\nbar/X\nschiff/X\nfahrt/X\nBaz/X\nabc/X\nend/XU\nabcdeg\ndef/X\nmid/XO\nqq/X\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["foobar", "barfoo", "foobarfoo", "barabc", "schiffahrt", "Fooend", "foomidbar", "qqbar"]);
    t.bad(&d, &["foofoo", "foobarfoobar", "schifffahrt", "fooBaz", "fooabc", "fooend", "abcdef", "mid", "foosbar"]);
    // a suffix only at the end of a compound (without COMPOUNDPERMITFLAG)
    t.good(&d, &["barfoox"]);
    t.bad(&d, &["fooxbar"]);
}

// ------------------------------------------------------------------ Hunspell's rules
// (every expectation here agrees with Hunspell 1.7 on the same small dictionary)

fn compound_rules(t: &mut T) {
    // COMPOUNDRULE as en_US writes ordinal numbers: digits (n), then a part with 1 and one
    // with t (11th), or a part with m and one with p (21st); "1th" only inside compounds
    let aff = b"COMPOUNDMIN 1\nONLYINCOMPOUND c\nCOMPOUNDRULE 2\nCOMPOUNDRULE n*1t\nCOMPOUNDRULE n*mp\nWORDCHARS 0123456789\n";
    let dic = b"16\n0/nm\n0th/pt\n1/n1\n1st/p\n1th/tc\n2/nm\n2nd/p\n2th/tc\n3/nm\n3rd/p\n3th/tc\n4/nm\n4th/pt\n5/nm\n5th/pt\n9/nm\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.check(d.unsupported().is_empty(), "COMPOUNDRULE is supported");
    t.good(&d, &["1st", "2nd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "24th", "101st", "111th", "1000th", "95th"]);
    t.bad(&d, &["1th", "2st", "3th", "11st", "12nd", "13rd", "21th", "22th", "1stth"]);
    // the parts are dictionary words (the last one may have affixes); '*' and '?'
    let aff = b"COMPOUNDMIN 1\nCOMPOUNDRULE 2\nCOMPOUNDRULE AB*C\nCOMPOUNDRULE DE?F\nSFX S Y 1\nSFX S 0 s .\n";
    let dic = b"6\nfoo/A\nbar/B\nbaz/CS\nqux/D\nquux/E\nend/F\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["foobaz", "foobarbaz", "foobarbarbaz", "quxend", "quxquuxend", "bazs", "foobazs"]);
    t.bad(&d, &["foobar", "barbaz", "bazfoo", "foobazsbaz", "quxquuxquuxend", "quxquux"]);
    // FLAG long: the flags of a rule in parentheses
    let aff = b"FLAG long\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE (aa)(bb)*(cc)\n";
    let dic = b"3\nfoo/aa\nbar/bb\nbaz/cc\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["foobaz", "foobarbarbaz"]);
    t.bad(&d, &["foobar", "barbaz"]);
}

fn hunspell_rules(t: &mut T) {
    // mixed case words and UPPER ones with flags: their UPPER form counts, a Title form typed
    // does not (Hunspell's hidden ONLYUPCASE entries)
    let aff = b"SFX S Y 1\nSFX S 0 s .\n";
    let dic = b"4\nKdF\niPhone\nCIA/S\nNASA\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["KdF", "KDF", "iPhone", "IPHONE", "CIA", "CIAs", "CIAS", "NASA"]);
    t.bad(&d, &["Kdf", "kdf", "Iphone", "iphone", "Cia", "cia", "Cias", "Nasa", "nasa"]);
    t.check(!d.suggest("Kdx").iter().any(|x| x == "Kdf"), "a hidden Title entry is never suggested");
    // ... only when the Title form is no word of its own (before or after it in the list)
    let dic = b"4\nHf\nHF/S\nPC/S\nPc\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["Hf", "HF", "HFs", "PC", "PCs", "Pc"]);
    t.bad(&d, &["HFS", "hf", "PCS", "Pcs"]);
    // a forbidden root rejects its affixed forms, also as a compound (Foto + graphen)
    let aff = b"COMPOUNDFLAG X\nFORBIDDENWORD d\nSFX P Y 1\nSFX P 0 en .\n";
    let dic = b"4\nFoto/X\ngraph/XP\nFotograph/dP\nFotograf/P\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["Fotograf", "Fotografen", "Fotografen-Foto"]);
    t.bad(&d, &["Fotograph", "Fotographen", "FOTOGRAPHEN", "fotographen", "Fotograph-Foto"]);
    // CIRCUMFIX: a prefix and a suffix with it come together, a prefix alone may have it
    let aff = b"CIRCUMFIX X\nPFX A Y 1\nPFX A 0 leg/X .\nSFX B Y 1\nSFX B 0 obb/X .\nSFX C Y 1\nSFX C 0 ok .\n";
    let dic = b"1\nnagy/ABC\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["nagy", "legnagyobb", "legnagy", "nagyok"]);
    t.bad(&d, &["nagyobb", "legnagyok"]);
    // the prefix's continuation class may enable a suffix (cross product)
    let aff = b"PFX P Y 1\nPFX P 0 un/S .\nSFX S Y 1\nSFX S 0 able .\n";
    let dic = b"1\ndo/P\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["do", "undo", "undoable"]);
    t.bad(&d, &["doable"]);
    // CHECKSHARPS: "SS" of an UPPER word may be "ß", also for a KEEPCASE word
    let d = Dictionary::from_bytes("SET UTF-8\nCHECKSHARPS\nKEEPCASE k\n".as_bytes(), "2\nMaß/k\nStraße\n".as_bytes());
    t.good(&d, &["Maß", "MASS", "Straße", "STRASSE", "STRAßE"]);
    t.bad(&d, &["maß", "Mass", "Strasse", "straße"]);
}

fn compound_places(t: &mut T) {
    // the compound flags of a prefix count when it stands alone; with a suffix only those of
    // the suffix and the root do (a prefix on the last part needs COMPOUNDPERMITFLAG)
    let aff = "SET UTF-8\nCOMPOUNDBEGIN x\nCOMPOUNDEND z\nONLYINCOMPOUND o\nCOMPOUNDPERMITFLAG c\nPFX L Y 1\nPFX L T t/zoc T\nSFX N Y 1\nSFX N 0 en .\nSFX M Y 1\nSFX M 0 en/zo .\n";
    let dic = "3\nHaus/x\nTür/LN\nTor/LM\n";
    let d = Dictionary::from_bytes(aff.as_bytes(), dic.as_bytes());
    t.good(&d, &["Haustür", "Türen", "Haustoren", "Haustor"]);
    t.bad(&d, &["Haustüren", "tür", "toren"]);
    // prefixes on a first or middle part need no COMPOUNDPERMITFLAG, on the last one they do
    let aff = b"COMPOUNDBEGIN x\nCOMPOUNDMIDDLE y\nCOMPOUNDEND z\nCOMPOUNDPERMITFLAG c\nPFX U Y 1\nPFX U 0 un/y .\nPFX V Y 1\nPFX V 0 re/z .\nPFX W Y 1\nPFX W 0 de/zc .\n";
    let dic = b"4\nfoo/x\nbar/U\nbaz/z\nqux/VW\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["foobaz", "foounbarbaz", "unbar", "foodequx", "requx"]);
    t.bad(&d, &["foobarbaz", "foorequx"]);
    // a forbidden entry ends a part only with a compound flag of its place (nl "pagina-/Fw"
    // beside "pagina" + the suffix "-")
    let aff = b"FLAG long\nFORBIDDENWORD Fw\nCOMPOUNDBEGIN Ca\nCOMPOUNDEND Cc\nCOMPOUNDPERMITFLAG Cp\nSFX Ce Y 1\nSFX Ce 0 -/CaCp .\n";
    let dic = b"3\npagina/Ce\npagina-/Fw\ninstelling/Cc\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["pagina", "pagina-instelling"]);
    t.bad(&d, &["pagina-", "paginainstelling"]);
    // CHECKCOMPOUNDPATTERN with the flags of the parts' roots (two vowels meet: a hyphen is
    // needed), with a trailing comment; "0": a part without affixes
    let aff = b"COMPOUNDFLAG X\nCOMPOUNDMIN 2\nCOMPOUNDPERMITFLAG P\nCHECKCOMPOUNDPATTERN 3\nCHECKCOMPOUNDPATTERN e /E  # resolutie-eenheid\nCHECKCOMPOUNDPATTERN oo kk # a comment\nCHECKCOMPOUNDPATTERN 0/Z ab\nSFX S Y 1\nSFX S 0 s/P .\n";
    let dic = b"7\nresolutie/X\neenheid/XE\nbeeld/X\nboo/X\nkkab/X\nzeta/XZS\nabc/X\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["beeldeenheid", "resolutiebeeld", "zetasabc", "abczeta"]);
    t.bad(&d, &["resolutieeenheid", "bookkab", "zetaabc"]);
    // CHECKCOMPOUNDREP: no compound that a REP rule without ^ / $ turns into an entry of the
    // list (one only for compounds too); no compound the list has as two words
    let aff = b"COMPOUNDFLAG X\nCOMPOUNDMIN 2\nONLYINCOMPOUND O\nCHECKCOMPOUNDREP\nREP 2\nREP oo uu\nREP ^ba fa\n";
    let dic = b"9\nfoo/X\nbar/X\nfuubar/O\nbaz/X\nqux/X\nfazqux\nlu/X\nab/X\nlu ab\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["barfoo", "bazqux", "ablu", "fazqux"]);
    t.bad(&d, &["foobar", "luab", "fuubar"]);
    // ICONV: "0" is the digit (uk_UA makes Latin words numbers this way)
    let aff = b"ICONV 2\nICONV a 0\nICONV b 0\n";
    let dic = b"1\nxyz\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["xyz", "ab", "ab-ba", "bab"]);
    t.bad(&d, &["abc", "xyzab"]);
    // three parts or more: not when a simple correction is a word (two words here)
    let aff = b"COMPOUNDFLAG X\nCOMPOUNDMIN 2\n";
    let dic = b"4\nfahr/X\nrad/X\nweg/X\nfahrrad\n";
    let d = Dictionary::from_bytes(aff, dic);
    t.good(&d, &["radweg", "fahrweg", "radfahrweg", "fahrrad"]);
    t.bad(&d, &["fahrradweg", "wegfahrrad"]);
}

// ------------------------------------------------------------------ tokens, finding

fn tokenizer(t: &mut T) {
    let text = "Das ist 3D, siehe https://x.y/z und mail@x.de; z.B. E-Mail-Adresse. \"Zitat\" (don't) /usr/share C:\\x www.Beispiel.de - Ende-";
    let wc = ['.', 'ß', '-'];
    let toks = tokens(text, &wc);
    let words: Vec<(&str, bool)> = toks.iter().map(|k| (&text[k.start..k.end], k.dot)).collect();
    let want = [
        ("Das", false),
        ("ist", false),
        ("siehe", false),
        ("und", false),
        ("z.B", true),
        ("E-Mail-Adresse", true),
        ("Zitat", false),
        ("don't", false),
        ("Ende", false),
    ];
    t.check(words == want, &format!("tokens = {:?}", words));
    // without '.' in WORDCHARS the dot ends a word
    let toks = tokens("e.g. ok", &[]);
    t.check(toks.len() == 3 && toks[0].dot && toks[1].dot, "tokens without WORDCHARS dots");
    // letters of other scripts and combining marks
    let text = "שָׁלוֹם мир ñandú e\u{301}te";
    let n = tokens(text, &[]).len();
    t.check(n == 4, &format!("tokens of other scripts: {}", n));
}

fn finding(t: &mut T) {
    match spell::find("de-DE", SRC_APP) {
        Some(Source::Hunspell { aff, dic, lang, src }) => {
            t.check(aff == "app://hunspell/de_TEST.aff" && dic == "app://hunspell/de_TEST.dic" && lang == "de-TEST" && src == SRC_APP, &format!("find(de-DE): {} {} {}", aff, dic, lang))
        }
        other => t.check(false, &format!("find(de-DE) = {:?}", other)),
    }
    t.check(matches!(spell::find("en", SRC_APP), Some(Source::Hunspell { ref lang, .. }) if lang == "en-TEST"), "find(en)");
    t.check(spell::find("fr-FR", SRC_APP).is_none(), "find(fr-FR): none");
    t.check(spell::find("de-DE", 0).is_none(), "find without sources: none");
    let l = spell::languages(SRC_APP);
    t.check(l == ["de-TEST", "en-TEST"], &format!("languages(SRC_APP) = {:?}", l));
    t.check(spell::personal_path("de-DE") == "config://spell/de.dic", "personal_path");
    let mut sp = Speller::open("en-US", SRC_APP);
    t.check(sp.as_mut().is_some_and(|s| s.check("houses") && s.check("flies") && !s.check("Haus")), "Speller::open(en-US)");
    let s = sp.as_mut().map(|s| s.suggest("fone", 5)).unwrap_or_default();
    t.check(s.iter().any(|x| x == "phone"), &format!("en: REP f -> ph ({:?})", s));
}

// --only bench (not in the default run): a generated dictionary of the size of a real one
// (200 000 words with affix flags): load, checks, suggestions -- the times are printed
fn bench(t: &mut T) {
    const SYL: [&str; 24] = ["ba", "be", "bi", "da", "de", "do", "fa", "fe", "ga", "ge", "ha", "he", "ka", "ke", "la", "le", "ma", "me", "na", "ne", "ra", "re", "sa", "se"];
    let mut x: u32 = 12345;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut dic = String::from("200000\n");
    let mut sample: Vec<String> = Vec::new();
    for i in 0..200_000u32 {
        let n = 2 + next() % 4;
        let mut w = String::new();
        for _ in 0..n {
            w.push_str(SYL[(next() % 24) as usize]);
        }
        if i % 2000 == 0 {
            sample.push(w.clone());
        }
        dic.push_str(&w);
        dic.push_str(if i % 3 == 0 { "/NE\n" } else { "/A\n" });
    }
    let aff = "SET UTF-8\nTRY ebadfghklmnrs\nSFX N Y 2\nSFX N 0 n e\nSFX N 0 en [^e]\nSFX E Y 1\nSFX E 0 e .\nSFX A Y 3\nSFX A 0 er .\nSFX A 0 es .\nSFX A 0 em .\n";
    let t0 = plat::time_us();
    let d = Dictionary::from_bytes(aff.as_bytes(), dic.as_bytes());
    let t1 = plat::time_us();
    let mut ok = 0;
    for _ in 0..100 {
        for w in &sample {
            ok += d.check(w) as u32;
            ok += d.check(&format!("{}er", w)) as u32;
        }
    }
    let t2 = plat::time_us();
    let mut nsug = 0;
    for w in sample.iter().take(20) {
        let mut v: Vec<char> = w.chars().collect();
        v.swap(1, 2);
        v.push('x');
        nsug += d.suggest(&v.iter().collect::<String>()).len();
    }
    let t3 = plat::time_us();
    out(&format!(
        "  bench: {} entries loaded in {} ms; {} checks in {} ms ({} correct); 20 suggestions in {} ms ({} found)\n",
        d.words(),
        (t1 - t0) / 1000,
        sample.len() * 200,
        (t2 - t1) / 1000,
        ok,
        (t3 - t2) / 1000,
        nsug
    ));
    t.check(d.words() > 100_000, "bench: the generated dictionary loads");
}

/// run every group -> the number of failures
// hyphenation (spell::Hyphenator, lib/spell/hyphen.rs)
fn hyphen(t: &mut T) {
    let hy = |h: &Hyphenator, w: &str| h.hyphenated(w, "-");
    // Liang's example (The TeXbook, appendix H): hy-phen-ation
    let tex = "% a comment \\patterns{x9x}\n\\patterns{ hy3ph he2n % more\n hena4 hen5at 1na n2at\n1tio 2io o2n }\n\\hyphenation{ta-ble pro-ject}";
    let Some(mut h) = Hyphenator::from_tex(tex) else {
        t.check(false, "from_tex");
        return;
    };
    t.check(h.patterns().1 == 9, &format!("TeX: 9 patterns, the commented one skipped ({:?})", h.patterns()));
    t.check(hy(&h, "hyphenation") == "hy-phen-ation", &format!("hyphenation -> {}", hy(&h, "hyphenation")));
    t.check(hy(&h, "Hyphenation") == "Hy-phen-ation", "upper case letters match the patterns");
    t.check(h.hyphenate("hyphenation") == [2, 6], "byte offsets of the breaks");
    t.check(hy(&h, "table") == "ta-ble" && hy(&h, "TABLE") == "TA-BLE" && hy(&h, "project") == "pro-ject", "\\hyphenation exceptions, any case");
    h.set_min(3, 4);
    t.check(hy(&h, "hyphenation") == "hyphen-ation" && hy(&h, "table") == "table", "set_min(3, 4)");
    h.set_min(0, 0);
    t.check(h.min() == (2, 2), "the minimums never go below the dictionary's (2, 2)");
    for w in ["", "h", "hy", "na", "\u{1F600}", "123"] {
        t.check(h.hyphenate(w).is_empty(), &format!("{:?}: no break", w));
    }
    h.add_patterns("1ty");
    t.check(hy(&h, "hyphenation") == "hy-phen-ation" && hy(&h, "nanatyty") == "na-na-ty-ty", &format!("add_patterns ({})", hy(&h, "nanatyty")));
    // digits are word boundaries ('.'): ".na" never matches inside "3na"
    let d = Hyphenator::from_dic(b"UTF-8\n.na1na\n").unwrap();
    t.check(hy(&d, "nanana") == "na-nana" && hy(&d, "3nanana") == "3na-nana", &format!("digits as '.' ({})", hy(&d, "3nanana")));
    // .dic: UTF-8, comments, a line with a leading blank (no pattern), the minimums (byte
    // offsets after two-byte letters)
    let d = Hyphenator::from_dic("UTF-8\n% c\n# c\n a1b\nLEFTHYPHENMIN 1\nRIGHTHYPHENMIN 3\nö1r\nr1t\n".as_bytes()).unwrap();
    t.check(d.patterns().1 == 2 && d.min() == (1, 3), &format!("UTF-8 .dic: 2 patterns, minimums (1, 3) ({:?} {:?})", d.patterns(), d.min()));
    t.check(hy(&d, "Wörter") == "Wö-r-ter" && d.hyphenate("Wörter") == [3, 4], &format!("UTF-8 .dic: Wö-r-ter ({})", hy(&d, "Wörter")));
    t.check(hy(&d, "wört") == "wört" && hy(&d, "abab") == "abab", "RIGHTHYPHENMIN 3; the pattern after a blank is not read");
    // 8-bit character sets
    let d = Hyphenator::from_dic(b"ISO8859-2\n\xb31k\n").unwrap();
    t.check(hy(&d, "piłka") == "pił-ka" && hy(&d, "PIŁKA") == "PIŁ-KA", &format!("ISO8859-2: pił-ka ({})", hy(&d, "piłka")));
    let d = Hyphenator::from_dic(b"KOI8-R\n\xc11\xcd\n").unwrap();
    t.check(hy(&d, "мама") == "ма-ма", &format!("KOI8-R: ма-ма ({})", hy(&d, "мама")));
    let d = Hyphenator::from_dic(b"ISO8859-1\r\nb\xfc1c\r\n").unwrap();
    t.check(hy(&d, "Bücher") == "Bü-cher", &format!("ISO8859-1 with CR LF: Bü-cher ({})", hy(&d, "Bücher")));
    // non-standard patterns (libhyphen's Zuk-ker) never break; a standard 1k next to one does not either
    let d = Hyphenator::from_dic(b"UTF-8\n1k\nc1k/k=k,1,1\n").unwrap();
    t.check(hy(&d, "zucker") == "zucker" && hy(&d, "zuker") == "zu-ker", &format!("non-standard pattern ({} / {})", hy(&d, "zucker"), hy(&d, "zuker")));
    // the default first level: parts at the apostrophe, never a break next to it
    let d = Hyphenator::from_dic(b"UTF-8\nm1m\n1l\n").unwrap();
    t.check(hy(&d, "l'homme") == "l'hom-me" && hy(&d, "homme") == "hom-me", &format!("apostrophe ({})", hy(&d, "l'homme")));
    // NEXTLEVEL: the first level finds the parts, COMPOUNDLEFTHYPHENMIN keeps a break away
    // from a part's start
    let two = "UTF-8\nhaus1\nNEXTLEVEL\n1k\n1r\n";
    let d = Hyphenator::from_dic(two.as_bytes()).unwrap();
    t.check(hy(&d, "haustürkante") == "haus-tü-r-kante", &format!("NEXTLEVEL ({})", hy(&d, "haustürkante")));
    let d = Hyphenator::from_dic(format!("UTF-8\nCOMPOUNDLEFTHYPHENMIN 3\n{}", &two[6..]).as_bytes()).unwrap();
    t.check(hy(&d, "haustürkante") == "haus-tür-kante", &format!("COMPOUNDLEFTHYPHENMIN 3 ({})", hy(&d, "haustürkante")));
    let d = Hyphenator::from_dic("UTF-8\nNOHYPHEN ü\nhaus1\nNEXTLEVEL\n1k\n1r\n".as_bytes()).unwrap();
    t.check(hy(&d, "haustürkante") == "haus-tür-kante", &format!("NOHYPHEN ü ({})", hy(&d, "haustürkante")));
    t.check(Hyphenator::from_dic(b"UTF-8\n% nothing\n").is_none() && Hyphenator::from_tex("\\patterns{}").is_none(), "no patterns: None");
    // libhyphen's automaton (.dic): only the longest pattern beginning counts ("bcde" hides
    // "c1d"); TeX: every pattern
    let d = Hyphenator::from_dic(b"UTF-8\nbcde\nc1d\n").unwrap();
    let x = Hyphenator::from_tex("\\patterns{bcde c1d}").unwrap();
    t.check(hy(&d, "xbcdex") == "xbcdex" && hy(&x, "xbcdex") == "xbc-dex", &format!("automaton / Liang ({} / {})", hy(&d, "xbcdex"), hy(&x, "xbcdex")));
    // libhyphen: every part goes through the first level again ("ge1ne" hidden in the whole
    // word by "degeneration"), parts without parts through the second level
    let d = Hyphenator::from_dic(b"UTF-8\nde1generation\nge1ne\nNEXTLEVEL\nne1ra\n").unwrap();
    t.check(hy(&d, "degeneration") == "de-ge-ne-ration", &format!("parts of parts ({})", hy(&d, "degeneration")));
    let d = Hyphenator::from_dic(b"UTF-8\nab1\nbcde\nc1d\nNEXTLEVEL\nzz\n").unwrap();
    t.check(hy(&d, "abcde") == "ab-c-de" && hy(&d, "bcde") == "bcde", &format!("the first level on a part ({})", hy(&d, "abcde")));
    // a non-standard pattern inhibits only the places around its replaced letters
    let d = Hyphenator::from_dic(b"UTF-8\n1fe\nfes5szi1o/sz=,3,1\n").unwrap();
    t.check(hy(&d, "professzionista") == "pro-fesszi-onista", &format!("non-standard with index ({})", hy(&d, "professzionista")));
    // the test dictionary of app://hyphen (ISO8859-1, two levels, written for these tests)
    let found = find_hyphen("de-DE", SRC_APP);
    t.check(found == Some((String::from("app://hyphen/hyph_de_TEST.dic"), String::from("de-TEST"))), &format!("find_hyphen(de-DE) {:?}", found));
    t.check(find_hyphen("fr", SRC_APP).is_none() && hyphen_languages(SRC_APP) == ["de-TEST"], "no French; the languages of app://");
    let Some(de) = Hyphenator::load("app://hyphen/hyph_de_TEST.dic") else {
        t.check(false, "load hyph_de_TEST.dic");
        return;
    };
    for (w, want) in [
        ("Donaudampfschifffahrtsgesellschaftskapitän", "Do-nau-dampf-schiff-fahrts-ge-sell-schafts-ka-pi-tän"),
        ("Silbentrennung", "Sil-ben-tren-nung"),
        ("Wörterbücher", "Wör-ter-bü-cher"),
        ("Rechtschreibung", "Recht-schrei-bung"),
        ("lesbar", "les-bar"),
        ("Ab", "Ab"),
    ] {
        t.check(hy(&de, w) == want, &format!("de_TEST: {} -> {} (want {})", w, hy(&de, w), want));
    }
}

pub fn run() -> i32 {
    let only = plat::arg_value("--only").map(String::from);
    let groups: &[(&str, fn(&mut T))] = &[
        ("affixes", affixes),
        ("compounds", compounds),
        ("suggest", suggestions),
        ("speller", speller),
        ("formats", formats),
        ("compound-options", compound_options),
        ("compound-rules", compound_rules),
        ("hunspell-rules", hunspell_rules),
        ("compound-places", compound_places),
        ("tokens", tokenizer),
        ("find", finding),
        ("hyphen", hyphen),
        ("bench", bench),
    ];
    let (mut failed, mut total) = (0, 0);
    for (name, f) in groups {
        if only.as_deref().map_or(*name == "bench", |o| o != *name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0 };
        let t0 = plat::time_us();
        f(&mut t);
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<17} {} ok, {} failed ({} ms)\n", name, t.ok, t.failed, ms));
        failed += t.failed;
        total += t.ok + t.failed;
    }
    out(&format!("spelltest engine: {} of {} checks passed\n", total - failed, total));
    failed as i32
}
