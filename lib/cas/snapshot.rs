// snapshot -- what a restore point holds: a tree of nodes per backed-up root, stored as
// chunks (an unchanged directory produces the same tree chunk and is stored once), and the
// snapshot record that names the roots, signed with Ed25519 and encrypted
//
//   snapshot file  "BCN1" | nonce:24 | XChaCha20-Poly1305(snapshot key, aad = snapshot id,
//                  signature:64 ‖ signer public key:32 ‖ JSON of the Snapshot)

use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Value};

use crate::{hex_id, parse_id, CasError, ChunkId};

pub const SNAP_MAGIC: &[u8; 4] = b"BCN1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NodeKind {
    #[default]
    File,
    Dir,
    Symlink,
    Other,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Symlink => "link",
            NodeKind::Other => "other",
        }
    }
    pub fn parse(s: &str) -> NodeKind {
        match s {
            "dir" => NodeKind::Dir,
            "link" => NodeKind::Symlink,
            "other" => NodeKind::Other,
            _ => NodeKind::File,
        }
    }
}

/// one entry of a directory (or a root)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
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
    /// symbolic links: the target as stored in the link
    pub link: String,
    /// files: the content chunks in order
    pub chunks: Vec<ChunkId>,
    /// directories: the tree chunk of their entries
    pub tree: Option<ChunkId>,
    /// a message when the entry could not be read completely (locked file, permission)
    pub error: String,
    /// directories: the files below (with `size` = their bytes); 0 in older snapshots
    pub count: u64,
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(|x| x.as_i64()).unwrap_or(0)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

impl Node {
    pub fn to_json(&self) -> Value {
        let mut o = Value::Object(Vec::new());
        o.insert("n", Value::string(self.name.as_str()));
        o.insert("k", Value::string(self.kind.as_str()));
        if self.size != 0 {
            o.insert("s", Value::u64(self.size));
        }
        o.insert("m", Value::i64(self.mtime_ns));
        if self.atime_ns != 0 {
            o.insert("a", Value::i64(self.atime_ns));
        }
        if self.ctime_ns != 0 {
            o.insert("c", Value::i64(self.ctime_ns));
        }
        if self.mode != 0 {
            o.insert("mode", Value::u64(self.mode as u64));
        }
        if self.uid != 0 || self.gid != 0 {
            o.insert("uid", Value::u64(self.uid as u64));
            o.insert("gid", Value::u64(self.gid as u64));
        }
        if self.attrs != 0 {
            o.insert("attrs", Value::u64(self.attrs as u64));
        }
        if self.ino != 0 {
            o.insert("ino", Value::u64(self.ino));
            o.insert("dev", Value::u64(self.dev));
        }
        if !self.link.is_empty() {
            o.insert("l", Value::string(self.link.as_str()));
        }
        if !self.chunks.is_empty() {
            o.insert("ch", Value::Array(self.chunks.iter().map(|c| Value::string(hex_id(c))).collect()));
        }
        if let Some(t) = &self.tree {
            o.insert("t", Value::string(hex_id(t)));
        }
        if !self.error.is_empty() {
            o.insert("err", Value::string(self.error.as_str()));
        }
        if self.count != 0 {
            o.insert("cnt", Value::u64(self.count));
        }
        o
    }

    pub fn from_json(v: &Value) -> Option<Node> {
        let chunks = match v.get("ch") {
            Some(a) => a.as_array()?.iter().map(|x| x.as_str().and_then(parse_id)).collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let tree = match v.get("t") {
            Some(t) => Some(parse_id(t.as_str()?)?),
            None => None,
        };
        Some(Node {
            name: String::from(s(v, "n")),
            kind: NodeKind::parse(s(v, "k")),
            size: n(v, "s"),
            mtime_ns: i(v, "m"),
            atime_ns: i(v, "a"),
            ctime_ns: i(v, "c"),
            mode: n(v, "mode") as u32,
            uid: n(v, "uid") as u32,
            gid: n(v, "gid") as u32,
            attrs: n(v, "attrs") as u32,
            ino: n(v, "ino"),
            dev: n(v, "dev"),
            link: String::from(s(v, "l")),
            chunks,
            tree,
            error: String::from(s(v, "err")),
            count: n(v, "cnt"),
        })
    }
}

/// the entries of a directory, sorted by name
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn encode(&self) -> Vec<u8> {
        let v = Value::Array(self.nodes.iter().map(Node::to_json).collect());
        json::stringify_with_limits(&v, crate::big_limits()).map(String::into_bytes).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Option<Tree> {
        let v = json::parse_with_limits(b, crate::big_limits()).ok()?;
        let nodes = v.as_array()?.iter().map(Node::from_json).collect::<Option<Vec<_>>>()?;
        Some(Tree { nodes })
    }

    pub fn find(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }
}

/// totals of a snapshot
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapStats {
    pub files: u64,
    pub dirs: u64,
    /// the plain size of all files
    pub bytes: u64,
    /// stored bytes added by this snapshot (after dedupe, compression, encryption)
    pub new_bytes: u64,
    pub new_chunks: u64,
    /// plain bytes of the new chunks
    pub new_raw: u64,
    /// files that could not be read (completely)
    pub errors: u64,
}

/// a restore point
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub created: i64,
    pub job_id: String,
    pub job_name: String,
    pub host: String,
    /// one node per backed-up source; its name is the full source path
    pub roots: Vec<Node>,
    pub parent: String,
    pub full: bool,
    pub stats: SnapStats,
    /// not deletable before this time (seconds)
    pub immutable_until: i64,
    /// the caller's extra data (session id, ...)
    pub meta: Value,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            id: String::new(),
            created: 0,
            job_id: String::new(),
            job_name: String::new(),
            host: String::new(),
            roots: Vec::new(),
            parent: String::new(),
            full: false,
            stats: SnapStats::default(),
            immutable_until: 0,
            meta: Value::Null,
        }
    }
}

impl Snapshot {
    pub fn to_json(&self) -> Value {
        let st = &self.stats;
        let mut o = Value::Object(Vec::new());
        o.insert("id", Value::string(self.id.as_str()));
        o.insert("created", Value::i64(self.created));
        o.insert("job_id", Value::string(self.job_id.as_str()));
        o.insert("job_name", Value::string(self.job_name.as_str()));
        o.insert("host", Value::string(self.host.as_str()));
        o.insert("roots", Value::Array(self.roots.iter().map(Node::to_json).collect()));
        o.insert("parent", Value::string(self.parent.as_str()));
        o.insert("full", Value::Bool(self.full));
        o.insert("stats", Value::object([
            ("files", Value::u64(st.files)),
            ("dirs", Value::u64(st.dirs)),
            ("bytes", Value::u64(st.bytes)),
            ("new_bytes", Value::u64(st.new_bytes)),
            ("new_chunks", Value::u64(st.new_chunks)),
            ("new_raw", Value::u64(st.new_raw)),
            ("errors", Value::u64(st.errors)),
        ]));
        o.insert("immutable_until", Value::i64(self.immutable_until));
        o.insert("meta", self.meta.clone());
        o
    }

    pub fn from_json(v: &Value) -> Option<Snapshot> {
        let st = v.get("stats")?;
        Some(Snapshot {
            id: String::from(s(v, "id")),
            created: i(v, "created"),
            job_id: String::from(s(v, "job_id")),
            job_name: String::from(s(v, "job_name")),
            host: String::from(s(v, "host")),
            roots: v.get("roots")?.as_array()?.iter().map(Node::from_json).collect::<Option<Vec<_>>>()?,
            parent: String::from(s(v, "parent")),
            full: v.get("full").and_then(|x| x.as_bool()).unwrap_or(false),
            stats: SnapStats {
                files: n(st, "files"),
                dirs: n(st, "dirs"),
                bytes: n(st, "bytes"),
                new_bytes: n(st, "new_bytes"),
                new_chunks: n(st, "new_chunks"),
                new_raw: n(st, "new_raw"),
                errors: n(st, "errors"),
            },
            immutable_until: i(v, "immutable_until"),
            meta: v.get("meta").cloned().unwrap_or(Value::Null),
        })
    }
}

/// seal a snapshot (signed by `signer`)
pub fn seal(key: &[u8; 32], signer: &crypto::ed25519::Keypair, snap: &Snapshot) -> Result<Vec<u8>, CasError> {
    let text = json::stringify_with_limits(&snap.to_json(), crate::big_limits()).map_err(|_| CasError::TooLarge)?;
    let sig = signer.sign(text.as_bytes());
    let mut plain = Vec::with_capacity(96 + text.len());
    plain.extend_from_slice(&sig);
    plain.extend_from_slice(&signer.public);
    plain.extend_from_slice(text.as_bytes());
    let nonce: [u8; 24] = plat::random_bytes();
    let mut out = Vec::from(&SNAP_MAGIC[..]);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&crypto::aead::x_seal(key, &nonce, snap.id.as_bytes(), &plain));
    Ok(out)
}

/// open a snapshot file: the snapshot and its signer (the signature is checked; whether the
/// signer is trusted is the caller's decision)
pub fn open(key: &[u8; 32], id: &str, b: &[u8]) -> Result<(Snapshot, [u8; 32]), CasError> {
    if b.len() < 4 + 24 + 16 + 96 || &b[..4] != SNAP_MAGIC {
        return Err(CasError::Corrupt(String::from("not a snapshot")));
    }
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&b[4..28]);
    let plain = crypto::aead::x_open(key, &nonce, id.as_bytes(), &b[28..]).ok_or_else(|| CasError::Corrupt(String::from("snapshot does not authenticate")))?;
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&plain[..64]);
    let mut signer = [0u8; 32];
    signer.copy_from_slice(&plain[64..96]);
    let text = &plain[96..];
    if !crypto::ed25519::verify(&signer, text, &sig) {
        return Err(CasError::Corrupt(String::from("snapshot signature invalid")));
    }
    let v = json::parse_with_limits(text, crate::big_limits()).map_err(|_| CasError::Corrupt(String::from("snapshot unreadable")))?;
    let snap = Snapshot::from_json(&v).ok_or_else(|| CasError::Corrupt(String::from("snapshot fields")))?;
    if snap.id != id {
        return Err(CasError::Corrupt(String::from("snapshot id mismatch")));
    }
    Ok((snap, signer))
}
