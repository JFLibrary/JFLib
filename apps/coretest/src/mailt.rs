// mailt -- checks of netpkt::smtp, netpkt::pop3 and netpkt::imf and their dissection: SMTP
// commands and paths with parameters, replies on one and several lines with enhanced codes, the
// reader in pieces, EHLO keywords, the server's stream through DATA with dot-stuffing both ways
// and pipelined commands, AUTH PLAIN; POP3 commands, multi-line responses with their dots, the
// reader with the client's expectations; messages of RFC 5322 (folding and unfolding, fields,
// addresses with display names and groups, the Date form both ways, encoded words,
// quoted-printable, base64, a multipart report, a draft as 7bit, 8bit and quoted-printable) and
// segments of both protocols with their layers and summaries.

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::imf::{self, Draft, Mailbox, Message};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::pop3;
use netpkt::smtp::{self, Command, Ehlo, Reply, ReplyReader, Stream};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::{Ipv4, Mac};

use crate::T;

fn s(x: &str) -> String {
    String::from(x)
}

/// a segment with data from 10.0.0.5:sport to 10.0.0.9:dport
fn segment(sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 9]));
    let mut p = Pkt::new(HEADROOM, data);
    TcpHdr { src_port: sport, dst_port: dport, seq: 1000, ack: 2000, flags: ACK | PSH, window: 64240, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 9]), src: Mac([2, 0, 0, 0, 0, 5]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

/// the fields of a frame's layer of protocol `p` as (name, value)
fn fields(frame: &[u8], p: Proto) -> Vec<(&'static str, String)> {
    decode::layers(frame).iter().find(|l| l.proto == p).map(|l| l.fields.iter().map(|f| (f.name, f.value.clone())).collect()).unwrap_or_default()
}

pub fn run(t: &mut T) {
    smtp_commands(t);
    smtp_replies(t);
    smtp_stream(t);
    pop3_t(t);
    imf_header(t);
    imf_addresses(t);
    imf_dates(t);
    imf_encodings(t);
    imf_bodies(t);
    segments(t);
}

fn smtp_commands(t: &mut T) {
    t.eq("parse", Command::parse("ehlo pc1.a.lab\r\n"), Some(Command { verb: s("EHLO"), arg: s("pc1.a.lab") }));
    t.eq("line", Command::new("rcpt", "TO:<bob@b.lab>").to_line(), s("RCPT TO:<bob@b.lab>\r\n"));
    t.eq("no argument", Command::new("DATA", "").to_line(), s("DATA\r\n"));
    t.check("known verbs", smtp::verb_known("ehlo") && smtp::verb_known("STARTTLS") && !smtp::verb_known("USER"));
    let p = smtp::parse_path("FROM:<anna@a.lab> SIZE=345 BODY=8BITMIME", "FROM");
    t.eq("mail from", p.as_ref().map(|p| p.addr.clone()), Some(s("anna@a.lab")));
    t.eq("size", p.as_ref().and_then(|p| p.size()), Some(345));
    t.eq("body", p.as_ref().and_then(|p| p.param("body").map(String::from)), Some(s("8BITMIME")));
    t.eq("null reverse path", smtp::parse_path("FROM:<>", "FROM").map(|p| p.addr), Some(String::new()));
    t.eq("space after the colon", smtp::parse_path("TO: <bob@b.lab>", "TO").map(|p| p.addr), Some(s("bob@b.lab")));
    t.eq("source route", smtp::parse_path("TO:<@x.lab,@y.lab:bob@b.lab>", "TO").map(|p| p.addr), Some(s("bob@b.lab")));
    t.eq("no brackets", smtp::parse_path("FROM:anna@a.lab", "FROM"), None);
    t.eq("wrong keyword", smtp::parse_path("TO:<a@b>", "FROM"), None);
    t.eq("format", smtp::fmt_path("FROM", "anna@a.lab", &[("SIZE", "345"), ("BODY", "8BITMIME")]), s("FROM:<anna@a.lab> SIZE=345 BODY=8BITMIME"));
    t.eq("auth plain", smtp::auth_plain("anna", "geheim"), s("AGFubmEAZ2VoZWlt"));
    t.eq("auth plain back", smtp::plain_of("AGFubmEAZ2VoZWlt"), Some((s("anna"), s("geheim"))));
    t.eq("login challenges", (imf::b64_decode(smtp::LOGIN_USER.as_bytes()), imf::b64_decode(smtp::LOGIN_PASS.as_bytes())), (Some(b"Username:".to_vec()), Some(b"Password:".to_vec())));
    let r = smtp::received("pc1", "pc1.a.lab [10.0.1.10]", "mail.a.lab", "ESMTPA", "4F2A1B", Some("bob@b.lab"), "Thu, 01 Jan 2026 00:00:05 +0000");
    t.eq("received", r, s("from pc1 (pc1.a.lab [10.0.1.10]) by mail.a.lab with ESMTPA id 4F2A1B for <bob@b.lab>; Thu, 01 Jan 2026 00:00:05 +0000"));
}

fn smtp_replies(t: &mut T) {
    t.eq("one line", Reply::new(220, "mail.a.lab ready").to_bytes(), b"220 mail.a.lab ready\r\n".to_vec());
    let e = Ehlo { domain: s("mail.a.lab"), keywords: alloc::vec![s("SIZE 10485760"), s("8BITMIME"), s("PIPELINING"), s("AUTH PLAIN LOGIN"), s("ENHANCEDSTATUSCODES")] };
    let r = e.to_reply("greets pc1");
    t.eq("multi-line", r.to_bytes(), b"250-mail.a.lab greets pc1\r\n250-SIZE 10485760\r\n250-8BITMIME\r\n250-PIPELINING\r\n250-AUTH PLAIN LOGIN\r\n250 ENHANCEDSTATUSCODES\r\n".to_vec());
    let back = Ehlo::of_reply(&r);
    t.eq("ehlo domain", back.domain.as_str(), "mail.a.lab");
    t.check("keywords", back.has("pipelining") && back.has("8BITMIME") && !back.has("STARTTLS"));
    t.eq("size limit", back.size_limit(), Some(10_485_760));
    t.check("auth mechanisms", back.auth("login") && back.auth("PLAIN") && !back.auth("CRAM-MD5"));
    t.eq("no size", Ehlo::default().size_limit(), None);
    let enh = Reply::enh(550, "5.1.1", "<x@b.lab>: no such user here");
    t.eq("enhanced", enh.to_bytes(), b"550 5.1.1 <x@b.lab>: no such user here\r\n".to_vec());
    t.eq("status", enh.status(), Some("5.1.1"));
    t.eq("no status", Reply::new(250, "Fine").status(), None);
    t.check("classes", enh.is_permanent() && Reply::new(354, "").is_intermediate() && Reply::new(451, "").is_transient() && Reply::new(250, "").is_positive());
    let mut rr = ReplyReader::new();
    t.eq("in pieces", rr.push(b"250-mail.a.lab hi\r\n250-SI"), Vec::<Reply>::new());
    let got = rr.push(b"ZE 100\r\n250 8BITMIME\r\n354 Go\r\n");
    t.eq("two replies", got.iter().map(|r| (r.code, r.lines.len())).collect::<Vec<_>>(), alloc::vec![(250, 3), (354, 1)]);
    t.eq("their lines", got.first().map(|r| r.lines.clone()), Some(alloc::vec![s("mail.a.lab hi"), s("SIZE 100"), s("8BITMIME")]));
    t.eq("a lone LF and no text", rr.push(b"221\n").first().map(|r| (r.code, r.text().len())), Some((221, 0)));
}

fn smtp_stream(t: &mut T) {
    let mut st = Stream::new();
    // pipelined: MAIL, RCPT and DATA in one segment, the message in pieces
    st.push(b"MAIL FROM:<anna@a.lab>\r\nRCPT TO:<bob@b.lab>\r\nDATA\r\nSubj");
    t.eq("first line", st.line(), Some(s("MAIL FROM:<anna@a.lab>")));
    t.eq("second", st.line(), Some(s("RCPT TO:<bob@b.lab>")));
    t.eq("third", st.line(), Some(s("DATA")));
    t.eq("no fourth yet", st.line(), None);
    st.begin_data(0);
    t.check("in data", st.in_data() && st.line().is_none());
    t.eq("not yet", st.data(), None);
    st.push(b"ect: x\r\n\r\n..one dot\r\n.\r\nQUIT\r\n");
    let d = st.data();
    t.eq("message", d.as_ref().map(|d| d.body.clone()), Some(b"Subject: x\r\n\r\n.one dot\r\n".to_vec()));
    t.eq("size", d.as_ref().map(|d| (d.size, d.over)), Some((24, false)));
    t.eq("the command after it", st.line(), Some(s("QUIT")));
    let mut big = Stream::new();
    big.begin_data(10);
    big.push(b"0123456789\r\nabc\r\n.\r\n");
    t.eq("over the limit", big.data().map(|d| (d.size, d.over, d.body.len())), Some((17, true, 0)));
    t.eq("stuffing", smtp::stuff(b"a\n.b\n..c"), b"a\r\n..b\r\n...c\r\n.\r\n".to_vec());
    t.eq("stuffing an empty message", smtp::stuff(b""), b".\r\n".to_vec());
    // both ways: what stuff() sends, the stream gives back
    let msg = b"Subject: t\r\n\r\n.\r\n..\r\nend\r\n";
    let mut back = Stream::new();
    back.begin_data(0);
    back.push(&smtp::stuff(msg));
    t.eq("round trip", back.data().map(|d| d.body), Some(msg.to_vec()));
}

fn pop3_t(t: &mut T) {
    t.eq("parse", pop3::Command::parse("retr 2\r\n"), Some(pop3::Command { verb: s("RETR"), args: alloc::vec![s("2")] }));
    t.eq("a password with spaces", pop3::Command::parse("PASS ge heim").map(|c| c.args), Some(alloc::vec![s("ge heim")]));
    t.eq("line", pop3::Command::new("top", &["1", "0"]).to_line(), s("TOP 1 0\r\n"));
    let multi = |l: &str| pop3::Command::parse(l).is_some_and(|c| pop3::multi_line(&c));
    t.check("multi-line commands", multi("LIST") && multi("UIDL") && multi("RETR 1") && multi("TOP 1 5") && multi("CAPA"));
    t.check("single-line commands", !multi("LIST 1") && !multi("UIDL 2") && !multi("STAT") && !multi("DELE 1") && !multi("USER bob"));
    t.check("known verbs", pop3::verb_known("uidl") && pop3::verb_known("TOP") && !pop3::verb_known("EHLO"));
    t.eq("ok", pop3::Response::ok("2 messages").to_bytes(), b"+OK 2 messages\r\n".to_vec());
    t.eq("err", pop3::Response::err("[AUTH] no").to_bytes(), b"-ERR [AUTH] no\r\n".to_vec());
    let r = pop3::Response::ok("120 octets").with_body(b"Subject: x\r\n\r\n.hidden\r\nend\r\n");
    t.eq("multi-line with a dot", r.to_bytes(), b"+OK 120 octets\r\nSubject: x\r\n\r\n..hidden\r\nend\r\n.\r\n".to_vec());
    t.eq("an empty list", pop3::Response::ok("").with_body(b"").to_bytes(), b"+OK\r\n.\r\n".to_vec());
    t.eq("LF lines", pop3::stuffed(b"a\nb\n"), b"a\r\nb\r\n.\r\n".to_vec());
    let mut rd = pop3::Reader::new();
    t.eq("greeting", rd.push(b"+OK ready\r\n"), alloc::vec![pop3::Response::ok("ready")]);
    rd.expect(false);
    rd.expect(true);
    rd.expect(true);
    let got = rd.push(b"+OK bob\r\n+OK 2 messages\r\n1 120\r\n2 3");
    t.eq("one done, one open", got, alloc::vec![pop3::Response::ok("bob")]);
    let got = rd.push(b"40\r\n.\r\n-ERR no such message\r\n");
    t.eq("the list", got.first().and_then(|r| r.body.clone()), Some(b"1 120\r\n2 340\r\n".to_vec()));
    t.eq("an error has no body", got.get(1).map(|r| (r.ok, r.body.is_none())), Some((false, true)));
    let mut rd2 = pop3::Reader::new();
    let _ = rd2.push(b"+OK hi\r\n");
    rd2.expect(true);
    t.eq("dots undone", rd2.push(&r.to_bytes()).first().and_then(|x| x.body.clone()), Some(b"Subject: x\r\n\r\n.hidden\r\nend\r\n".to_vec()));
}

fn imf_header(t: &mut T) {
    let raw = b"From: Anna <anna@a.lab>\r\nTo: bob@b.lab,\r\n  carl@b.lab\r\nSubject: Hallo\r\n\tWelt\r\nbad line\r\n\r\nText\r\n";
    let m = Message::parse(raw);
    t.eq("fields", m.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), alloc::vec!["From", "To", "Subject"]);
    t.eq("unfolded", m.get("to"), Some("bob@b.lab,  carl@b.lab"));
    t.eq("subject unfolded", m.subject(), s("Hallo\tWelt"));
    t.eq("body", m.body.clone(), b"Text\r\n".to_vec());
    t.eq("LF lines", Message::parse(b"A: 1\n\nbody\n").get("A"), Some("1"));
    t.eq("no header", Message::parse(b"\r\nonly body").body, b"only body".to_vec());
    let long = "word ".repeat(30);
    let f = imf::fold("Subject", long.trim_end());
    t.check("folded lines at most 78", f.split("\r\n").all(|l| l.len() <= 78) && f.matches("\r\n ").count() >= 1);
    t.eq("unfold of fold", imf::unfold(f.trim_end_matches("\r\n")), alloc::format!("Subject: {}", long.trim_end()));
    t.eq("an empty value", imf::fold("X-Empty", ""), s("X-Empty:\r\n"));
    let mut m2 = Message::new();
    m2.add("Subject", "a");
    m2.prepend("Received", "from x by y; date");
    m2.set("subject", "b");
    m2.body = b"hi\r\n".to_vec();
    t.eq("set and prepend", m2.to_bytes(), b"Received: from x by y; date\r\nSubject: b\r\n\r\nhi\r\n".to_vec());
    let (ct, ps) = imf::params("text/plain; charset=\"UTF-8\"; format=flowed");
    t.eq("params", (ct, ps), (s("text/plain"), alloc::vec![(s("charset"), s("UTF-8")), (s("format"), s("flowed"))]));
    t.eq("param", imf::param("multipart/report; report-type=delivery-status; boundary=\"a;b\"", "boundary"), Some(s("a;b")));
}

fn imf_addresses(t: &mut T) {
    t.eq("name and address", imf::parse_mailbox("Anna Muster <anna@a.lab>"), Some(Mailbox::new("Anna Muster", "anna@a.lab")));
    t.eq("quoted name", imf::parse_mailbox("\"Muster, Anna\" <anna@a.lab>"), Some(Mailbox::new("Muster, Anna", "anna@a.lab")));
    t.eq("bare", imf::parse_mailbox("bob@b.lab"), Some(Mailbox::new("", "bob@b.lab")));
    t.eq("comment", imf::parse_mailbox("bob@b.lab (Bob)"), Some(Mailbox::new("Bob", "bob@b.lab")));
    t.eq("encoded name", imf::parse_mailbox("=?UTF-8?Q?J=C3=BCrgen?= <j@a.lab>").map(|m| m.name), Some(s("Jürgen")));
    t.eq("not an address", imf::parse_mailbox("nobody"), None);
    let l = imf::parse_list("\"Muster, Anna\" <anna@a.lab>, bob@b.lab, Team: carl@b.lab, dora@b.lab;, undisclosed-recipients:;");
    t.eq("list", l.iter().map(|m| m.addr.as_str()).collect::<Vec<_>>(), alloc::vec!["anna@a.lab", "bob@b.lab", "carl@b.lab", "dora@b.lab"]);
    t.eq("header form", (Mailbox::new("Anna Muster", "a@x").to_header(), Mailbox::new("Muster, Anna", "a@x").to_header(), Mailbox::new("", "a@x").to_header()), (s("Anna Muster <a@x>"), s("\"Muster, Anna\" <a@x>"), s("a@x")));
    t.eq("non-ASCII name", Mailbox::new("Jürgen", "j@a.lab").to_header(), s("=?UTF-8?Q?J=C3=BCrgen?= <j@a.lab>"));
    t.check("good addresses", imf::addr_ok("anna@a.lab") && imf::addr_ok("a.b+c@x-y.lab") && imf::addr_ok("\"a b\"@x.lab") && imf::addr_ok("root@[10.0.0.1]"));
    t.check("bad addresses", !imf::addr_ok("anna") && !imf::addr_ok("@a.lab") && !imf::addr_ok("a@") && !imf::addr_ok("a..b@x") && !imf::addr_ok("a@-x.lab") && !imf::addr_ok("a b@x"));
    t.eq("split", imf::split_addr("bob@b.lab"), Some(("bob", "b.lab")));
}

fn imf_dates(t: &mut T) {
    t.eq("epoch of the simulator", imf::date(1_767_225_600), s("Thu, 01 Jan 2026 00:00:00 +0000"));
    t.eq("a leap day", imf::date(951_782_400 + 86_399), s("Tue, 29 Feb 2000 23:59:59 +0000"));
    t.eq("1970", imf::date(0), s("Thu, 01 Jan 1970 00:00:00 +0000"));
    t.eq("read back", imf::parse_date("Thu, 01 Jan 2026 00:00:05 +0000"), Some(1_767_225_605));
    t.eq("a zone", imf::parse_date("1 Jan 2026 02:00:05 +0200"), Some(1_767_225_605));
    t.eq("an old zone and a comment", imf::parse_date("Wed, 31 Dec 2025 19:00:05 EST (Eastern)"), Some(1_767_225_605));
    t.eq("two-digit year", imf::parse_date("01 Jan 26 00:00 GMT"), Some(1_767_225_600));
    t.eq("garbage", imf::parse_date("yesterday"), None);
    t.eq("message id", imf::message_id("20260101000005.4F2A@pc1", "a.lab"), s("<20260101000005.4F2A@pc1@a.lab>"));
}

fn imf_encodings(t: &mut T) {
    t.eq("ascii stays", imf::encode_word("Hallo"), s("Hallo"));
    t.eq("encoded word", imf::encode_word("Grüße aus A"), s("=?UTF-8?Q?Gr=C3=BC=C3=9Fe_aus_A?="));
    t.eq("decoded", imf::decode_words("=?UTF-8?Q?Gr=C3=BC=C3=9Fe_aus_A?="), s("Grüße aus A"));
    t.eq("B and latin1", imf::decode_words("=?UTF-8?B?R3LDvMOfZQ==?= =?ISO-8859-1?Q?=FC?= und mehr"), s("Grüßeü und mehr"));
    let long = "ä".repeat(40);
    let enc = imf::encode_word(&long);
    t.check("long words split, at most 75 each", enc.split(' ').count() > 1 && enc.split(' ').all(|w| w.len() <= 75));
    t.eq("long words back", imf::decode_words(&enc), long.clone());
    t.eq("qp", imf::qp_encode("Grüße = gut \nZeile".as_bytes()), b"Gr=C3=BC=C3=9Fe =3D gut=20\r\nZeile".to_vec());
    t.eq("qp back", imf::qp_decode(b"Gr=C3=BC=C3=9Fe =3D gut=20\r\nZeile"), "Grüße = gut \r\nZeile".as_bytes().to_vec());
    let line = "x".repeat(100);
    let q = imf::qp_encode(line.as_bytes());
    t.check("soft line breaks", q.split(|&c| c == b'\n').all(|l| l.len() <= 77) && q.windows(3).any(|w| w == b"=\r\n"));
    t.eq("soft breaks back", imf::qp_decode(&q), line.as_bytes().to_vec());
    t.eq("base64", imf::b64_encode(b"Hallo Welt"), s("SGFsbG8gV2VsdA=="));
    t.eq("base64 back", imf::b64_decode(b"SGFs\r\nbG8gV2VsdA=="), Some(b"Hallo Welt".to_vec()));
    t.eq("base64 garbage", imf::b64_decode(b"SG*s"), None);
    t.check("base64 lines", imf::b64_lines(&[7u8; 100]).split("\r\n").all(|l| l.len() <= 76));
}

fn imf_bodies(t: &mut T) {
    let d = Draft {
        from: Mailbox::new("Anna Muster", "anna@a.lab"),
        to: alloc::vec![Mailbox::new("", "bob@b.lab")],
        cc: Vec::new(),
        subject: String::from("Grüße"),
        date: 1_767_225_605,
        id: imf::message_id("1.2", "a.lab"),
        in_reply_to: String::new(),
        references: String::new(),
        text: String::from("Hallo Bob,\nbis bald.\n"),
    };
    let raw = d.to_bytes(false);
    let m = Message::parse(&raw);
    t.eq("7bit", m.get("Content-Transfer-Encoding"), Some("7bit"));
    t.eq("subject encoded", m.get("Subject"), Some("=?UTF-8?Q?Gr=C3=BC=C3=9Fe?="));
    t.eq("subject read", m.subject(), s("Grüße"));
    t.eq("from", m.from(), Some(Mailbox::new("Anna Muster", "anna@a.lab")));
    t.eq("to", m.to(), alloc::vec![Mailbox::new("", "bob@b.lab")]);
    t.eq("date", m.date(), Some(1_767_225_605));
    t.eq("text", m.text(), s("Hallo Bob,\nbis bald.\n"));
    let u = Draft { text: String::from("Schöne Grüße\n"), ..d.clone() };
    let m8 = Message::parse(&u.to_bytes(true));
    t.eq("8bit", (m8.get("Content-Transfer-Encoding"), m8.text()), (Some("8bit"), s("Schöne Grüße\n")));
    let mq = Message::parse(&u.to_bytes(false));
    t.eq("quoted-printable", (mq.get("Content-Transfer-Encoding"), mq.text()), (Some("quoted-printable"), s("Schöne Grüße\n")));
    t.check("a 7-bit message", u.to_bytes(false).iter().all(|&c| c < 128));
    // a bounce: multipart/report, the text part first
    let report = b"Content-Type: multipart/report; report-type=delivery-status; boundary=\"B1\"\r\nMIME-Version: 1.0\r\n\r\nPreamble\r\n--B1\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\nThe mail could not be delivered.\r\n--B1\r\nContent-Type: message/delivery-status\r\n\r\nFinal-Recipient: rfc822; x@b.lab\r\nStatus: 5.1.1\r\n--B1--\r\n";
    let r = Message::parse(report);
    t.eq("report text", r.text(), s("The mail could not be delivered."));
    let parts = imf::split_multipart(&r.body, "B1");
    t.eq("report parts", parts.len(), 2);
    t.eq("the status part", parts.get(1).map(|p| Message::parse(p).get("Content-Type").map(String::from)), Some(Some(s("message/delivery-status"))));
    t.eq("crlf and back", imf::lf(&imf::crlf(b"a\nb\r\nc\n")), b"a\nb\nc\n".to_vec());
}

fn segments(t: &mut T) {
    let c = segment(40000, smtp::PORT_SMTP, b"EHLO pc1\r\n");
    t.eq("smtp mask", decode::proto_mask(&c) & (decode::P_MAIL | decode::P_TCP), decode::P_MAIL | decode::P_TCP);
    t.eq("smtp request summary", decode::info(&c), (Proto::Smtp, s("C: EHLO pc1")));
    t.eq("smtp request fields", fields(&c, Proto::Smtp), alloc::vec![("Command Line", s("EHLO pc1")), ("Command", s("EHLO")), ("Request parameter", s("pc1"))]);
    let r = segment(smtp::PORT_SMTP, 40000, b"250-mail.a.lab hi\r\n250 5.1.0 fine\r\n");
    t.eq("smtp response summary", decode::info(&r), (Proto::Smtp, s("S: 250-mail.a.lab hi")));
    let f = fields(&r, Proto::Smtp);
    t.eq("smtp enhanced code", f.iter().find(|x| x.0 == "Enhanced status code").map(|x| x.1.clone()), Some(s("5.1.0")));
    t.eq("smtp codes", f.iter().filter(|x| x.0 == "Response code").count(), 2);
    let d = segment(40000, smtp::PORT_SMTP, b"Subject: x\r\n\r\n..dot\r\n.\r\n");
    t.eq("smtp data summary", decode::info(&d), (Proto::Smtp, s("C: DATA fragment, 24 bytes")));
    t.eq("smtp data lines", fields(&d, Proto::Smtp).iter().map(|x| x.0).collect::<Vec<_>>(), alloc::vec!["Message line", "Message line", "Message line", "End of message"]);
    let sub = segment(40001, smtp::PORT_SUBMISSION, b"AUTH PLAIN AGFubmEAZ2VoZWlt\r\n");
    t.eq("submission port", decode::info(&sub).0, Proto::Smtp);
    let p = segment(40002, pop3::PORT_POP3, b"UIDL\r\n");
    t.eq("pop3 summary", decode::info(&p), (Proto::Pop3, s("C: UIDL")));
    t.eq("pop3 mask", decode::proto_mask(&p) & decode::P_MAIL, decode::P_MAIL);
    let pr = segment(pop3::PORT_POP3, 40002, b"+OK 2 messages\r\n1 a1\r\n2 b2\r\n.\r\n");
    t.eq("pop3 response fields", fields(&pr, Proto::Pop3).iter().map(|x| x.0).collect::<Vec<_>>(), alloc::vec!["Response", "Response indicator", "Response description", "Data line", "Data line", "End of data"]);
    t.eq("pop3 response summary", decode::info(&pr), (Proto::Pop3, s("S: +OK 2 messages")));
    let rest = segment(pop3::PORT_POP3, 40002, b"more body\r\n");
    t.eq("pop3 continuation", decode::info(&rest), (Proto::Pop3, s("S: DATA fragment, 11 bytes")));
    let bad = segment(40002, pop3::PORT_POP3, b"HELO x\r\n");
    t.eq("pop3 unknown command", decode::layers(&bad).iter().find(|l| l.proto == Proto::Pop3).map(|l| l.problems.clone()), Some(alloc::vec![s("a command POP3 does not define: HELO")]));
    t.eq("names", (Proto::Smtp.name(), Proto::Pop3.name()), ("SMTP", "POP3"));
}
