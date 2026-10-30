// mailtrace -- the decisions of mail as notes (Note::Mail) with their German sentences: the SMTP
// server (sessions, logins, senders and recipients taken or refused, refused relays, messages
// too big, the queue), its deliveries (local mailboxes, MX lookups, relays, deferrals with
// their next attempt, bounces), the POP3 server (logins, retrievals, deletions) and the clients
// (E-Mail, curl, mail)
//
//   MNote::message() -> Msg     the sentence
//   MNote::is_drop() / mask()   something refused, bounced or given up; the P_* bits (mail
//                               over TCP)
//
// Addresses as the envelope named them, queue ids as the server made them, sizes in bytes.

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::{dur, Msg};
use netpkt::decode::{P_IPV4, P_MAIL, P_TCP};
use netpkt::IpAddr;

use crate::kernel::Ns;

/// a decision of mail
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MNote {
    /// a client connects to the SMTP server (port 25, or 587 for submission)
    SmtpSession { from: IpAddr, port: u16 },
    /// it names itself with EHLO (else HELO)
    SmtpHelo { name: String, esmtp: bool },
    /// AUTH: the login of a user accepted or refused
    SmtpAuth { user: String, ok: bool },
    /// MAIL FROM taken (size: what SIZE announced, 0 without)
    SmtpMail { from: String, size: u64 },
    /// RCPT TO taken: a mailbox here or a relay
    SmtpRcpt { rcpt: String, local: bool },
    /// a command refused with its code (an unknown user, a missing EHLO, AUTH needed ...)
    SmtpRefused { cmd: String, code: u16 },
    /// a recipient elsewhere from a client that may not relay
    RelayDenied { rcpt: String, from: IpAddr },
    /// the message is bigger than the limit (announced or sent)
    TooBig { size: u64, max: u64 },
    /// the message is in the queue
    Queued { id: String, rcpts: u32, bytes: u64 },
    /// delivered into a local mailbox (the file)
    Delivered { id: String, rcpt: String, path: String },
    /// the MX records of a domain (or its address when it has none)
    MxFound { domain: String, hosts: String },
    /// handed to the next mail server
    Relayed { id: String, rcpt: String, host: String, reply: String },
    /// not delivered now: another attempt later
    Deferred { id: String, rcpt: String, why: Msg, again: Ns },
    /// given up on: the sender gets a bounce
    Bounced { id: String, rcpt: String, why: Msg },
    /// the bounce goes to the sender
    BounceSent { id: String, to: String },
    /// a bounce that cannot be delivered either is thrown away
    BounceDropped { id: String },
    /// a client connects to the POP3 server
    Pop3Session { from: IpAddr },
    /// a login with the messages of the mailbox
    Pop3Login { user: String, msgs: u32, bytes: u64 },
    Pop3LoginFailed { user: String },
    /// RETR or TOP sends a message
    Pop3Retr { n: u32, bytes: u64 },
    /// the end of a session (QUIT) removes the messages marked with DELE
    Pop3Deleted { user: String, n: u32 },
    /// a command refused (no such message, not logged in)
    Pop3Refused { cmd: String },
    /// a client handed in a message
    ClientSent { to: String, server: String },
    /// a client fetched messages
    ClientFetched { n: u32, server: String },
    /// a client failed (the reason as the program words it)
    ClientFailed { what: Msg },
}

impl MNote {
    pub fn message(&self) -> Msg {
        match self {
            MNote::SmtpSession { from, port } => msg!("SMTP: {} verbindet sich (Port {}).", from, port),
            MNote::SmtpHelo { name, esmtp: true } => msg!("SMTP: der Client nennt sich {} (EHLO).", name),
            MNote::SmtpHelo { name, esmtp: false } => msg!("SMTP: der Client nennt sich {} (HELO, ohne Erweiterungen).", name),
            MNote::SmtpAuth { user, ok: true } => msg!("SMTP: {} ist angemeldet (AUTH).", user),
            MNote::SmtpAuth { user, ok: false } => msg!("SMTP: Anmeldung von {} abgelehnt (AUTH).", user),
            MNote::SmtpMail { from, size: 0 } => msg!("SMTP: Absender <{}> angenommen.", from),
            MNote::SmtpMail { from, size } => msg!("SMTP: Absender <{}> angenommen, angekündigt {} Bytes.", from, size),
            MNote::SmtpRcpt { rcpt, local: true } => msg!("SMTP: Empfänger <{}> hat hier ein Postfach.", rcpt),
            MNote::SmtpRcpt { rcpt, local: false } => msg!("SMTP: Empfänger <{}> liegt woanders: wird weitergeleitet.", rcpt),
            MNote::SmtpRefused { cmd, code } => msg!("SMTP: {} abgelehnt ({}).", cmd, code),
            MNote::RelayDenied { rcpt, from } => msg!("SMTP: kein Relay für <{}>: {} darf nicht weiterleiten lassen.", rcpt, from),
            MNote::TooBig { size, max } => msg!("SMTP: Nachricht mit {} Bytes ist größer als erlaubt ({} Bytes).", size, max),
            MNote::Queued { id, rcpts, bytes } => msg!("SMTP: Nachricht {} in der Warteschlange: {} Empfänger, {} Bytes.", id, rcpts, bytes),
            MNote::Delivered { id, rcpt, path } => msg!("Mail: {} an <{}> zugestellt: {}.", id, rcpt, path),
            MNote::MxFound { domain, hosts } => msg!("Mail: Mailserver von {}: {}.", domain, hosts),
            MNote::Relayed { id, rcpt, host, reply } => msg!("Mail: {} an <{}> an {} übergeben ({}).", id, rcpt, host, reply),
            MNote::Deferred { id, rcpt, why, again } => msg!("Mail: {} an <{}> zurückgestellt ({}), nächster Versuch in {}.", id, rcpt, why, dur(*again as i64)),
            MNote::Bounced { id, rcpt, why } => msg!("Mail: {} an <{}> nicht zustellbar ({}).", id, rcpt, why),
            MNote::BounceSent { id, to } => msg!("Mail: Unzustellbarkeitsnachricht zu {} geht an <{}>.", id, to),
            MNote::BounceDropped { id } => msg!("Mail: Unzustellbarkeitsnachricht {} ist selbst nicht zustellbar: verworfen.", id),
            MNote::Pop3Session { from } => msg!("POP3: {} verbindet sich.", from),
            MNote::Pop3Login { user, msgs, bytes } => msg!("POP3: {} ist angemeldet, {} Nachrichten mit {} Bytes.", user, msgs, bytes),
            MNote::Pop3LoginFailed { user } => msg!("POP3: Anmeldung von {} abgelehnt.", user),
            MNote::Pop3Retr { n, bytes } => msg!("POP3: Nachricht {} gesendet, {} Bytes.", n, bytes),
            MNote::Pop3Deleted { user, n } => msg!("POP3: {} Nachrichten von {} gelöscht.", n, user),
            MNote::Pop3Refused { cmd } => msg!("POP3: {} abgelehnt.", cmd),
            MNote::ClientSent { to, server } => msg!("E-Mail: Nachricht an {} über {} gesendet.", to, server),
            MNote::ClientFetched { n: 1, server } => msg!("E-Mail: eine neue Nachricht von {} abgerufen.", server),
            MNote::ClientFetched { n, server } => msg!("E-Mail: {} neue Nachrichten von {} abgerufen.", n, server),
            MNote::ClientFailed { what } => msg!("E-Mail: {}", what),
        }
    }
    /// something was refused, bounced or given up
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            MNote::SmtpAuth { ok: false, .. }
                | MNote::SmtpRefused { .. }
                | MNote::RelayDenied { .. }
                | MNote::TooBig { .. }
                | MNote::Bounced { .. }
                | MNote::BounceDropped { .. }
                | MNote::Pop3LoginFailed { .. }
                | MNote::Pop3Refused { .. }
                | MNote::ClientFailed { .. }
        )
    }
    /// the protocols of the note
    pub fn mask(&self) -> u32 {
        P_IPV4 | P_TCP | P_MAIL
    }
}
