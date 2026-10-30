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
