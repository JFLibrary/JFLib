// xfertrace -- the decisions of the file transfers as notes (Note::Xfer) with their German
// sentences: TFTP servers and clients (requests, options, blocks sent again, a duplicate ACK,
// errors, unknown transfer IDs, giving up, the end), FTP servers and clients (logins accepted
// and refused, data connections, transfers, refusals, quotas)
//
//   XNote::message() -> Msg     the sentence
//   XNote::is_drop() / mask()   something refused or given up; the P_* bits (TFTP over UDP, FTP
//                               over TCP)
//
// Files are named as the request named them, addresses as they came; sizes are bytes.

use alloc::string::String;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::decode::{P_FTP, P_IPV4, P_TCP, P_TFTP, P_UDP};
use netpkt::IpAddr;

/// a decision of a file transfer
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XNote {
    /// a TFTP server takes a read or write request from a client, in blocks of `blksize`
    TftpRequest { write: bool, file: String, from: IpAddr, blksize: u16 },
    /// a TFTP server answers with an error (the code and its text)
    TftpRefused { file: String, code: u16, why: String },
    /// the options both sides agreed on (an OACK went out or came in)
    TftpOptions { opts: String },
    /// a packet went out again after its timeout (attempt n)
    TftpAgain { block: u16, n: u8 },
    /// a duplicate ACK is ignored: no second DATA (RFC 1123 4.2.3.1)
    TftpDupAck { block: u16 },
    /// a duplicate DATA block is acknowledged again
    TftpDupData { block: u16 },
    /// a packet from a port that is no part of the transfer: ERROR 5 to it
    TftpUnknownTid { from: IpAddr, port: u16 },
    /// no answer after the last attempt: the transfer ends
    TftpGaveUp { file: String, block: u16 },
    /// the other side sent an ERROR: the transfer ends
    TftpError { code: u16, why: String },
    /// the transfer is complete
    TftpDone { file: String, write: bool, bytes: u64 },
    /// a TFTP client sends its request
    TftpAsk { write: bool, file: String, to: IpAddr },
    /// an FTP server accepts a login (anonymous: the user name says it)
    FtpLogin { user: String },
    /// it refuses one (a wrong password, an unknown name, anonymous logins off)
    FtpLoginFailed { user: String },
    /// a passive data connection waits on a port / an active one goes to the client
    FtpPassive { port: u16 },
    FtpActive { to: IpAddr, port: u16 },
    /// a command needs a login, a file that is not there, a right the user has not
    FtpRefused { cmd: String, code: u16 },
    /// a transfer of the data connection is done (RETR, STOR, LIST, NLST)
    FtpTransfer { cmd: String, bytes: u64 },
    /// an upload does not fit (the quota of the server or the device's space)
    FtpFull { file: String },
}

impl XNote {
    pub fn message(&self) -> Msg {
        match self {
            XNote::TftpRequest { write: false, file, from, blksize } => msg!("TFTP: Leseanfrage für {} von {}, Blöcke zu {} Byte.", file, from, blksize),
            XNote::TftpRequest { write: true, file, from, blksize } => msg!("TFTP: Schreibanfrage für {} von {}, Blöcke zu {} Byte.", file, from, blksize),
            XNote::TftpRefused { file, code, why } => msg!("TFTP: {} abgelehnt, Fehler {} ({}).", file, code, why),
            XNote::TftpOptions { opts } => msg!("TFTP: Optionen vereinbart: {}.", opts),
            XNote::TftpAgain { block, n } => msg!("TFTP: keine Antwort auf Block {}: noch einmal gesendet (Versuch {}).", block, n),
            XNote::TftpDupAck { block } => msg!("TFTP: doppelte Bestätigung von Block {} ignoriert (kein zweites Senden).", block),
            XNote::TftpDupData { block } => msg!("TFTP: Block {} kam noch einmal: wieder bestätigt.", block),
            XNote::TftpUnknownTid { from, port } => msg!("TFTP: Paket von {} Port {} gehört zu keiner Übertragung: Fehler 5.", from, port),
            XNote::TftpGaveUp { file, block } => msg!("TFTP: keine Antwort mehr, die Übertragung von {} endet bei Block {}.", file, block),
            XNote::TftpError { code, why } => msg!("TFTP: die Gegenseite meldet Fehler {} ({}): die Übertragung endet.", code, why),
            XNote::TftpDone { file, write: false, bytes } => msg!("TFTP: {} gesendet, {} Bytes.", file, bytes),
            XNote::TftpDone { file, write: true, bytes } => msg!("TFTP: {} empfangen, {} Bytes.", file, bytes),
            XNote::TftpAsk { write: false, file, to } => msg!("TFTP: fragt {} nach {}.", to, file),
            XNote::TftpAsk { write: true, file, to } => msg!("TFTP: will {} an {} senden.", file, to),
            XNote::FtpLogin { user } => msg!("FTP: {} ist angemeldet.", user),
            XNote::FtpLoginFailed { user } => msg!("FTP: Anmeldung von {} abgelehnt.", user),
            XNote::FtpPassive { port } => msg!("FTP: passive Datenverbindung auf Port {} angeboten.", port),
            XNote::FtpActive { to, port } => msg!("FTP: aktive Datenverbindung zu {} Port {}.", to, port),
            XNote::FtpRefused { cmd, code } => msg!("FTP: {} abgelehnt ({}).", cmd, code),
            XNote::FtpTransfer { cmd, bytes } => msg!("FTP: {} fertig, {} Bytes.", cmd, bytes),
            XNote::FtpFull { file } => msg!("FTP: {} passt nicht: Kontingent oder Speicher erschöpft.", file),
        }
    }
    /// something was refused, thrown away or given up
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            XNote::TftpRefused { .. } | XNote::TftpUnknownTid { .. } | XNote::TftpGaveUp { .. } | XNote::TftpError { .. } | XNote::FtpLoginFailed { .. } | XNote::FtpRefused { .. } | XNote::FtpFull { .. }
        )
    }
    /// the protocols of the note
    pub fn mask(&self) -> u32 {
        match self {
            XNote::TftpRequest { .. }
            | XNote::TftpRefused { .. }
            | XNote::TftpOptions { .. }
            | XNote::TftpAgain { .. }
            | XNote::TftpDupAck { .. }
            | XNote::TftpDupData { .. }
            | XNote::TftpUnknownTid { .. }
            | XNote::TftpGaveUp { .. }
            | XNote::TftpError { .. }
            | XNote::TftpDone { .. }
            | XNote::TftpAsk { .. } => P_IPV4 | P_UDP | P_TFTP,
            _ => P_IPV4 | P_TCP | P_FTP,
        }
    }
}
