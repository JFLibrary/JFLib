// fetch -- the updater's HTTP(S): small files with redirects, and downloads that hash while
// they write and resume an interrupted file with a Range request
//
//   join(base, href) -> Option<Url>   a manifest's file address
//   get(cfg, url, max) -> (final url, bytes)
//   download(cfg, url, part, size, sha256, progress) -> Ok when part holds the whole file

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};

use netio::{Client, Headers, HttpError, Part, Url};
use plat::net::NetErrorKind;

use crate::{Config, Error};

const MAX_REDIRECTS: usize = 5;

/// the counters a download updates and the flag that stops it
pub struct Progress<'a> {
    pub done: &'a AtomicU64,
    pub total: &'a AtomicU64,
    pub cancel: &'a AtomicBool,
}

fn client(cfg: &Config) -> Client {
    let extra = netio::pem::decode_all(&cfg.ca_pem, "CERTIFICATE").iter().filter_map(|d| netio::x509::Cert::parse(d)).collect();
    let mut c = Client::new().with_connector(netio::trust::system_connector(extra));
    c.timeout_ms = cfg.timeout_ms.max(1000);
    c.connect_ms = c.timeout_ms.min(10_000);
    c.user_agent = format!("{}/{} JFLib-update", cfg.app, cfg.version);
    c
}

fn net_err(url: &Url, e: HttpError) -> Error {
    match e {
        HttpError::Net(n) if n.kind == NetErrorKind::Denied => Error::Denied(url.host.clone()),
        e => Error::Net(format!("{}: {}", url, e)),
    }
}

/// href relative to base: an absolute http(s) address, "//host/..", "/path" or a name
/// relative to base's folder
pub fn join(base: &Url, href: &str) -> Option<Url> {
    if href.contains("://") {
        return Url::parse(href);
    }
    if let Some(rest) = href.strip_prefix("//") {
        return Url::parse(&format!("{}://{}", base.scheme, rest));
    }
    if href.is_empty() || href.chars().any(|c| c.is_control() || c == ' ') {
        return None;
    }
    let target = if href.starts_with('/') {
        String::from(href)
    } else {
        let path = base.target.split(['?', '#']).next().unwrap_or("/");
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        format!("{}{}", if dir.is_empty() { "/" } else { dir }, href)
    };
    Some(base.with_target(&target))
}

fn redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// a small file (at most max bytes), redirects followed -> the address it came from and its
/// bytes
pub fn get(cfg: &Config, url: &Url, max: usize) -> Result<(Url, Vec<u8>), Error> {
    let mut c = client(cfg);
    c.max_body = max;
    let mut u = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        let r = c.get(&u, &Headers::new()).map_err(|e| net_err(&u, e))?;
        if r.status == 200 {
            return Ok((u, Vec::from(r.body_bytes())));
        }
        if !redirect(r.status) {
            return Err(Error::Http(r.status));
        }
        u = r.headers.get("location").and_then(|l| join(&u, l.trim())).ok_or(Error::Http(r.status))?;
    }
    Err(Error::Net(format!("{}: zu viele Umleitungen", url)))
}

// "bytes START-END/TOTAL" -> (START, TOTAL or None for "*")
fn content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let r = v.trim().strip_prefix("bytes")?.trim_start();
    let (span, total) = r.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    let total = if total.trim() == "*" { None } else { Some(total.trim().parse().ok()?) };
    Some((start.trim().parse().ok()?, total))
}

fn write_all(f: &plat::File, mut off: u64, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        match f.write_at(off, data) {
            Some(n) if n > 0 => {
                off += n as u64;
                data = &data[n..];
            }
            _ => return false,
        }
    }
    true
}

/// download url into the file part: a part that already holds the start of the file is
/// hashed and continued with a Range request (a server that answers 200 instead starts it
/// over).  The answer must have exactly `size` bytes with this SHA-256; a wrong file is
/// deleted (Error::Size / Error::Hash), an interrupted one kept (Error::Incomplete).
pub fn download(cfg: &Config, url: &Url, part: &str, size: u64, sha256: &[u8; 32], p: &Progress) -> Result<(), Error> {
    let f = plat::File::open(part, plat::FO_RW).ok_or_else(|| Error::Io(format!("{} kann nicht geöffnet werden", part)))?;
    let mut have = f.size().unwrap_or(0);
    if have > size {
        f.set_len(0);
        have = 0;
    }
    // the hash of what is there
    let mut h = crypto::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut off = 0;
    while off < have {
        let want = (have - off).min(buf.len() as u64) as usize;
        match f.read_at(off, &mut buf[..want]) {
            Some(n) if n > 0 => {
                h.update(&buf[..n]);
                off += n as u64;
            }
            _ => {
                f.set_len(0);
                have = 0;
                h = crypto::Sha256::new();
                break;
            }
        }
    }
    p.total.store(size, SeqCst);
    p.done.store(have, SeqCst);
    let c = client(cfg);
    let mut u = url.clone();
    let mut hops = 0;
    while have < size {
        if p.cancel.load(SeqCst) {
            return Err(Error::Cancelled);
        }
        let mut headers = Headers::new();
        if have > 0 {
            headers.set("Range", &format!("bytes={}-", have));
        }
        let mut status = 0u16;
        let mut location: Option<String> = None;
        let mut bad: Option<Error> = None;
        let mut pos = have;
        let r = c.get_to(&u, &headers, size, &mut |piece| match piece {
            Part::Head(st, hd) => {
                status = st;
                match st {
                    200 => {
                        if hd.get("content-length").and_then(|v| v.trim().parse::<u64>().ok()).is_some_and(|n| n != size) {
                            bad = Some(Error::Size { expected: size, got: hd.get("content-length").and_then(|v| v.trim().parse().ok()).unwrap_or(0) });
                            return false;
                        }
                        if pos > 0 {
                            // the server ignored the Range: from the start
                            pos = 0;
                            h = crypto::Sha256::new();
                            f.set_len(0);
                        }
                        true
                    }
                    206 => match hd.get("content-range").and_then(content_range) {
                        Some((start, total)) if start == pos && total.is_none_or(|t| t == size) => true,
                        _ => {
                            bad = Some(Error::Net(format!("{}: falscher Content-Range", u)));
                            false
                        }
                    },
                    s if redirect(s) => {
                        location = hd.get("location").map(String::from);
                        false
                    }
                    _ => false,
                }
            }
            Part::Data(d) => {
                if p.cancel.load(SeqCst) {
                    return false;
                }
                if pos + d.len() as u64 > size {
                    bad = Some(Error::Size { expected: size, got: pos + d.len() as u64 });
                    return false;
                }
                if !write_all(&f, pos, d) {
                    bad = Some(Error::Io(format!("{} kann nicht geschrieben werden", part)));
                    return false;
                }
                h.update(d);
                pos += d.len() as u64;
                p.done.store(pos, SeqCst);
                true
            }
        });
        if let Some(e) = bad {
            if matches!(e, Error::Size { .. }) {
                drop(f);
                plat::delete(part);
            }
            return Err(e);
        }
        if p.cancel.load(SeqCst) {
            return Err(Error::Cancelled);
        }
        match r {
            Err(e) if pos == have => return Err(net_err(&u, e)),
            Err(_) => {}
            Ok(_) if redirect(status) => {
                hops += 1;
                u = location.as_deref().and_then(|l| join(&u, l.trim())).filter(|_| hops <= MAX_REDIRECTS).ok_or(Error::Http(status))?;
                continue;
            }
            Ok(_) if status != 200 && status != 206 => return Err(Error::Http(status)),
            Ok(_) => {}
        }
        if pos < size {
            // the connection ended early: keep what arrived for the next attempt
            f.sync();
            return Err(Error::Incomplete { done: pos, total: size });
        }
        have = pos;
    }
    f.sync();
    if !crypto::ct_eq(&h.finalize(), sha256) {
        drop(f);
        plat::delete(part);
        return Err(Error::Hash);
    }
    Ok(())
}
