// agent -- the interface between server and agent (mutual TLS, port 7680 on the agent)
//
//   GET  /v1/info                       AgentInfo
//   GET  /v1/list?path=&offset=         AgentListing (entries with every metadata field)
//   GET  /v1/stat?path=                 AgentEntry (the entry itself, links not followed)
//   GET  /v1/read?path=&offset=&len=    the bytes (at most READ_MAX)
//   PUT  /v1/roots                      AgentRoots (the folders shared for backup / restore)
//   POST /v1/restore/open               RestoreOpen -> RestoreOpened (skip, or a temporary file)
//   POST /v1/restore/write?tmp=&offset= the bytes (at most READ_MAX)
//   POST /v1/restore/commit             RestoreCommit (rename into place, times, mode)
//   POST /v1/restore/mkdir              RestoreDir
//   POST /v1/restore/symlink            RestoreLink
//   POST /v1/restore/abort?tmp=
//
// Every path must lie inside the agent's shared folders (backup: roots, restore:
// restore_roots); the agent checks it itself and never follows a link while writing.  There is
// no command execution.

use alloc::string::String;
use alloc::vec::Vec;

use crate::model::Os;
use crate::record;

/// the largest read / write of one request
pub const READ_MAX: u64 = 8 << 20;

/// the CN of the certificate the server talks to agents with (issued by the server's CA)
pub const SERVER_CN: &str = "BackupCraft Server";

record! {
    pub struct AgentInfo {
        pub host_id: String,
        pub name: String,
        pub os: Os,
        pub platform: String,
        pub version: String,
        pub roots: Vec<String>,
        pub restore_roots: Vec<String>,
    }
}

record! {
    /// an entry with everything a backup keeps
    pub struct AgentEntry {
        pub name: String,
        /// "file", "dir", "link", "other"
        pub kind: String,
        pub size: u64,
        pub mtime_ns: i64,
        pub atime_ns: i64,
        pub ctime_ns: i64,
        pub mode: u32,
        pub uid: u32,
        pub gid: u32,
        pub attrs: u32,
        pub ino: u64,
        pub dev: u64,
        pub link: String,
    }
}

record! {
    pub struct AgentListing {
        pub path: String,
        pub entries: Vec<AgentEntry>,
        pub more: bool,
    }
}

record! {
    pub struct AgentRoots {
        pub roots: Vec<String>,
        pub restore_roots: Vec<String>,
    }
}

record! {
    pub struct RestoreOpen {
        pub path: String,
        /// "skip", "rename", "overwrite"
        pub conflict: String,
    }
}

record! {
    pub struct RestoreOpened {
        /// the file exists and is skipped
        pub skip: bool,
        /// where the data goes (a temporary file next to the target)
        pub tmp: String,
        /// the final name (may be renamed)
        pub dest: String,
    }
}

record! {
    pub struct RestoreCommit {
        pub tmp: String,
        pub dest: String,
        pub atime_ns: i64,
        pub mtime_ns: i64,
        pub mode: u32,
        pub attrs: u32,
    }
}

record! {
    /// what changed in the shared folders since a sequence number (CDP); overflow: events were
    /// lost, walk everything
    pub struct AgentChanges {
        pub seq: u64,
        pub paths: Vec<String>,
        pub overflow: bool,
    }
}

record! {
    /// a path to remove (replica mirrors: only inside the restore folders)
    pub struct RemovePath {
        pub path: String,
    }
}

record! {
    pub struct RestoreDir {
        pub path: String,
        pub atime_ns: i64,
        pub mtime_ns: i64,
        pub mode: u32,
    }
}

record! {
    pub struct RestoreLink {
        pub path: String,
        pub target: String,
        pub conflict: String,
        /// a link to a folder (Windows tells folder links from file links)
        pub dir: bool,
    }
}

record! {
    /// the agent's periodic sign of life
    pub struct Heartbeat {
        pub version: String,
        pub name: String,
        pub os: Os,
        pub platform: String,
        /// where the server reaches the agent ("host:port")
        pub address: String,
        pub roots: Vec<String>,
        pub restore_roots: Vec<String>,
    }
}
