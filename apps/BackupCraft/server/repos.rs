// repos -- repositories: create or attach (the chunk store in a folder of the server), change
// the name and the immutability (only longer), detach (the data stays), check and clean up
// (operations).  Capacity and free space come from the file system at every listing.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, RepositoryRequest};
use bc_proto::j::{self, Json};
use bc_proto::model::{Job, RepoStatus, Repository, RestorePoint};
use netio::http::Response;

use crate::admin::is_absolute;
use crate::api::Call;
use crate::ctx::{Ctx, C_JOBS, C_POINTS, C_REPOS};
use crate::engine;
use crate::ops::Ops;
use crate::util::{self, all, bad, conflict, err, field, list as list_resp, new_id, no_content, not_found, now, ok, rec};

/// a repository with its current figures (free space, points)
pub fn live(ctx: &Ctx, mut r: Repository) -> Repository {
    match plat::fs::free_space(&r.path) {
        Some((free, total)) if plat::fs::stat(&r.path).is_some_and(|m| m.is_dir()) => {
            r.free = free;
            r.capacity = total;
            if r.status == RepoStatus::Offline {
                r.status = RepoStatus::Ok;
                r.message.clear();
            }
        }
        _ => {
            r.status = RepoStatus::Offline;
            r.message = String::from("Der Ordner ist nicht erreichbar");
        }
    }
    let points: Vec<RestorePoint> = all(&ctx.db, C_POINTS);
    let mine: Vec<&RestorePoint> = points.iter().filter(|p| p.repository_id == r.id).collect();
    r.points = mine.len() as u32;
    r.logical = mine.iter().map(|p| p.size).sum();
    if r.used > 0 {
        r.dedup_x100 = (r.logical.saturating_mul(100) / r.used).min(u32::MAX as u64) as u32;
    }
    if r.status == RepoStatus::Ok && r.capacity > 0 && r.free.saturating_mul(10) < r.capacity {
        r.status = RepoStatus::Warning;
        r.message = String::from("Weniger als 10 % frei");
    }
    r
}

pub fn all_live(ctx: &Ctx) -> Vec<Repository> {
    let mut v: Vec<Repository> = all::<Repository>(&ctx.db, C_REPOS).into_iter().map(|r| live(ctx, r)).collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

pub fn list(ctx: &Ctx) -> Response {
    list_resp(&all_live(ctx))
}

pub fn get(ctx: &Ctx, c: &Call) -> Response {
    match rec::<Repository>(ctx.get(C_REPOS, &c.param("id"))) {
        Some(r) => ok(200, &live(ctx, r)),
        None => not_found("Repository nicht gefunden"),
    }
}

pub fn create(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = RepositoryRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let name = r.name.trim();
    let path = r.path.trim();
    if name.is_empty() {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    if !is_absolute(path) {
        return field(code::BAD_REQUEST, "Bitte einen absoluten Pfad angeben", "path");
    }
    let existing: Vec<Repository> = all(&ctx.db, C_REPOS);
    if existing.iter().any(|x| same_path(&x.path, path)) {
        return field(code::CONFLICT, "Dieses Repository ist schon eingebunden", "path");
    }
    if r.immutable_days > 3650 {
        return field(code::BAD_REQUEST, "Höchstens 3650 Tage", "immutable_days");
    }
    let id = new_id();
    let info = if r.create { engine::repo_init(ctx, path, &id) } else { engine::repo_attach(ctx, path) };
    let info = match info {
        Ok(i) => i,
        Err(e) => return field(code::BAD_REQUEST, &e, "path"),
    };
    let repo = Repository {
        id: if r.create { id } else { info.id.clone() },
        name: String::from(name),
        path: String::from(path),
        status: RepoStatus::Ok,
        encrypted: true,
        immutable_days: r.immutable_days.max(info.immutable_days),
        created: now(),
        used: info.used,
        ..Repository::default()
    };
    if ctx.db.contains(C_REPOS, &repo.id) {
        return field(code::CONFLICT, "Dieses Repository ist schon eingebunden", "path");
    }
    if ctx.db.put(C_REPOS, &repo.id, repo.to_json()).is_err() {
        return util::internal("");
    }
    c.audit(if r.create { "repository.create" } else { "repository.attach" }, name, path);
    ok(201, &live(ctx, repo))
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| String::from(s.trim_end_matches(['/', '\\']));
    if cfg!(windows) { n(a).eq_ignore_ascii_case(&n(b)) } else { n(a) == n(b) }
}

pub fn update(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(mut r): Option<Repository> = rec(ctx.get(C_REPOS, &id)) else { return not_found("Repository nicht gefunden") };
    let name = j::str_of(&c.body, "name").trim();
    if !name.is_empty() {
        r.name = String::from(name);
    }
    if let Some(days) = c.body.get("immutable_days").and_then(|v| v.as_u64()) {
        let days = days.min(3650) as u32;
        if days < r.immutable_days {
            return err(409, code::IMMUTABLE, "Die Unveränderlichkeit kann nur verlängert werden");
        }
        if days != r.immutable_days {
            if let Err(e) = engine::repo_set_immutable(ctx, &r, days) {
                return util::internal(&e);
            }
        }
        r.immutable_days = days;
    }
    if ctx.db.put(C_REPOS, &id, r.to_json()).is_err() {
        return util::internal("");
    }
    c.audit("repository.update", &r.name, &format!("Unveränderlich {} Tage", r.immutable_days));
    ok(200, &live(ctx, r))
}

pub fn remove(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(r): Option<Repository> = rec(ctx.get(C_REPOS, &id)) else { return not_found("Repository nicht gefunden") };
    if all::<Job>(&ctx.db, C_JOBS).iter().any(|j| j.repository_id == id) {
        return conflict("Jobs verwenden dieses Repository noch");
    }
    // the points stay in the repository; the server forgets them
    let points: Vec<String> = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.repository_id == id).map(|p| p.id).collect();
    let _ = ctx.db.write(|_, tx| {
        tx.delete(C_REPOS, &id);
        for p in &points {
            tx.delete(C_POINTS, p);
        }
        Ok::<(), datastore::StoreError>(())
    });
    c.audit("repository.detach", &r.name, "Daten bleiben erhalten");
    no_content()
}

pub fn check(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let id = c.param("id");
    let Some(r): Option<Repository> = rec(ctx.get(C_REPOS, &id)) else { return not_found("Repository nicht gefunden") };
    if ctx.ops.busy(&id, "repository.check") || ctx.ops.busy(&id, "repository.gc") {
        return err(409, code::BUSY, "Für dieses Repository läuft schon ein Vorgang");
    }
    let full = c.body.get("read_data").and_then(|v| v.as_bool()).unwrap_or(false);
    let rr = r.clone();
    let op = Ops::start(ctx, "repository.check", &format!("Repository „{}“ prüfen", r.name), &c.actor(), &id, "", Box::new(move |o| engine::repo_check(o, &rr, full)));
    c.audit("repository.check", &r.name, "");
    util::accepted(&op)
}

pub fn gc(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let id = c.param("id");
    let Some(r): Option<Repository> = rec(ctx.get(C_REPOS, &id)) else { return not_found("Repository nicht gefunden") };
    if ctx.ops.busy(&id, "repository.check") || ctx.ops.busy(&id, "repository.gc") {
        return err(409, code::BUSY, "Für dieses Repository läuft schon ein Vorgang");
    }
    let rr = r.clone();
    let op = Ops::start(ctx, "repository.gc", &format!("Repository „{}“ aufräumen", r.name), &c.actor(), &id, "", Box::new(move |o| engine::repo_gc(o, &rr)));
    c.audit("repository.gc", &r.name, "");
    util::accepted(&op)
}
