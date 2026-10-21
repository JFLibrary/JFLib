// compare -- spelltest --compare AFF DIC WORDS [--sug N]: every line of the file WORDS
// checked with a dictionary from any path (a real one, to compare with Hunspell); prints
// "1<tab>word" / "0<tab>word" per line, for the first N wrong words also "<tab>sug|sug", and
// a summary line: load time, entries, the time of the checks and of the suggestions.
// spelltest --hyphenate DIC WORDS: every line of WORDS hyphenated with a hyphenation
// dictionary from any path (hyph_xx_YY.dic, or a TeX file .tex; "@TAG": the system's for a
// language, spell::find_hyphen): "word<tab>hy-phen-ated" per line and a summary line "# ..."
// (the file, load time, patterns, time), to compare with libhyphen.

use alloc::format;
use alloc::string::String;

use spell::{find_hyphen, Dictionary, Hyphenator, SRC_SYSTEM};

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// --hyphenate DIC WORDS -> exit code (2: a file could not be read)
pub fn hyphenate() -> i32 {
    let a = plat::args();
    let at = a.iter().position(|x| x == "--hyphenate").unwrap_or(0);
    let (Some(dic), Some(words)) = (a.get(at + 1), a.get(at + 2)) else {
        out("usage: spelltest --hyphenate DIC WORDS\n");
        return 2;
    };
    let t0 = plat::time_us();
    let path = match dic.strip_prefix('@') {
        Some(tag) => find_hyphen(tag, SRC_SYSTEM).map_or_else(String::new, |f| f.0),
        None => dic.clone(),
    };
    let (Some(h), Some(wb)) = (Hyphenator::load(&path), plat::read_file(words)) else {
        out("hyphenate: cannot read the files\n");
        return 2;
    };
    let t1 = plat::time_us();
    let text = String::from_utf8_lossy(&wb);
    let mut res = String::with_capacity(text.len() * 2);
    let mut n = 0usize;
    for w in text.lines() {
        let w = w.trim_end_matches('\r');
        if w.is_empty() {
            continue;
        }
        res.push_str(w);
        res.push('\t');
        res.push_str(&h.hyphenated(w, "-"));
        res.push('\n');
        n += 1;
    }
    let t2 = plat::time_us();
    let (p0, p1) = h.patterns();
    res.push_str(&format!("# {}: load {} ms, patterns {} + {}, minimums {:?}, {} words in {} ms\n", path, (t1 - t0) / 1000, p0, p1, h.min(), n, (t2 - t1) / 1000));
    out(&res);
    0
}

/// -> exit code (2: a file could not be read)
pub fn run() -> i32 {
    let a = plat::args();
    let at = a.iter().position(|x| x == "--compare").unwrap_or(0);
    let (Some(aff), Some(dic), Some(words)) = (a.get(at + 1), a.get(at + 2), a.get(at + 3)) else {
        out("usage: spelltest --compare AFF DIC WORDS [--sug N]\n");
        return 2;
    };
    let nsug: usize = plat::arg_value("--sug").and_then(|n| n.parse().ok()).unwrap_or(0);
    let (Some(ab), Some(db), Some(wb)) = (plat::read_file(aff), plat::read_file(dic), plat::read_file(words)) else {
        out("compare: cannot read the files\n");
        return 2;
    };
    let t0 = plat::time_us();
    let d = Dictionary::from_bytes(&ab, &db);
    let t1 = plat::time_us();
    drop((ab, db));
    let text = String::from_utf8_lossy(&wb);
    let mut res = String::with_capacity(text.len() * 2);
    let (mut n, mut tc, mut ts, mut tsmax, mut sugged) = (0usize, 0u64, 0u64, 0u64, 0usize);
    let (mut tcmax, mut slowest) = (0u64, "");
    for w in text.lines() {
        let w = w.trim_end_matches('\r');
        if w.is_empty() {
            continue;
        }
        n += 1;
        let c0 = plat::time_us();
        let ok = d.check(w);
        let dt = plat::time_us() - c0;
        tc += dt;
        if dt > tcmax {
            (tcmax, slowest) = (dt, w);
        }
        res.push_str(if ok { "1\t" } else { "0\t" });
        res.push_str(w);
        if !ok && sugged < nsug {
            let s0 = plat::time_us();
            let s = d.suggest(w);
            let dt = plat::time_us() - s0;
            ts += dt;
            tsmax = tsmax.max(dt);
            sugged += 1;
            res.push('\t');
            res.push_str(&s.join("|"));
        }
        res.push('\n');
    }
    out(&res);
    out(&format!(
        "# jflib: load {} ms, {} entries, unsupported {:?}; {} checks {} ms (slowest {} us: {}); {} suggestions {} ms (max {} ms)\n",
        (t1 - t0) / 1000,
        d.words(),
        d.unsupported(),
        n,
        tc / 1000,
        tcmax,
        slowest,
        sugged,
        ts / 1000,
        tsmax / 1000
    ));
    0
}
