// imapt -- checks of netpkt::imap with its dissector (run by the group eap of coretest): lines with
// literals in pieces, commands and responses byte for byte, sequence sets, FETCH items, SEARCH keys,
// the ENVELOPE, dates and the parts of a message

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::imap::{self, Command, FetchItem, Item, Kind, Line, LineReader, Read, Response, SearchKey, Section, SeqSet, Status};
use netpkt::imf::Message;
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::{Ipv4, Mac};

use crate::T;

fn lines(b: &[u8]) -> Vec<Read> {
    let mut r = LineReader::new();
    r.push(b);
    let mut v = Vec::new();
    while let Some(x) = r.next() {
        v.push(x);
    }
    v
}

fn cmd(s: &str) -> Command {
    Command::parse(&Line::plain(s)).expect("command")
}

fn resp(s: &str) -> Response {
    Response::parse(&Line::plain(s)).expect("response")
}

/// a TCP segment with these bytes between a client and port 143
fn frame(payload: &[u8], to_server: bool) -> Vec<u8> {
    let (a, b) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]));
    let (src, dst, sport, dport) = if to_server { (a, b, 50000, imap::PORT_IMAP) } else { (b, a, imap::PORT_IMAP, 50000) };
    let mut p = Pkt::new(HEADROOM, payload);
    TcpHdr { src_port: sport, dst_port: dport, seq: 1, ack: 1, flags: ACK | PSH, window: 1000, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 2]), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_IPV4 }.frame(&p.into_vec())
}

pub fn run(t: &mut T) {
    // lines and literals
    let v = lines(b"a1 NOOP\r\nb2 LOGOUT\r\n");
    t.eq("two lines", v.len(), 2);
    let v = lines(b"a1 LOGIN {4}\r\nanna {6}\r\ngeheim\r\n");
    t.eq("two literals ask twice", v.iter().filter(|x| matches!(x, Read::NeedLiteral(_))).count(), 2);
    let Some(Read::Line(l)) = v.last() else {
        t.check("a line", false);
        return;
    };
    t.eq("the literals", l.lits.clone(), alloc::vec![b"anna".to_vec(), b"geheim".to_vec()]);
    let c = Command::parse(l).expect("login");
    t.eq("login user", c.arg(0), Some(String::from("anna")));
    t.eq("login pass", c.arg(1), Some(String::from("geheim")));
    // in pieces
    let mut r = LineReader::new();
    r.push(b"a1 APPE");
    t.eq("nothing yet", r.next(), None);
    r.push(b"ND Sent {5}\r\nhe");
    t.eq("needs it", r.next(), Some(Read::NeedLiteral(5)));
    t.eq("still waits", r.next(), None);
    r.push(b"llo\r\n");
    let Some(Read::Line(l)) = r.next() else {
        t.check("line in pieces", false);
        return;
    };
    t.eq("append's message", Command::parse(&l).map(|c| c.args), Some(alloc::vec![Item::atom("Sent"), Item::Literal(b"hello".to_vec())]));
    // a literal with + does not ask
    let v = lines(b"a LOGIN {1+}\r\nx y\r\n");
    t.eq("no question", v.len(), 1);
    t.check("a brace in the middle is no literal", matches!(&lines(b"a SEARCH {x}\r\n")[0], Read::Line(_)));
    // commands
    let c = cmd("A001 login anna \"ge\\\"heim\"");
    t.eq("tag", c.tag.as_str(), "A001");
    t.eq("verb in capitals", c.verb.as_str(), "LOGIN");
    t.eq("quoted", c.arg(1), Some(String::from("ge\"heim")));
    let c = cmd("a2 FETCH 1:* (FLAGS UID BODY.PEEK[HEADER.FIELDS (From Subject)])");
    t.eq("list", c.args.len(), 2);
    let Item::List(items) = &c.args[1] else {
        t.check("a list", false);
        return;
    };
    t.eq("a bracket keeps its spaces", items[2].clone(), Item::atom("BODY.PEEK[HEADER.FIELDS (From Subject)]"));
    t.eq("no verb", Command::parse(&Line::plain("a1")), None);
    t.eq("NIL", cmd("a X nil").args, alloc::vec![Item::Nil]);
    t.eq("line", imap::line("a1", "SELECT", "INBOX"), b"a1 SELECT INBOX\r\n".to_vec());
    t.eq("line bare", imap::line("a2", "NOOP", ""), b"a2 NOOP\r\n".to_vec());
    // responses
    let r = resp("a1 OK [READ-WRITE] SELECT done");
    t.eq("tagged", r.kind.clone(), Kind::Tagged(String::from("a1")));
    t.eq("status", r.status, Some(Status::Ok));
    t.eq("code", r.code.as_deref(), Some("READ-WRITE"));
    t.eq("text", r.text.as_str(), "SELECT done");
    let r = resp("* 5 EXISTS");
    t.eq("untagged", r.kind.clone(), Kind::Untagged);
    t.eq("name", r.name(), Some("EXISTS"));
    t.eq("number", r.number(), Some(5));
    let r = resp("* LIST (\\HasNoChildren) \"/\" INBOX");
    t.eq("list name", r.name(), Some("LIST"));
    t.eq("list values", r.items[3].clone(), Item::atom("INBOX"));
    t.eq("continuation", resp("+ Ready").kind, Kind::Cont);
    t.eq("BYE", resp("* BYE gone").status, Some(Status::Bye));
    // a FETCH answer with its literal
    let v = lines(b"* 1 FETCH (UID 7 BODY[] {5}\r\nhello)\r\n");
    let Some(Read::Line(l)) = v.last() else {
        t.check("a fetch line", false);
        return;
    };
    let r = Response::parse(l).expect("fetch");
    t.eq("fetch name", r.name(), Some("FETCH"));
    let Item::List(items) = &r.items[2] else {
        t.check("fetch list", false);
        return;
    };
    t.eq("fetch literal", items[3].clone(), Item::Literal(b"hello".to_vec()));
    // server lines
    t.eq("ok", imap::ok("a1", None, "done"), b"a1 OK done\r\n".to_vec());
    t.eq("ok with a code", imap::ok("a1", Some("READ-ONLY"), "done"), b"a1 OK [READ-ONLY] done\r\n".to_vec());
    t.eq("no", imap::no("a1", Some("NONEXISTENT"), "no"), b"a1 NO [NONEXISTENT] no\r\n".to_vec());
    t.eq("bad", imap::bad("a1", "what?"), b"a1 BAD what?\r\n".to_vec());
    t.eq("untagged", imap::untagged("OK hello"), b"* OK hello\r\n".to_vec());
    t.eq("cont", imap::cont("Ready"), b"+ Ready\r\n".to_vec());
    t.eq("astring atom", imap::astring("INBOX"), String::from("INBOX"));
    t.eq("astring quoted", imap::astring("my box"), String::from("\"my box\""));
    t.eq("astring escapes", imap::quoted("a\"b"), String::from("\"a\\\"b\""));
    t.eq("astring literal", imap::astring("Büro"), String::from("{5}\r\nBüro"));
    // sequence sets
    let s = SeqSet::parse("1:3,5,7:*").expect("set");
    t.check("in", s.contains(2, 9) && s.contains(5, 9) && s.contains(9, 9) && s.contains(8, 9));
    t.check("out", !s.contains(4, 9) && !s.contains(6, 9));
    t.eq("text", s.to_text(), String::from("1:3,5,7:*"));
    t.check("star alone", SeqSet::parse("*").expect("star").contains(4, 4));
    t.check("reversed", SeqSet::parse("5:2").expect("rev").contains(3, 9));
    t.eq("zero", SeqSet::parse("0"), None);
    t.eq("word", SeqSet::parse("abc"), None);
    // FETCH items
    t.eq("macro", imap::parse_fetch(&Item::atom("FAST")), Some(alloc::vec![FetchItem::Flags, FetchItem::InternalDate, FetchItem::Rfc822Size]));
    t.eq(
        "body",
        imap::parse_fetch(&Item::atom("body.peek[]")),
        Some(alloc::vec![FetchItem::Body { peek: true, section: Section::Full, name: String::from("BODY[]") }]),
    );
    t.eq(
        "fields",
        imap::parse_fetch(&Item::atom("BODY[HEADER.FIELDS (FROM SUBJECT)]")),
        Some(alloc::vec![FetchItem::Body { peek: false, section: Section::Fields(alloc::vec![String::from("FROM"), String::from("SUBJECT")], false), name: String::from("BODY[HEADER.FIELDS (FROM SUBJECT)]") }]),
    );
    t.eq("unknown", imap::parse_fetch(&Item::atom("BODYSTRUCTURE")), None);
    // SEARCH keys
    let a = imap::tokens(&Line::plain("UNSEEN FROM anna NOT DELETED OR FLAGGED SUBJECT \"Hallo Welt\" 2:4"));
    let k = imap::parse_search(&a).expect("search");
    t.eq("keys", k.len(), 5);
    t.eq("key from", k[1].clone(), SearchKey::From(String::from("anna")));
    t.check("key not", matches!(&k[2], SearchKey::Not(x) if **x == SearchKey::Deleted));
    t.check("key or", matches!(&k[3], SearchKey::Or(a, b) if **a == SearchKey::Flagged && **b == SearchKey::Subject(String::from("Hallo Welt"))));
    t.check("key seq", matches!(&k[4], SearchKey::Seq(_)));
    t.eq("key unknown", imap::parse_search(&imap::tokens(&Line::plain("WHATEVER"))), None);
    // dates
    t.eq("date", imap::internal_date(1_767_225_600), String::from("01-Jan-2026 00:00:00 +0000"));
    t.eq("date 2", imap::internal_date(1_767_225_600 + 86_400 * 59 + 3661), String::from("01-Mar-2026 01:01:01 +0000"));
    // the parts of a message and its ENVELOPE
    let data = b"From: Anna <anna@a.lab>\r\nTo: bob@b.lab\r\nSubject: Hallo\r\nDate: Thu, 01 Jan 2026 00:00:00 +0000\r\nMessage-ID: <1@a.lab>\r\nX-Long: a\r\n b\r\n\r\nText\r\nmehr\r\n";
    t.eq("text part", imap::text_of(data), b"Text\r\nmehr\r\n".as_slice());
    t.check("header part", imap::header_of(data).ends_with(b"\r\n\r\n"));
    t.eq("fields", imap::header_fields(data, &[String::from("subject")], false), b"Subject: Hallo\r\n\r\n".to_vec());
    let notx = imap::header_fields(data, &[String::from("X-Long")], true);
    t.check("fields.not", !notx.windows(6).any(|w| w == b"X-Long") && !notx.windows(3).any(|w| w == b" b\r"));
    let keep = imap::header_fields(data, &[String::from("x-long")], false);
    t.eq("with its continuation", keep, b"X-Long: a\r\n b\r\n\r\n".to_vec());
    t.eq(
        "envelope",
        imap::envelope(&Message::parse(data)),
        String::from("(\"Thu, 01 Jan 2026 00:00:00 +0000\" \"Hallo\" ((\"Anna\" NIL \"anna\" \"a.lab\")) ((\"Anna\" NIL \"anna\" \"a.lab\")) ((\"Anna\" NIL \"anna\" \"a.lab\")) ((NIL NIL \"bob\" \"b.lab\")) NIL NIL NIL \"<1@a.lab>\")"),
    );
    t.check("verbs", imap::verb_known("fetch") && imap::verb_known("UID") && !imap::verb_known("RETR"));
    // the dissector
    let f = frame(b"a1 LOGIN anna geheim\r\n", true);
    let (top, info) = decode::info(&f);
    t.eq("top", top, Proto::Imap);
    t.eq("request summary", info, String::from("a1 LOGIN anna geheim"));
    t.check("mail bit", decode::proto_mask(&f) & decode::P_MAIL != 0);
    let layers = decode::layers(&f);
    let l = layers.iter().find(|l| l.proto == Proto::Imap);
    t.check("tag field", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Tag" && x.value == "a1")));
    t.check("verb field", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Command" && x.value == "LOGIN")));
    let f = frame(b"* 3 EXISTS\r\na1 OK [READ-WRITE] done\r\n", false);
    let layers = decode::layers(&f);
    let l = layers.iter().find(|l| l.proto == Proto::Imap);
    t.check("untagged field", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Untagged" && x.value == "3 EXISTS")));
    t.check("status field", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Status" && x.value == "OK")));
    t.check("code field", l.is_some_and(|l| l.fields.iter().any(|x| x.name == "Response code" && x.value == "READ-WRITE")));
    t.eq("response summary", decode::info(&f).1, String::from("* 3 EXISTS"));
}
