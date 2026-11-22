// hostile -- spelltest --test hostile [--only NAME]: crafted dictionaries (what someone could
// put into config://hunspell) against the spell crate's bounds (docs/ROADMAP.md section 13,
// U11).  Each one is built here in memory: 20 000 empty suffixes that may follow each other,
// cross products of 3000 x 3000 empty affixes, 3000 prefixes and 20 000 suffixes on every root
// (the n-gram step's forms), 100 000 ICONV and REP entries, a TRY of 60 000 characters, a KEY
// row of 200 000 letters, MAP groups, 10 000 COMPOUNDRULEs and 100 000 CHECKCOMPOUNDPATTERNs,
// CHECKCOMPOUNDREP with 50 000 REP rules, 10 000 BREAK patterns, words whose hashes share one
// slot.  Their load, checks and suggestions are timed against limits far above what the bounds
// allow (before them: a check took up to 9 s, a suggestion 540 s), and a word in the list must
// still be taken.  Prints a line per operation; the exit code is the number over their limit.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use spell::Dictionary;

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// a word of letters from a number (a..z)
fn word(mut n: u64, len: usize) -> String {
    let mut s = String::new();
    for _ in 0..len {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

struct Run {
    over: u32,
    /// --only NAME: the dictionaries whose name starts so
    only: Option<String>,
}

impl Run {
    /// time f; over `limit_ms`: counted
    fn time<R>(&mut self, name: &str, what: &str, limit_ms: u64, f: impl FnOnce() -> R) -> R {
        let t0 = plat::time_us();
        let r = f();
        let ms = (plat::time_us() - t0) / 1000;
        let over = ms > limit_ms;
        self.over += over as u32;
        out(&format!("{} {:<22} {:<26} {:>7} ms (limit {})\n", if over { "OVER" } else { "ok  " }, name, what, ms, limit_ms));
        r
    }

    fn wanted(&self, name: &str) -> bool {
        self.only.as_deref().is_none_or(|o| name.starts_with(o))
    }

    /// load, check and suggest
    fn dict(&mut self, name: &str, aff: &str, dic: &str, words: &[&str]) {
        if !self.wanted(name) {
            return;
        }
        let d = self.time(name, &format!("load ({} KB)", (aff.len() + dic.len()) / 1024), 3000, || Dictionary::from_bytes(aff.as_bytes(), dic.as_bytes()));
        // the bounds take nothing from a word that is there
        if dic.lines().any(|l| l.split('/').next() == Some("haus")) && !d.check("haus") {
            self.over += 1;
            out(&format!("FAIL {:<22} \"haus\" is in the list and not taken\n", name));
        }
        for w in words {
            self.time(name, &format!("check {:.16}", w), 1000, || d.check(w));
        }
        for w in words {
            self.time(name, &format!("suggest {:.14}", w), 6000, || d.suggest(w));
        }
    }
}

// FNV-1a of dic.rs with its mixing
fn fnv(s: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in s {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h ^ (h >> 29)
}

/// `n` words whose hash falls into one slot of a table of 2^bits
fn colliding(n: usize, bits: u32) -> Vec<String> {
    let mask = (1u64 << bits) - 1;
    let mut v = Vec::new();
    let mut k = 0u64;
    while v.len() < n {
        let w = word(k, 7);
        if fnv(w.as_bytes()) & mask == 0 {
            v.push(w);
        }
        k += 1;
    }
    v
}

pub fn run() -> i32 {
    let mut r = Run { over: 0, only: plat::arg_value("--only").map(String::from) };
    let base = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwzxjq\n";
    let words = ["haus", "hausx", "xyzzyplugh", "abcdefghijklmnopqrstuvwxyzabcdefghijklmnop"];

    // 20 000 suffixes with nothing to strip or add, each its own continuation (two suffixes)
    let mut aff = String::from(base);
    let n = 20_000;
    aff.push_str(&format!("SFX A Y {}\n", n));
    for _ in 0..n {
        aff.push_str("SFX A 0 0/A .\n");
    }
    r.dict("sfx empty x2", &aff, "1\nhaus/A\n", &words);

    // a prefix and a suffix of 3000 empty rules each, cross products
    let mut aff = String::from(base);
    aff.push_str("PFX P Y 3000\n");
    for _ in 0..3000 {
        aff.push_str("PFX P 0 0 .\n");
    }
    aff.push_str("SFX S Y 3000\n");
    for _ in 0..3000 {
        aff.push_str("SFX S 0 0 .\n");
    }
    r.dict("cross empty", &aff, "1\nhaus/PS\n", &words);

    // 3000 prefixes and 20 000 suffixes that all apply to a root: its forms for the n-gram step
    let mut aff = String::from(base);
    aff.push_str("PFX P Y 3000\n");
    for i in 0..3000 {
        aff.push_str(&format!("PFX P 0 {} .\n", word(i, 3)));
    }
    aff.push_str("SFX S Y 20000\n");
    for i in 0..20_000 {
        aff.push_str(&format!("SFX S 0 {} .\n", word(i, 4)));
    }
    let mut dic = String::from("40\n");
    for i in 0..40 {
        dic.push_str(&format!("{}/PS\n", word(i * 7 + 3, 6)));
    }
    r.dict("forms cross", &aff, &dic, &words);

    // ICONV of 100 000 entries
    let mut aff = String::from(base);
    aff.push_str("ICONV 100000\n");
    for i in 0..100_000 {
        aff.push_str(&format!("ICONV {} x\n", word(i, 4)));
    }
    r.dict("iconv", &aff, "1\nhaus\n", &words);

    // REP of 100 000 entries that all match
    let mut aff = String::from(base);
    aff.push_str("REP 100000\n");
    for i in 0..100_000 {
        aff.push_str(&format!("REP a {}\n", word(i, 3)));
    }
    r.dict("rep", &aff, "1\nhaus\n", &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "hausa"]);

    // TRY of 60 000 characters
    let mut aff = String::from("SET UTF-8\nTRY ");
    for c in 0x4E00u32..0x4E00 + 60_000 {
        if let Some(ch) = char::from_u32(c) {
            aff.push(ch);
        }
    }
    aff.push('\n');
    r.dict("try", &aff, "1\nhaus\n", &words);

    // KEY of one row of 200 000 equal letters
    let mut aff = String::from(base);
    aff.push_str("KEY ");
    for _ in 0..200_000 {
        aff.push('a');
    }
    aff.push('\n');
    r.dict("key", &aff, "1\nhaus\n", &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);

    // MAP: one group of 5000 letters, and 5000 groups of two
    let mut aff = String::from(base);
    aff.push_str("MAP 5001\nMAP ");
    for i in 0..5000u32 {
        aff.push(char::from_u32(0x4E00 + i).unwrap_or('a'));
    }
    aff.push('\n');
    for i in 0..5000u32 {
        aff.push_str(&format!("MAP a{}\n", char::from_u32(0x6000 + i).unwrap_or('b')));
    }
    let wide: String = (0..40u32).map(|i| char::from_u32(0x4E00 + i).unwrap_or('a')).collect();
    r.dict("map", &aff, "1\nhaus\n", &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &wide]);

    // compounds: 10 000 COMPOUNDRULEs and 100 000 CHECKCOMPOUNDPATTERNs over short parts
    let mut aff = String::from(base);
    aff.push_str("COMPOUNDMIN 1\nCOMPOUNDFLAG C\nCOMPOUNDRULE 10000\n");
    for _ in 0..10_000 {
        aff.push_str("COMPOUNDRULE a*b*a*b*a*b*a*b*c\n");
    }
    aff.push_str("CHECKCOMPOUNDPATTERN 100000\n");
    for i in 0..100_000 {
        aff.push_str(&format!("CHECKCOMPOUNDPATTERN {} {}\n", word(i, 2), word(i + 7, 2)));
    }
    let mut dic = String::from("30\n");
    for i in 0..26 {
        dic.push_str(&format!("{}/abcC\n", word(i, 1)));
    }
    r.dict("compound", &aff, &dic, &["ababababababababababababababababababababababx", "abcabcabcabcabcabcabcabcabcabcabcabcabcabcz"]);

    // CHECKCOMPOUNDREP with 50 000 REP rules: every compound tries them
    let mut aff = String::from(base);
    aff.push_str("COMPOUNDMIN 1\nCOMPOUNDFLAG C\nCHECKCOMPOUNDREP\nREP 50000\n");
    for i in 0..50_000 {
        aff.push_str(&format!("REP {} {}\n", word(i % 26, 1), word(i, 3)));
    }
    let mut dic = String::from("30\n");
    for i in 0..26 {
        dic.push_str(&format!("{}/C\n", word(i, 1)));
    }
    r.dict("compound rep", &aff, &dic, &["abcdefghijklmnopqrstuvwxyzabcdefghijklmn", "zyxwvutsrqponmlkjihgfedcbaq"]);

    // BREAK of 10 000 patterns and a word full of them
    let mut aff = String::from(base);
    aff.push_str("BREAK 10000\n");
    for i in 0..10_000 {
        aff.push_str(&format!("BREAK {}\n", word(i, 2)));
    }
    r.dict("break", &aff, "1\nhaus\n", &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"]);

    // words whose hashes share one slot of the table (the count line sizes it: 2^11 .. 2^14
    // slots; words that collide on 14 bits do on fewer): it grew with the square of them
    let all = colliding(12_000, 14);
    for n in [1500usize, 3000, 6000, 12_000] {
        let mut dic = format!("{}\n", n * 2 / 3);
        for w in &all[..n] {
            dic.push_str(w);
            dic.push('\n');
        }
        let name = format!("collide {}", n);
        if !r.wanted(&name) {
            continue;
        }
        r.time(&name, "load", 3000, || Dictionary::from_bytes(base.as_bytes(), dic.as_bytes()));
    }
    out(&format!("hostile: {} operations over their limit\n", r.over));
    r.over as i32
}
