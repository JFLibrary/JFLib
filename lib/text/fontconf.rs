// fontconf -- generic families, substitutes and choices per language: a light fontconfig
//
//   FontConf::new()                  no rules
//   FontConf::system()               the rules of this system: on Linux the <alias> and
//                                    language rules of fontconfig's files (/etc/fonts/fonts.conf
//                                    and what it includes) -- or instead the file of
//                                    jfgui_FONTCONF (the same format, every OS; tests) --, then
//                                    the built-in rules of this OS
//   conf.parse(text, dir) / conf.load(path) -> bool   add the rules of a fontconfig file
//   conf.add_alias(family, prefer, accept, default)    add an alias
//   conf.add_lang(lang, family, mode, families)        add a language rule (LR_*)
//   conf.candidates(family, lang) -> Vec<String>       the families to try, best first
//   conf.lang_families(lang) -> Vec<String>            what the language puts into
//                                    sans-serif beyond what every language gets (fallback.rs
//                                    tries them first: Han characters in Japanese forms for ja)
//   conf.files() -> usize            fontconfig files read
//   is_generic(name)                 GENERICS: sans-serif, serif, monospace, cursive, fantasy,
//                                    system-ui, emoji, math (also fontconfig's sans and mono)
//
// FontDb::resolve(&conf, family, lang, weight, italic) (fontdb.rs) finds the first installed
// candidate; for a name nothing installed matches, the candidates of sans-serif.
//
// Rules, fontconfig's semantics in short: an alias for F puts its <prefer> families right
// before F (so a later alias's prefer list follows an earlier one), its <accept> families
// right after F (a later list before an earlier one), its <default> families at the end.  A
// <match target="pattern"> applies when its tests are only lang (eq / contains) and family
// (eq) and they hold; its family edits prepend (before F), prepend_first, append (after F),
// append_last or assign (in place of F).  Other matches (charset, spacing, prgname, target
// font: hinting) are skipped.  The families in the list that have rules of their own are
// expanded in place (4 levels).  Layers: the files' rules first, the built-in ones after them;
// names compare ASCII-case-insensitively, each once.  Languages: "ja" matches ja and ja-JP;
// zh-Hans / zh-CN / zh-SG / plain zh also match zh-cn, zh-Hant / zh-TW zh-tw, zh-HK / zh-MO
// zh-hk and zh-tw.  <include> follows files and folders (*.conf in name order) relative to
// the including file's folder, prefix="xdg" ($XDG_CONFIG_HOME, else ~/.config) and "~/"; at
// most FILES_MAX files of at most 1 MiB.

use alloc::string::String;
use alloc::vec::Vec;

/// add_lang modes (fontconfig's edit modes)
pub const LR_PREPEND: u8 = 0;
pub const LR_PREPEND_FIRST: u8 = 1;
pub const LR_APPEND: u8 = 2;
pub const LR_APPEND_LAST: u8 = 3;
pub const LR_ASSIGN: u8 = 4;

/// the generic families
pub const GENERICS: [&str; 10] = ["sans-serif", "serif", "monospace", "cursive", "fantasy", "system-ui", "emoji", "math", "sans", "mono"];

const FILES_MAX: usize = 256;
const FILE_MAX: usize = 1 << 20;
const DEPTH_MAX: u32 = 4;

/// a generic family name (GENERICS)
pub fn is_generic(name: &str) -> bool {
    GENERICS.iter().any(|g| g.eq_ignore_ascii_case(name.trim()))
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[derive(Clone, Debug)]
enum Rule {
    Alias { family: String, prefer: Vec<String>, accept: Vec<String>, default: Vec<String> },
    Lang { lang: Option<String>, family: Option<String>, mode: u8, families: Vec<String> },
}

impl Rule {
    fn family(&self) -> Option<&str> {
        match self {
            Rule::Alias { family, .. } => Some(family),
            Rule::Lang { family, .. } => family.as_deref(),
        }
    }
}

/// generic families, substitutes and language choices (see the module header)
#[derive(Clone, Debug, Default)]
pub struct FontConf {
    files: Vec<Rule>,
    builtin: Vec<Rule>,
    nfiles: usize,
}

impl FontConf {
    pub fn new() -> FontConf {
        FontConf::default()
    }

    /// the rules of this system (see the module header)
    pub fn system() -> FontConf {
        let mut c = FontConf::new();
        match plat::env("jfgui_FONTCONF") {
            Some(p) => {
                c.load(&p);
            }
            None => {
                #[cfg(target_os = "linux")]
                c.load("/etc/fonts/fonts.conf");
            }
        }
        c.builtin = builtin();
        c
    }

    /// fontconfig files read
    pub fn files(&self) -> usize {
        self.nfiles
    }

    /// add the rules of a fontconfig file (and of what it includes) -> it could be read
    pub fn load(&mut self, path: &str) -> bool {
        let mut seen = Vec::new();
        self.load_file(path, &mut seen)
    }

    fn load_file(&mut self, path: &str, seen: &mut Vec<String>) -> bool {
        let norm = path_norm(path);
        if seen.len() >= FILES_MAX || seen.iter().any(|s| *s == norm) {
            return false;
        }
        seen.push(norm);
        let Some(data) = plat::read_file(path) else { return false };
        if data.len() > FILE_MAX {
            return false;
        }
        let text = String::from_utf8_lossy(&data);
        let dir = path.rsplit_once(['/', '\\']).map_or("", |x| x.0);
        let root = parse_xml(&text);
        self.nfiles += 1;
        for e in &root.kids {
            self.element(e, dir, seen);
        }
        true
    }

    /// add the rules of fontconfig text (includes relative to dir) -> rules found
    pub fn parse(&mut self, text: &str, dir: &str) -> usize {
        let n = self.files.len();
        let root = parse_xml(text);
        let mut seen = Vec::new();
        for e in &root.kids {
            self.element(e, dir, &mut seen);
        }
        self.files.len() - n
    }

    /// add an alias: prefer before the family, accept after it, default at the end
    pub fn add_alias(&mut self, family: &str, prefer: &[&str], accept: &[&str], default: &[&str]) {
        let v = |l: &[&str]| l.iter().map(|s| String::from(*s)).collect();
        self.files.push(Rule::Alias { family: String::from(family), prefer: v(prefer), accept: v(accept), default: v(default) });
    }

    /// add a language rule (lang None: every language; family None: every family)
    pub fn add_lang(&mut self, lang: Option<&str>, family: Option<&str>, mode: u8, families: &[&str]) {
        self.files.push(Rule::Lang {
            lang: lang.map(lang_norm),
            family: family.map(String::from),
            mode,
            families: families.iter().map(|s| String::from(*s)).collect(),
        });
    }

    fn element(&mut self, e: &Elem, dir: &str, seen: &mut Vec<String>) {
        match e.name.as_str() {
            "fontconfig" => {
                for k in &e.kids {
                    self.element(k, dir, seen);
                }
            }
            "include" => self.include(e, dir, seen),
            "alias" => {
                let Some(family) = e.kid("family").map(|f| f.text.trim()).filter(|t| !t.is_empty()) else { return };
                let list = |name: &str| -> Vec<String> {
                    e.kids.iter().filter(|k| k.name == name).flat_map(|k| k.kids.iter()).filter(|f| f.name == "family")
                        .map(|f| String::from(f.text.trim())).filter(|t| !t.is_empty()).collect()
                };
                self.files.push(Rule::Alias { family: String::from(family), prefer: list("prefer"), accept: list("accept"), default: list("default") });
            }
            "match" => {
                if e.attr("target").is_some_and(|t| t != "pattern") {
                    return;
                }
                let (mut lang, mut family) = (None, None);
                for t in e.kids.iter().filter(|k| k.name == "test") {
                    let cmp = t.attr("compare").unwrap_or("eq");
                    let val = t.kid("string").map(|s| String::from(s.text.trim()));
                    match (t.attr("name"), cmp, val) {
                        (Some("lang"), "eq" | "contains", Some(v)) if lang.is_none() => lang = Some(lang_norm(&v)),
                        (Some("family"), "eq", Some(v)) if family.is_none() => family = Some(v),
                        _ => return, // a test this light reader cannot decide
                    }
                }
                for ed in e.kids.iter().filter(|k| k.name == "edit" && k.attr("name") == Some("family")) {
                    let mode = match ed.attr("mode").unwrap_or("assign") {
                        "prepend" => LR_PREPEND,
                        "prepend_first" => LR_PREPEND_FIRST,
                        "append" => LR_APPEND,
                        "append_last" => LR_APPEND_LAST,
                        "assign" | "assign_replace" => LR_ASSIGN,
                        _ => continue,
                    };
                    let families: Vec<String> =
                        ed.kids.iter().filter(|s| s.name == "string").map(|s| String::from(s.text.trim())).filter(|t| !t.is_empty()).collect();
                    // an assignment without a family test would replace every family: only with one
                    if !families.is_empty() && (mode != LR_ASSIGN || family.is_some()) {
                        self.files.push(Rule::Lang { lang: lang.clone(), family: family.clone(), mode, families });
                    }
                }
            }
            _ => {}
        }
    }

    fn include(&mut self, e: &Elem, dir: &str, seen: &mut Vec<String>) {
        let p = e.text.trim();
        if p.is_empty() {
            return;
        }
        let path = if e.attr("prefix") == Some("xdg") {
            match plat::env("XDG_CONFIG_HOME") {
                Some(x) if x.starts_with('/') => alloc::format!("{}/{}", x, p),
                _ => match plat::env("HOME") {
                    Some(h) => alloc::format!("{}/.config/{}", h, p),
                    None => return,
                },
            }
        } else if let Some(rest) = p.strip_prefix("~/") {
            match plat::env("HOME") {
                Some(h) => alloc::format!("{}/{}", h, rest),
                None => return,
            }
        } else if p.starts_with('/') || dir.is_empty() {
            String::from(p)
        } else {
            alloc::format!("{}/{}", dir, p)
        };
        if plat::file_exists(&path) == 2 {
            let mut names = Vec::new();
            plat::dir_list(&path, &mut |name, is_dir, _| {
                if !is_dir && name.ends_with(".conf") {
                    names.push(String::from(name));
                }
            });
            names.sort();
            for n in names {
                self.load_file(&alloc::format!("{}/{}", path, n), seen);
            }
        } else {
            self.load_file(&path, seen);
        }
    }

    /// the families to try for a name in a language, best first (each once; generic names
    /// stay in the list where they stand)
    pub fn candidates(&self, family: &str, lang: &str) -> Vec<String> {
        let langs = lang_keys(lang);
        let mut out: Vec<String> = Vec::new();
        for (k, layer) in [&self.files, &self.builtin].into_iter().enumerate() {
            // the first layer decides whether the name itself stays (assign removes it)
            let mut memo = Memo { done: Vec::new(), steps: 0 };
            for f in expand(layer, family, &langs, 0, &mut memo) {
                if !out.iter().any(|o| same(o, &f)) && (k == 0 || !same(&f, family)) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// the families a language adds to sans-serif (fallback.rs tries them first)
    pub fn lang_families(&self, lang: &str) -> Vec<String> {
        let all = self.candidates("sans-serif", "");
        let mut out: Vec<String> = self.candidates("sans-serif", lang).into_iter().filter(|f| !all.iter().any(|a| same(a, f))).collect();
        out.retain(|f| !is_generic(f));
        out
    }
}

// the list for one family in one layer (see the module header)
// expansions already done (name, depth) and the steps spent: a crafted config cannot blow up the recursion
struct Memo {
    done: Vec<(String, u32, Vec<String>)>,
    steps: u32,
}

const STEPS_MAX: u32 = 2000;

fn expand(rules: &[Rule], family: &str, langs: &[String], depth: u32, memo: &mut Memo) -> Vec<String> {
    let mut list: Vec<String> = alloc::vec![String::from(family)];
    let mut fpos = Some(0usize);
    for r in rules {
        let Some(p) = fpos else { break };
        if r.family().is_some_and(|f| !same(f, family)) {
            continue;
        }
        match r {
            Rule::Alias { prefer, accept, default, .. } => {
                insert(&mut list, p, prefer);
                let p = p + prefer.len();
                insert(&mut list, p + 1, accept);
                list.extend(default.iter().cloned());
                fpos = Some(p);
            }
            Rule::Lang { lang, mode, families, .. } => {
                if lang.as_ref().is_some_and(|l| !langs.iter().any(|k| k == l)) {
                    continue;
                }
                match *mode {
                    LR_PREPEND => {
                        insert(&mut list, p, families);
                        fpos = Some(p + families.len());
                    }
                    LR_PREPEND_FIRST => {
                        insert(&mut list, 0, families);
                        fpos = Some(p + families.len());
                    }
                    LR_APPEND => insert(&mut list, p + 1, families),
                    LR_APPEND_LAST => list.extend(families.iter().cloned()),
                    _ => {
                        list.remove(p);
                        insert(&mut list, p, families);
                        fpos = None; // the family is gone: later rules for it do not apply
                    }
                }
            }
        }
    }
    // the families of the list that have rules of their own, expanded in place
    let mut out: Vec<String> = Vec::new();
    for f in list {
        let own = depth < DEPTH_MAX && !same(&f, family) && rules.iter().any(|r| r.family().is_some_and(|x| same(x, &f)));
        let parts = if own && memo.steps < STEPS_MAX {
            let key = f.trim().to_ascii_lowercase();
            if let Some(d) = memo.done.iter().find(|d| d.1 == depth + 1 && d.0 == key) {
                d.2.clone()
            } else {
                memo.steps += 1;
                let r = expand(rules, &f, langs, depth + 1, memo);
                memo.done.push((key, depth + 1, r.clone()));
                r
            }
        } else {
            alloc::vec![f]
        };
        for x in parts {
            if !out.iter().any(|o| same(o, &x)) {
                out.push(x);
            }
        }
    }
    out
}

fn insert(list: &mut Vec<String>, at: usize, items: &[String]) {
    let at = at.min(list.len());
    for (k, it) in items.iter().enumerate() {
        list.insert(at + k, it.clone());
    }
}

// a path with '\' as '/' and its "." / ".." segments resolved (include cycle detection)
fn path_norm(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for s in p.split(['/', '\\']) {
        match s {
            "." => {}
            ".." if parts.last().is_some_and(|l| !l.is_empty() && *l != "..") => {
                parts.pop();
            }
            _ => parts.push(s),
        }
    }
    parts.join("/")
}

// fontconfig's language form: lower case, '-' (ja_JP -> ja-jp)
fn lang_norm(l: &str) -> String {
    l.trim().to_ascii_lowercase().replace('_', "-")
}

// the rule languages a request matches: itself, its prefixes, the Chinese forms
fn lang_keys(lang: &str) -> Vec<String> {
    let l = lang_norm(lang);
    let mut out: Vec<String> = Vec::new();
    if l.is_empty() {
        return out;
    }
    let mut cur = l.as_str();
    loop {
        out.push(String::from(cur));
        match cur.rfind('-') {
            Some(i) => cur = &cur[..i],
            None => break,
        }
    }
    if l == "zh" || l.starts_with("zh-hans") || l.starts_with("zh-cn") || l.starts_with("zh-sg") {
        out.push(String::from("zh-cn"));
    }
    if l.starts_with("zh-hk") || l.starts_with("zh-mo") || l.starts_with("zh-hant-hk") || l.starts_with("zh-hant-mo") {
        out.push(String::from("zh-hk"));
        out.push(String::from("zh-tw"));
    }
    if l.starts_with("zh-hant") || l.starts_with("zh-tw") {
        out.push(String::from("zh-tw"));
    }
    out
}

// ---- the built-in rules: every generic family's defaults, the languages' families

#[cfg(windows)]
const GEN: &[(&str, &[&str])] = &[
    ("sans-serif", &["Segoe UI", "Arial", "Tahoma", "Verdana", "Microsoft Sans Serif"]),
    ("serif", &["Times New Roman", "Cambria", "Georgia"]),
    ("monospace", &["Cascadia Mono", "Consolas", "Courier New", "Lucida Console"]),
    ("cursive", &["Segoe Script", "Comic Sans MS", "Segoe Print"]),
    ("fantasy", &["Impact", "Gabriola"]),
    ("system-ui", &["Segoe UI Variable Text", "Segoe UI"]),
    ("emoji", &["Segoe UI Emoji"]),
    ("math", &["Cambria Math"]),
    ("sans", &["sans-serif"]),
    ("mono", &["monospace"]),
    ("Helvetica", &["Arial"]),
    ("Times", &["Times New Roman"]),
    ("Courier", &["Courier New"]),
    ("Liberation Sans", &["Arial"]),
    ("Liberation Serif", &["Times New Roman"]),
    ("Liberation Mono", &["Courier New"]),
    ("DejaVu Sans", &["Segoe UI", "Verdana"]),
    ("DejaVu Serif", &["Cambria", "Georgia"]),
    ("DejaVu Sans Mono", &["Consolas", "Courier New"]),
];

#[cfg(windows)]
const LANGS: &[(&str, &str, &[&str])] = &[
    ("ja", "sans-serif", &["Yu Gothic UI", "Meiryo UI", "Yu Gothic", "Meiryo", "MS UI Gothic"]),
    ("ja", "serif", &["Yu Mincho", "MS Mincho"]),
    ("ja", "monospace", &["MS Gothic"]),
    ("zh-cn", "sans-serif", &["Microsoft YaHei UI", "Microsoft YaHei", "SimHei"]),
    ("zh-cn", "serif", &["SimSun"]),
    ("zh-cn", "monospace", &["NSimSun"]),
    ("zh-tw", "sans-serif", &["Microsoft JhengHei UI", "Microsoft JhengHei"]),
    ("zh-tw", "serif", &["PMingLiU", "MingLiU"]),
    ("zh-tw", "monospace", &["MingLiU"]),
    ("zh-hk", "serif", &["PMingLiU_HKSCS", "MingLiU_HKSCS"]),
    ("ko", "sans-serif", &["Malgun Gothic", "Gulim"]),
    ("ko", "serif", &["Batang"]),
    ("ko", "monospace", &["GulimChe"]),
    ("th", "sans-serif", &["Leelawadee UI", "Tahoma"]),
    ("lo", "sans-serif", &["Leelawadee UI"]),
    ("km", "sans-serif", &["Leelawadee UI", "Khmer UI"]),
    ("hi", "sans-serif", &["Nirmala UI"]),
    ("mr", "sans-serif", &["Nirmala UI"]),
    ("ne", "sans-serif", &["Nirmala UI"]),
    ("bn", "sans-serif", &["Nirmala UI"]),
    ("ta", "sans-serif", &["Nirmala UI"]),
    ("te", "sans-serif", &["Nirmala UI"]),
    ("gu", "sans-serif", &["Nirmala UI"]),
    ("pa", "sans-serif", &["Nirmala UI"]),
    ("am", "sans-serif", &["Ebrima"]),
    ("he", "sans-serif", &["Segoe UI", "Arial"]),
    ("ar", "sans-serif", &["Segoe UI", "Tahoma"]),
    ("fa", "sans-serif", &["Segoe UI", "Tahoma"]),
];

#[cfg(not(windows))]
const GEN: &[(&str, &[&str])] = &[
    ("sans-serif", &["DejaVu Sans", "Noto Sans", "Liberation Sans", "Ubuntu", "Cantarell", "FreeSans"]),
    ("serif", &["DejaVu Serif", "Noto Serif", "Liberation Serif", "FreeSerif"]),
    ("monospace", &["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono", "Ubuntu Mono", "FreeMono"]),
    ("cursive", &["URW Chancery L", "Z003"]),
    ("fantasy", &["Impact"]),
    ("system-ui", &["Cantarell", "Ubuntu", "Noto Sans", "DejaVu Sans"]),
    ("emoji", &["Noto Color Emoji", "Noto Emoji"]),
    ("math", &["Noto Sans Math", "DejaVu Math TeX Gyre", "STIX Two Math"]),
    ("sans", &["sans-serif"]),
    ("mono", &["monospace"]),
    ("Arial", &["Liberation Sans"]),
    ("Helvetica", &["Liberation Sans"]),
    ("Times New Roman", &["Liberation Serif"]),
    ("Times", &["Liberation Serif"]),
    ("Courier New", &["Liberation Mono"]),
    ("Courier", &["Liberation Mono"]),
    ("Segoe UI", &["Noto Sans", "DejaVu Sans"]),
    ("Consolas", &["DejaVu Sans Mono", "Liberation Mono"]),
];

#[cfg(not(windows))]
const LANGS: &[(&str, &str, &[&str])] = &[
    ("ja", "sans-serif", &["Noto Sans CJK JP", "IPAGothic", "TakaoGothic", "VL Gothic"]),
    ("ja", "serif", &["Noto Serif CJK JP", "IPAMincho", "TakaoMincho"]),
    ("ja", "monospace", &["Noto Sans Mono CJK JP", "IPAGothic"]),
    ("zh-cn", "sans-serif", &["Noto Sans CJK SC", "WenQuanYi Micro Hei", "Droid Sans Fallback"]),
    ("zh-cn", "serif", &["Noto Serif CJK SC", "AR PL UMing CN"]),
    ("zh-cn", "monospace", &["Noto Sans Mono CJK SC", "WenQuanYi Micro Hei Mono"]),
    ("zh-tw", "sans-serif", &["Noto Sans CJK TC", "WenQuanYi Micro Hei"]),
    ("zh-tw", "serif", &["Noto Serif CJK TC", "AR PL UMing TW"]),
    ("zh-tw", "monospace", &["Noto Sans Mono CJK TC"]),
    ("zh-hk", "sans-serif", &["Noto Sans CJK HK", "Noto Sans CJK TC"]),
    ("ko", "sans-serif", &["Noto Sans CJK KR", "NanumGothic", "UnDotum"]),
    ("ko", "serif", &["Noto Serif CJK KR", "NanumMyeongjo", "UnBatang"]),
    ("ko", "monospace", &["Noto Sans Mono CJK KR", "NanumGothicCoding"]),
    ("th", "sans-serif", &["Noto Sans Thai", "Loma", "Garuda"]),
    ("hi", "sans-serif", &["Noto Sans Devanagari", "Lohit Devanagari"]),
    ("mr", "sans-serif", &["Noto Sans Devanagari", "Lohit Marathi"]),
    ("bn", "sans-serif", &["Noto Sans Bengali", "Lohit Bengali"]),
    ("ta", "sans-serif", &["Noto Sans Tamil", "Lohit Tamil"]),
    ("he", "sans-serif", &["Noto Sans Hebrew", "DejaVu Sans"]),
    ("ar", "sans-serif", &["Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans"]),
    ("fa", "sans-serif", &["Noto Sans Arabic", "Vazirmatn", "DejaVu Sans"]),
    ("am", "sans-serif", &["Noto Sans Ethiopic", "Abyssinica SIL"]),
];

fn builtin() -> Vec<Rule> {
    let s = |l: &[&str]| -> Vec<String> { l.iter().map(|x| String::from(*x)).collect() };
    let mut out = Vec::new();
    // language families before the generic's defaults (prepend), so a CJK font comes first
    for (lang, fam, list) in LANGS {
        out.push(Rule::Lang { lang: Some(String::from(*lang)), family: Some(String::from(*fam)), mode: LR_PREPEND, families: s(list) });
    }
    for (fam, list) in GEN {
        out.push(Rule::Alias { family: String::from(*fam), prefer: Vec::new(), accept: Vec::new(), default: s(list) });
    }
    out
}

// ---- a tolerant reader of fontconfig's XML: elements, attributes, text; comments, the
// declaration, DOCTYPE and processing instructions are skipped

#[derive(Default, Debug)]
struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    kids: Vec<Elem>,
}

impl Elem {
    fn attr(&self, n: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == n).map(|a| a.1.as_str())
    }
    fn kid(&self, n: &str) -> Option<&Elem> {
        self.kids.iter().find(|k| k.name == n)
    }
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(j) = tail.find(';').filter(|&j| j <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=j]),
        }
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    out
}

// the document as a tree under a nameless root (unclosed elements end with the text)
fn parse_xml(s: &str) -> Elem {
    let mut stack: Vec<Elem> = alloc::vec![Elem::default()];
    let mut rest = s;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(rest));
            }
            break;
        };
        if lt > 0 {
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(&rest[..lt]));
            }
        }
        rest = &rest[lt..];
        // comments, declarations, processing instructions
        let skip_to = |r: &str, end: &str| r.find(end).map(|i| i + end.len()).unwrap_or(r.len());
        if rest.starts_with("<!--") {
            rest = &rest[skip_to(rest, "-->")..];
            continue;
        }
        if rest.starts_with("<?") {
            rest = &rest[skip_to(rest, "?>")..];
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").unwrap_or(rest.len());
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9.min(end)..end]);
            }
            rest = &rest[(end + 3).min(rest.len())..];
            continue;
        }
        if rest.starts_with("<!") {
            rest = &rest[skip_to(rest, ">")..];
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            // close up to the matching element (tolerating a missing end tag)
            if let Some(k) = stack.iter().rposition(|e| e.name == name).filter(|&k| k > 0) {
                while stack.len() > k {
                    let e = stack.pop().unwrap_or_default();
                    if let Some(parent) = stack.last_mut() {
                        parent.kids.push(e);
                    }
                }
            }
            continue;
        }
        let empty = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name_end = tag.find(|c: char| c.is_ascii_whitespace()).unwrap_or(tag.len());
        let mut e = Elem { name: String::from(&tag[..name_end]), ..Elem::default() };
        let mut a = &tag[name_end..];
        loop {
            a = a.trim_start();
            let Some(eq) = a.find('=') else { break };
            let key = a[..eq].trim();
            let v = a[eq + 1..].trim_start();
            let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
            let Some(close) = v[1..].find(q) else { break };
            e.attrs.push((String::from(key), unescape(&v[1..1 + close])));
            a = &v[close + 2..];
        }
        if empty {
            if let Some(top) = stack.last_mut() {
                top.kids.push(e);
            }
        } else if stack.len() < 64 {
            stack.push(e);
        }
    }
    while stack.len() > 1 {
        let e = stack.pop().unwrap_or_default();
        if let Some(parent) = stack.last_mut() {
            parent.kids.push(e);
        }
    }
    stack.pop().unwrap_or_default()
}
