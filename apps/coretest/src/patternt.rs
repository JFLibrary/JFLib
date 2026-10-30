// patternt -- checks of base::pattern: wildcards (stars with backtracking, sets, ranges,
// negation, classes, escapes, an open bracket), basic and extended regular expressions
// (anchors, repetitions and intervals, groups, alternatives, classes, word edges, case
// folding, GNU's lax cases), leftmost-longest matches and the errors in GNU grep's words

use alloc::vec::Vec;

use base::pattern::{glob, has_glob, ReErr, Regex, Syntax};

use crate::T;

fn bre(p: &str, s: &str) -> bool {
    Regex::new(p, Syntax::Basic, false).is_ok_and(|r| r.is_match(s))
}

fn ere(p: &str, s: &str) -> bool {
    Regex::new(p, Syntax::Extended, false).is_ok_and(|r| r.is_match(s))
}

fn find(p: &str, syn: Syntax, s: &str) -> Option<(usize, usize)> {
    Regex::new(p, syn, false).ok()?.find(s)
}

fn err(p: &str, syn: Syntax) -> Option<ReErr> {
    Regex::new(p, syn, false).err()
}

pub fn run(t: &mut T) {
    // wildcards
    let yes = [("*", ""), ("*", "abc"), ("a*", "a"), ("*.txt", "notiz.txt"), ("a*b*c", "aXbYc"), ("a*b*c", "abbbc"), ("?", "x"), ("??", "äö"), ("[abc]x", "bx"), ("[a-c]", "b"), ("[!a-c]", "d"), ("[^a-c]", "d"), ("[]]", "]"), ("[a-]", "-"), ("[[:digit:]]*", "7z"), ("\\*", "*"), ("[*]", "*"), ("x[", "x["), (".*", ".hidden"), ("*/*", "a/b")];
    for (p, s) in yes {
        t.check(&alloc::format!("glob {} ~ {}", p, s), glob(p, s));
    }
    let no = [("a", ""), ("*.txt", "notiz.txt.bak"), ("?", ""), ("?", "ab"), ("[abc]", "d"), ("[!a-c]", "b"), ("a*b*c", "aXbY"), ("\\*", "x"), ("[[:alpha:]]", "1"), ("*a", "bbb")];
    for (p, s) in no {
        t.check(&alloc::format!("glob {} !~ {}", p, s), !glob(p, s));
    }
    t.check("has_glob", has_glob("*.jfs") && has_glob("a?") && has_glob("[ab]") && !has_glob("plain") && !has_glob("\\*"));
    // a long star run backtracks without blowing up
    let long: alloc::string::String = core::iter::repeat_n('a', 60).collect();
    t.check("star backtracking", !glob("*a*a*a*a*a*b", &long) && glob("*a*a*a*a*a*", &long));

    // basic regular expressions
    t.check("bre literal", bre("ell", "hello") && !bre("elo", "hello"));
    t.check("bre dot", bre("h.llo", "hallo") && !bre("h.llo", "hllo"));
    t.check("bre anchors", bre("^he", "hello") && !bre("^el", "hello") && bre("lo$", "hello") && !bre("l$", "hello"));
    t.check("bre ^ and $ inside are characters", bre("a^b", "a^b") && bre("a$b", "a$b"));
    t.check("bre star", bre("ab*c", "ac") && bre("ab*c", "abbbc") && !bre("ab*c", "adc"));
    t.check("bre leading star is a character", bre("*a", "x*a") && !bre("*a", "xa"));
    t.check("bre + ? are characters", bre("a+", "a+") && !bre("a+", "aa") && bre("a?", "a?"));
    t.check("bre \\+ \\?", bre("ab\\+c", "abbc") && !bre("ab\\+c", "ac") && bre("ab\\?c", "ac") && bre("ab\\?c", "abc") && !bre("^ab\\?c$", "abbc"));
    t.check("bre interval", bre("^a\\{2,3\\}$", "aa") && bre("^a\\{2,3\\}$", "aaa") && !bre("^a\\{2,3\\}$", "aaaa") && bre("^a\\{2\\}$", "aa") && bre("^a\\{2,\\}$", "aaaaa"));
    t.check("bre group and alternative", bre("\\(ab\\)*c", "ababc") && bre("^\\(cat\\|dog\\)$", "dog") && !bre("^\\(cat\\|dog\\)$", "cow"));
    t.check("bre ( | are characters", bre("(a|b)", "(a|b)") && !bre("(a|b)", "a"));
    // extended
    t.check("ere + ? {}", ere("ab+c", "abbc") && !ere("ab+c", "ac") && ere("^ab?c$", "ac") && ere("^a{3}$", "aaa") && !ere("^a{3}$", "aa"));
    t.check("ere alternatives", ere("cat|dog", "hotdog") && ere("^(GET|POST) /", "POST /x") && !ere("^(GET|POST) /", "PUT /x"));
    t.check("ere nested groups", ere("^((ab)+|x)y$", "ababy") && ere("^((ab)+|x)y$", "xy") && !ere("^((ab)+|x)y$", "aby_"));
    t.check("ere { without interval is a character", ere("a{", "a{") && ere("a{x}", "a{x}"));
    t.check("ere {,n}", ere("^a{,2}$", "") && ere("^a{,2}$", "aa") && !ere("^a{,2}$", "aaa"));
    t.check("ere anchors anywhere", ere("^a|b$", "ab") && ere("x|^a", "ab") && !ere("x|^b", "ab"));
    t.check("ere empty-matching loops end", ere("^(a*)*$", "aaa") && ere("^(a|)*b$", "aab") && !ere("^(a*)*$", "aab"));
    // brackets and classes
    t.check("sets", ere("^[0-9]+$", "2026") && !ere("^[0-9]+$", "20x6") && ere("^[^ ]+$", "abc") && !ere("^[^ ]+$", "a c"));
    t.check("set with ] and -", ere("^[]a]+$", "]a]") && ere("^[a-]+$", "-a-") && ere("^[^]a]$", "b"));
    t.check("classes", ere("^[[:alpha:]][[:alnum:]_]*$", "eth0_x") && !ere("^[[:alpha:]]", "0x") && ere("[[:space:]]", "a b") && ere("^[[:upper:]]+$", "ABC") && ere("^[[:xdigit:]]+$", "0aF9"));
    t.check("\\w \\s", ere("^\\w+\\s\\w+$", "hallo welt") && !ere("^\\w+$", "a-b") && ere("^\\S+$", "a-b") && ere("\\W", "a-b"));
    t.check("word edges", ere("\\<eth", "dev eth0") && !ere("\\<th", "dev eth0") && ere("eth0\\>", "eth0 up") && ere("\\bup\\b", "is up now") && !ere("\\bup\\b", "upper") && ere("p\\B", "upper"));
    t.check("escaped operators", ere("a\\.b", "a.b") && !ere("a\\.b", "axb") && ere("\\(x\\)", "(x)") && bre("a\\.b", "a.b"));
    t.check("unicode", ere("^.{3}$", "äöü") && ere("ü+", "Grüüße"));
    // case
    let ri = Regex::new("hallo", Syntax::Basic, true);
    t.check("icase", ri.as_ref().is_ok_and(|r| r.is_match("HaLLo Welt") && !r.is_match("hello")));
    let rs = Regex::new("[a-c]x", Syntax::Extended, true);
    t.check("icase sets", rs.as_ref().is_ok_and(|r| r.is_match("BX") && !r.is_match("DX")));
    // leftmost, then longest
    t.eq("leftmost", find("b+", Syntax::Extended, "abbbcbb"), Some((1, 4)));
    t.eq("longest of the alternatives", find("a|ab|abc", Syntax::Extended, "xabcd"), Some((1, 4)));
    t.eq("empty match at the start", find("x*", Syntax::Extended, "abc"), Some((0, 0)));
    t.eq("end anchor", find("c$", Syntax::Extended, "abcabc"), Some((5, 6)));
    t.eq("byte offsets", find("ö+", Syntax::Extended, "aööb"), Some((1, 5)));
    t.eq("no match", find("z", Syntax::Basic, "abc"), None);
    let r = Regex::new("o", Syntax::Basic, false);
    let all: Vec<usize> = match &r {
        Ok(r) => {
            let mut v = Vec::new();
            let mut at = 0;
            while let Some((s, e)) = r.find_at("foo boo", at) {
                v.push(s);
                at = e.max(s + 1);
            }
            v
        }
        Err(_) => Vec::new(),
    };
    t.eq("find_at walks on", all, alloc::vec![1, 2, 5, 6]);
    // a pathological expression stays linear
    let hard = Regex::new("^(a|a)*b$", Syntax::Extended, false);
    let text: alloc::string::String = core::iter::repeat_n('a', 2000).collect();
    t.check("no exponential case", hard.as_ref().is_ok_and(|r| !r.is_match(&text)));
    // errors
    t.eq("open group", err("(ab", Syntax::Extended), Some(ReErr::Paren));
    t.eq("open bre group", err("\\(ab", Syntax::Basic), Some(ReErr::Paren));
    t.eq("close without open", err("ab)", Syntax::Extended), Some(ReErr::CloseParen));
    t.eq("open bracket", err("[ab", Syntax::Basic), Some(ReErr::Bracket));
    t.eq("range backwards", err("[z-a]", Syntax::Basic), Some(ReErr::Range));
    t.eq("unknown class", err("[[:nope:]]", Syntax::Basic), Some(ReErr::Class));
    t.eq("trailing backslash", err("ab\\", Syntax::Basic), Some(ReErr::Backslash));
    t.eq("open brace", err("a\\{2", Syntax::Basic), Some(ReErr::Brace));
    t.eq("interval backwards", err("a{3,2}", Syntax::Extended), Some(ReErr::BraceContent));
    t.eq("too big", err("a{200}{200}", Syntax::Extended), Some(ReErr::TooBig));
    t.eq("wording", (ReErr::Paren.text(), ReErr::Bracket.text(), ReErr::Backslash.text()), ("Unmatched ( or \\(", "Unmatched [, [^, [:, [., or [=", "Trailing backslash"));
}
