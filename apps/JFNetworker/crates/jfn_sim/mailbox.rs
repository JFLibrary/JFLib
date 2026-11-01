// mailbox -- the mailboxes of a mail server in the device's file store (files.rs): one Maildir
// per user, a file per message, as Postfix and Dovecot keep them with home_mailbox = Maildir/
//
//   SPOOL                       "/var/mail": where the mailboxes are (the services' "mailbox")
//   maildir(root, user)         "/var/mail/bob": the Maildir of a user (new/ and cur/ in it)
//   messages(store, root, user) the messages of a mailbox in the order they came: (unique name,
//                               path, size in bytes as they travel with CRLF line ends)
//   octets(data)                that size of a stored message
//   unique(secs, usec, n, host) the name of a new message: "1767225612.M345678Q1.mailb"
//   OsApi::store_put / store_remove / store_report   writes and removals without a report,
//                               then one report for all of them (a delivery to several
//                               mailboxes, the deletions at the end of a POP3 session)
//
// Folders and flags for IMAP (imapd.rs), Maildir++ as Dovecot lays them out:
//
//   INBOX, SENT, TRASH          the names of the folders every mailbox has
//   folder_dir(root, user, f)   INBOX is the Maildir itself, another folder "<Maildir>/.<name>"
//   folders(st, root, user)     the folders that exist, INBOX first, the others by name
//   FolderMsg { uid, name, path, flags, size, recent }   a message of a folder
//   folder_msgs(st, dir)        its messages in UID order; names without a UID yet get the next
//                               numbers, given back with whether the list changed
//   uidlist(st, dir) / uidlist_text(..)   the file .uidlist: "UIDVALIDITY UIDNEXT", then "uid name"
//   flags_of(name) / set_flags(name, flags)   Maildir's ":2,FRST" letters <-> IMAP's flags
//   imap_flags(letters, recent) the flag words of a message
//
// The flags live in the file's name (a message in new/ has none and counts as \Recent; one that gets
// a flag moves to cur/), so POP3, which reads new/ and cur/ by the unique name, keeps working on
// the same mailbox.  \Deleted is the letter T as Maildir has it.
//
// A message lies in new/ as the SMTP server delivered it (Return-Path on top, line ends "\n"
// as in a Unix mailbox); cur/ holds those a program moved there (a name may carry Maildir's
// ":2,S" flags, which the unique name leaves out).  The simulator writes a file in one step,
// so tmp/ is not needed.  The directories come with the first message; a user without them
// has an empty mailbox.  Nobody else takes part in the file names: a POP3 server's UIDL is the
// unique name.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::{FsErr, Store};

use crate::os::OsApi;

/// where the mailboxes are
pub const SPOOL: &str = "/var/mail";

/// the Maildir of a user
pub fn maildir(root: &str, user: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), user)
}

/// the unique name of a message: what the name of a Maildir file starts with (without the
/// ":2," flags)
pub fn uid_of(name: &str) -> &str {
    name.split(':').next().unwrap_or(name)
}

/// a new message's name: the device clock's seconds and microseconds, a counter of the
/// service, the device's name
pub fn unique(secs: i64, usec: u32, n: u32, host: &str) -> String {
    let h: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_ascii_lowercase();
    format!("{}.M{}Q{}.{}", secs, usec, n, if h.is_empty() { "host" } else { &h })
}

/// the size of a message as it travels (every "\n" as CRLF)
pub fn octets(data: &[u8]) -> u64 {
    let mut n = data.len() as u64;
    let mut prev = 0u8;
    for &c in data {
        if c == b'\n' && prev != b'\r' {
            n += 1;
        }
        prev = c;
    }
    n
}

/// the messages of a user's mailbox, oldest first: (unique name, path, octets); new/ and cur/
/// together, by name (the name starts with the time it came)
pub fn messages(st: &Store, root: &str, user: &str) -> Vec<(String, String, u64)> {
    let base = maildir(root, user);
    let mut v: Vec<(String, String, u64)> = Vec::new();
    for sub in ["new", "cur"] {
        let dir = format!("{}/{}", base, sub);
        for (name, s) in st.list(&dir).unwrap_or_default() {
            if s.dir {
                continue;
            }
            let path = format!("{}/{}", dir, name);
            let size = st.read(&path).map(octets).unwrap_or(s.size);
            v.push((String::from(uid_of(&name)), path, size));
        }
    }
    v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    v
}

/// the user has a mailbox (its directory)
pub fn has_mailbox(st: &Store, root: &str, user: &str) -> bool {
    st.is_dir(&maildir(root, user))
}

pub const INBOX: &str = "INBOX";
pub const SENT: &str = "Sent";
pub const TRASH: &str = "Trash";

/// a folder's directory in a user's Maildir
pub fn folder_dir(root: &str, user: &str, folder: &str) -> String {
    if folder.eq_ignore_ascii_case(INBOX) { maildir(root, user) } else { format!("{}/.{}", maildir(root, user), folder) }
}

/// the folders of a mailbox: INBOX, then the others by name
pub fn folders(st: &Store, root: &str, user: &str) -> Vec<String> {
    let mut v = alloc::vec![String::from(INBOX)];
    for (name, s) in st.list(&maildir(root, user)).unwrap_or_default() {
        if s.dir
            && let Some(n) = name.strip_prefix('.')
        {
            v.push(String::from(n));
        }
    }
    v[1..].sort();
    v
}

/// a message of a folder
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderMsg {
    pub uid: u32,
    /// the unique name (without Maildir's flags)
    pub name: String,
    pub path: String,
    /// Maildir's letters
    pub letters: String,
    pub size: u64,
    /// it still lies in new/
    pub recent: bool,
}

/// the letters of a file's name: what follows ":2,"
pub fn flags_of(name: &str) -> String {
    name.split_once(":2,").map(|x| String::from(x.1)).unwrap_or_default()
}

/// a unique name with these letters (in the alphabetical order Maildir asks for)
pub fn set_flags(uid_name: &str, letters: &str) -> String {
    let mut l: Vec<char> = letters.chars().collect();
    l.sort_unstable();
    l.dedup();
    format!("{}:2,{}", uid_name, l.into_iter().collect::<String>())
}

/// the flag words of Maildir's letters
pub fn imap_flags(letters: &str, recent: bool) -> Vec<&'static str> {
    use netpkt::imap;
    let mut v = Vec::new();
    for (c, w) in [('R', imap::ANSWERED), ('D', imap::DRAFT), ('F', imap::FLAGGED), ('S', imap::SEEN), ('T', imap::DELETED)] {
        if letters.contains(c) {
            v.push(w);
        }
    }
    if recent {
        v.push(imap::RECENT);
    }
    v
}

/// the letter of a flag word
pub fn letter_of(flag: &str) -> Option<char> {
    match flag.to_ascii_lowercase().as_str() {
        "\\seen" => Some('S'),
        "\\answered" => Some('R'),
        "\\flagged" => Some('F'),
        "\\deleted" => Some('T'),
        "\\draft" => Some('D'),
        _ => None,
    }
}

/// the file of a folder's UIDs
fn uidlist_path(dir: &str) -> String {
    format!("{}/.uidlist", dir)
}

/// the folder's .uidlist: (UIDVALIDITY, UIDNEXT, name -> uid)
pub fn uidlist(st: &Store, dir: &str) -> (u32, u32, alloc::collections::BTreeMap<String, u32>) {
    let mut map = alloc::collections::BTreeMap::new();
    let Ok(b) = st.read(&uidlist_path(dir)) else {
        return (0, 1, map);
    };
    let text = String::from_utf8_lossy(b);
    let mut lines = text.lines();
    let mut head = lines.next().unwrap_or("").split_whitespace();
    let validity = head.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let next = head.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    for l in lines {
        if let Some((u, n)) = l.split_once(' ')
            && let Ok(u) = u.parse::<u32>()
        {
            map.insert(String::from(n), u);
        }
    }
    (validity, next, map)
}

/// the messages of a folder in UID order and the file .uidlist as it should be now (None: it
/// already is): what has no UID yet gets the next ones, by name (that is by time); `validity` is
/// taken for a folder without a list
pub fn folder_msgs(st: &Store, dir: &str, validity: u32) -> (Vec<FolderMsg>, Option<(u32, Vec<u8>)>) {
    let (mut val, mut next, map) = uidlist(st, dir);
    let had = st.exists(&uidlist_path(dir));
    if val == 0 {
        val = validity;
    }
    let mut files: Vec<(String, String, String, u64, bool)> = Vec::new();
    for sub in ["new", "cur"] {
        let d = format!("{}/{}", dir, sub);
        for (name, s) in st.list(&d).unwrap_or_default() {
            if s.dir {
                continue;
            }
            let path = format!("{}/{}", d, name);
            let size = st.read(&path).map(octets).unwrap_or(s.size);
            files.push((String::from(uid_of(&name)), path, flags_of(&name), size, sub == "new"));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::with_capacity(files.len());
    let mut changed = !had;
    let mut keep = alloc::collections::BTreeMap::new();
    for (name, path, letters, size, recent) in files {
        let uid = match map.get(&name) {
            Some(&u) => u,
            None => {
                let u = next;
                next += 1;
                changed = true;
                u
            }
        };
        keep.insert(name.clone(), uid);
        out.push(FolderMsg { uid, name, path, letters, size, recent });
    }
    if map.len() != keep.len() {
        changed = true;
    }
    out.sort_by_key(|m| m.uid);
    if !changed {
        return (out, None);
    }
    let mut t = format!("{} {}\n", val, next);
    let mut rows: Vec<(&String, &u32)> = keep.iter().collect();
    rows.sort_by_key(|r| *r.1);
    for (n, u) in rows {
        t.push_str(&format!("{} {}\n", u, n));
    }
    (out, Some((val, t.into_bytes())))
}

/// writes a folder's .uidlist
pub fn uidlist_file(dir: &str) -> String {
    uidlist_path(dir)
}

impl OsApi<'_> {
    /// writes a file into the store without a report (store_report follows); the directories
    /// on the way are made
    pub(crate) fn store_put(&mut self, path: &str, data: &[u8]) -> Result<String, FsErr> {
        let t = self.file_mtime();
        let os = self.eng.os_mut(self.ni).ok_or(FsErr::NotFound)?;
        let p = os.files.norm(path)?;
        let dir = os.files.parent(&p);
        if !os.files.is_dir(&dir) {
            os.files.mkdir(&dir, t, true)?;
        }
        os.files.write(&p, data, t)?;
        Ok(p)
    }
    /// makes a directory (and those on the way) without a report
    pub(crate) fn store_mkdir(&mut self, path: &str) -> Result<(), FsErr> {
        let t = self.file_mtime();
        let os = self.eng.os_mut(self.ni).ok_or(FsErr::NotFound)?;
        os.files.mkdir(path, t, true)
    }
    /// removes a file without a report
    pub(crate) fn store_remove(&mut self, path: &str) -> Result<(), FsErr> {
        let os = self.eng.os_mut(self.ni).ok_or(FsErr::NotFound)?;
        os.files.remove(path)
    }
    /// the project hears of the changes (one step), named by `path`
    pub(crate) fn store_report(&mut self, path: &str) {
        let ni = self.ni;
        self.eng.files_changed(ni, path);
    }
}
