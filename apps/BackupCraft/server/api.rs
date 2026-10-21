// api -- the router: the route table of bc_proto (one place for paths, roles, staged
// features), access checks, the setup gate, audit of every request that changes something,
// and the dispatch to the handlers
//
// Order of the checks: route (404 / 405) -> staged capability (501) -> setup gate (503
// setup_required) -> access (401 / 403) -> JSON body (400) -> handler.  Every request that is
// not a GET leaves an audit entry; handlers name the action and its target, otherwise the
// route pattern is used.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;

use bc_proto::api::{self, code, Access, Method, Route};
use bc_proto::j::{self, Value};
use bc_proto::model::User;
use netio::http::{Request, Response};

use crate::ctx::Ctx;
use crate::util::err;
use crate::{admin, auth, cdp, fleet, hosts, jobs, monitor, notify, replica, repos, tape, update, vm, volume};

/// one API call as the handlers see it
pub struct Call<'a> {
    pub req: &'a Request,
    pub route: &'static Route,
    pub params: Vec<(&'static str, String)>,
    pub body: Value,
    pub user: Option<User>,
    pub token: Option<String>,
    /// the agent host proved by the TLS client certificate
    pub agent: Option<String>,
    audit: RefCell<Option<(String, String, String, String)>>,
}

impl Call<'_> {
    pub fn param(&self, name: &str) -> String {
        self.params.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    pub fn query(&self, key: &str) -> Option<String> {
        self.req.query(key)
    }

    /// who acts (user name, "agent:<id>" or "anonym")
    pub fn actor(&self) -> String {
        if let Some(u) = &self.user {
            return u.username.clone();
        }
        if let Some(a) = &self.agent {
            return alloc::format!("agent:{}", a);
        }
        String::from("anonym")
    }

    /// the audit entry of this call
    pub fn audit(&self, action: &str, target: &str, detail: &str) {
        *self.audit.borrow_mut() = Some((self.actor(), String::from(action), String::from(target), String::from(detail)));
    }

    /// the audit entry with an explicit actor (login, setup)
    pub fn audit_as(&self, actor: &str, action: &str, target: &str, detail: &str) {
        *self.audit.borrow_mut() = Some((String::from(actor), String::from(action), String::from(target), String::from(detail)));
    }
}

/// answer one HTTP request
pub fn handle(ctx: &Arc<Ctx>, req: &Request) -> Response {
    let Some(method) = Method::parse(&req.method) else {
        return if req.method == "HEAD" || req.method == "OPTIONS" { Response::new(405).header("Allow", "GET, POST, PUT, DELETE") } else { err(405, code::BAD_REQUEST, "Methode nicht erlaubt") };
    };
    let Some((route, params)) = api::route_match(method.as_str(), &req.target) else {
        return if api::path_known(&req.target) { err(405, code::BAD_REQUEST, "Methode nicht erlaubt") } else { err(404, code::NOT_FOUND, "Unbekannter Pfad") };
    };
    let params: Vec<(&'static str, String)> = params.into_iter().map(|(k, v)| (k, bc_proto::ids::percent_decode(v))).collect();
    if !route.capability.is_empty() && !api::available(route.capability) {
        return err(501, code::CAPABILITY, "Diese Funktion ist in dieser Version noch nicht verfügbar");
    }
    let public_before_setup = matches!(route.path, "/health" | "/api" | "/api/setup");
    if !ctx.is_set_up() && !public_before_setup {
        return err(503, code::SETUP_REQUIRED, "Der Server ist noch nicht eingerichtet");
    }
    let token = req.bearer().map(String::from);
    let mut user = None;
    let mut agent = None;
    match route.access {
        Access::Public => {
            user = token.as_deref().and_then(|t| auth::user_of_token(ctx, t));
        }
        Access::Role(need) => match token.as_deref().and_then(|t| auth::user_of_token(ctx, t)) {
            None => return err(401, code::UNAUTHORIZED, "Bitte erneut anmelden"),
            Some(u) if !u.role.allows(need) => return err(403, code::FORBIDDEN, "Dafür fehlt die Berechtigung"),
            Some(u) => user = Some(u),
        },
        Access::Agent => match &req.client_id {
            Some(id) if ctx.db.contains(crate::ctx::C_HOSTS, id) => agent = Some(id.clone()),
            _ => return err(401, code::UNAUTHORIZED, "Nur für gekoppelte Agenten"),
        },
        Access::Fleet => {
            if !token.as_deref().is_some_and(|t| fleet::token_ok(ctx, t)) {
                return err(401, code::UNAUTHORIZED, "Nur mit einem Überwachungs-Token");
            }
        }
    }
    let body = if req.body.is_empty() {
        Value::Null
    } else {
        match j::parse(&req.body) {
            Some(v) => v,
            None => return err(400, code::BAD_REQUEST, "Der Inhalt ist kein gültiges JSON"),
        }
    };
    let call = Call { req, route, params, body, user, token, agent, audit: RefCell::new(None) };
    let resp = dispatch(ctx, &call);
    if method != Method::Get {
        let ok = resp.status < 400;
        let (actor, action, target, detail) = call.audit.borrow_mut().take().unwrap_or_else(|| {
            (call.actor(), alloc::format!("{} {}", method.as_str(), route.path), call.param("id"), String::new())
        });
        let detail = if ok || !detail.is_empty() { detail } else { error_text(&resp) };
        ctx.audit(&actor, &action, &target, &detail, ok);
    }
    resp
}

/// the message of an error answer (for the audit)
fn error_text(r: &Response) -> String {
    j::parse(r.body_bytes()).map(|v| String::from(j::str_of(&v, "message"))).unwrap_or_default()
}

fn dispatch(ctx: &Arc<Ctx>, c: &Call) -> Response {
    use Method::*;
    match (c.route.method, c.route.path) {
        (Get, "/health") => crate::util::json(200, &j::obj([("status", Value::string("ok"))])),
        (Get, "/api") => admin::info(ctx),
        (Post, "/api/setup") => auth::setup(ctx, c),
        (Post, "/api/auth/login") => auth::login(ctx, c),
        (Post, "/api/auth/logout") => auth::logout(ctx, c),
        (Get, "/api/auth/me") => auth::me(ctx, c),
        (Get, "/api/summary") => monitor::summary(ctx),
        (Get, "/api/capabilities") => admin::capabilities(),
        (Get, "/api/users") => auth::users(ctx, c),
        (Post, "/api/users") | (Put, "/api/users/{id}") => auth::user_save(ctx, c),
        (Delete, "/api/users/{id}") => auth::user_delete(ctx, c),
        (Post, "/api/users/me/password") => auth::change_password(ctx, c),
        (Get, "/api/jobs") => jobs::list(ctx),
        (Post, "/api/jobs") | (Put, "/api/jobs/{id}") => jobs::save(ctx, c),
        (Get, "/api/jobs/{id}") => jobs::get(ctx, c),
        (Delete, "/api/jobs/{id}") => jobs::delete(ctx, c),
        (Post, "/api/jobs/{id}/enable") | (Post, "/api/jobs/{id}/disable") => jobs::enable(ctx, c),
        (Post, "/api/jobs/{id}/run") => jobs::run(ctx, c),
        (Get, "/api/jobs/{id}/points") => jobs::points(ctx, c),
        (Get, "/api/points/{id}") => jobs::point(ctx, c),
        (Get, "/api/points/{id}/browse") => jobs::point_browse(ctx, c),
        (Post, "/api/points/{id}/restore") => jobs::restore(ctx, c),
        (Post, "/api/points/{id}/verify") => jobs::verify(ctx, c),
        (Post, "/api/points/{id}/tape") => tape::point_write(ctx, c),
        (Get, "/api/hosts/{id}/volumes") => volume::volumes(ctx, c),
        (Get, "/api/hypervisors") => vm::status(ctx),
        (Get, "/api/vms") => vm::list_vms(ctx),
        (Get, "/api/instant-start") => vm::instant_list(ctx),
        (Post, "/api/points/{id}/instant-start") => vm::instant_start(ctx, c),
        (Get, "/api/instant-start/{id}/console") => vm::instant_console(ctx, c),
        (Post, "/api/instant-start/{id}/stop") => vm::instant_stop(ctx, c),
        (Post, "/api/instant-start/{id}/migrate") => vm::instant_migrate(ctx, c),
        (Get, "/api/boot-checks") => vm::bootcheck_list(ctx),
        (Post, "/api/points/{id}/boot-check") => vm::bootcheck_start(ctx, c),
        (Get, "/api/tape") => tape::status(ctx),
        (Get, "/api/tape/tapes") => tape::tapes(ctx),
        (Post, "/api/tape/tapes") => tape::tapes_add(ctx, c),
        (Put, "/api/tape/tapes/{id}") => tape::tape_update(ctx, c),
        (Delete, "/api/tape/tapes/{id}") => tape::tape_delete(ctx, c),
        (Post, "/api/tape/tapes/{id}/erase") => tape::start_erase(ctx, c),
        (Post, "/api/tape/rescan") => tape::start_rescan(ctx, c),
        (Get, "/api/tape/pools") => tape::pools(ctx),
        (Post, "/api/tape/pools") | (Put, "/api/tape/pools/{id}") => tape::pool_save(ctx, c),
        (Delete, "/api/tape/pools/{id}") => tape::pool_delete(ctx, c),
        (Get, "/api/tape/sets") => tape::sets(ctx, c),
        (Get, "/api/tape/sets/{id}/browse") => tape::set_browse(ctx, c),
        (Post, "/api/tape/sets/{id}/restore") => tape::start_set_restore(ctx, c),
        (Post, "/api/tape/sets/{id}/verify") => tape::start_set_verify(ctx, c),
        (Delete, "/api/points/{id}") => jobs::point_delete(ctx, c),
        (Get, "/api/sessions") => monitor::sessions(ctx, c),
        (Get, "/api/sessions/{id}") => monitor::session(ctx, c),
        (Get, "/api/sessions/{id}/log") => monitor::session_log(ctx, c),
        (Get, "/api/operations") => monitor::operations(ctx),
        (Get, "/api/operations/{id}") => monitor::operation(ctx, c),
        (Post, "/api/operations/{id}/cancel") => monitor::cancel(ctx, c),
        (Get, "/api/audit") => monitor::audit(ctx, c),
        (Get, "/api/hosts") => hosts::list(ctx),
        (Get, "/api/hosts/{id}") => hosts::get(ctx, c),
        (Put, "/api/hosts/{id}") => hosts::update(ctx, c),
        (Delete, "/api/hosts/{id}") => hosts::remove(ctx, c),
        (Get, "/api/hosts/{id}/browse") => hosts::browse(ctx, c),
        (Post, "/api/hosts/{id}/check") => hosts::check(ctx, c),
        (Post, "/api/hosts/pairing") => hosts::pairing(ctx, c),
        (Post, "/api/hosts/ssh-push") => hosts::ssh_push(ctx, c),
        (Post, "/api/hosts/ssh-fingerprint") => hosts::ssh_fingerprint(ctx, c),
        (Post, "/api/agent/enroll") => hosts::enroll(ctx, c),
        (Post, "/api/agent/heartbeat") => hosts::heartbeat(ctx, c),
        (Get, "/api/repositories") => repos::list(ctx),
        (Post, "/api/repositories") => repos::create(ctx, c),
        (Get, "/api/repositories/{id}") => repos::get(ctx, c),
        (Put, "/api/repositories/{id}") => repos::update(ctx, c),
        (Delete, "/api/repositories/{id}") => repos::remove(ctx, c),
        (Post, "/api/repositories/{id}/check") => repos::check(ctx, c),
        (Post, "/api/repositories/{id}/gc") => repos::gc(ctx, c),
        (Get, "/api/credentials") => admin::credentials(ctx),
        (Post, "/api/credentials") => admin::credential_save(ctx, c),
        (Delete, "/api/credentials/{id}") => admin::credential_delete(ctx, c),
        (Get, "/api/settings") => admin::settings(ctx),
        (Put, "/api/settings") => admin::settings_save(ctx, c),
        (Get, "/api/recovery") => admin::recovery(ctx),
        (Post, "/api/recovery/kit") => admin::recovery_kit(ctx, c),
        (Get, "/api/config/backups") => admin::config_backups(ctx),
        (Post, "/api/config/backups") => admin::config_backup(ctx, c),
        (Post, "/api/config/restore") => admin::config_restore(ctx, c),
        (Post, "/api/settings/test-email") => notify::test(ctx, c),
        (Get, "/api/notify") => notify::status(ctx),
        (Post, "/api/notify/summary") => notify::summary_now(ctx, c),
        (Get, "/api/cdp") => cdp::status(ctx),
        (Get, "/api/fleet") => fleet::list_servers(ctx),
        (Post, "/api/fleet/probe") => fleet::probe(ctx, c),
        (Post, "/api/fleet/servers") | (Put, "/api/fleet/servers/{id}") => fleet::save(ctx, c),
        (Delete, "/api/fleet/servers/{id}") => fleet::delete(ctx, c),
        (Post, "/api/fleet/servers/{id}/check") => fleet::check_now(ctx, c),
        (Get, "/api/fleet/tokens") => fleet::tokens(ctx),
        (Post, "/api/fleet/tokens") => fleet::token_create(ctx, c),
        (Delete, "/api/fleet/tokens/{id}") => fleet::token_delete(ctx, c),
        (Get, "/api/fleet/report") => fleet::report(ctx),
        (Get, "/api/replicas") => replica::list(ctx),
        (Post, "/api/replicas") | (Put, "/api/replicas/{id}") => replica::save(ctx, c),
        (Delete, "/api/replicas/{id}") => replica::delete(ctx, c),
        (Post, "/api/replicas/{id}/sync") | (Post, "/api/replicas/{id}/undo") => replica::sync_now(ctx, c),
        (Post, "/api/replicas/{id}/failover") => replica::failover(ctx, c),
        (Post, "/api/replicas/{id}/failback") => replica::failback_now(ctx, c),
        (Get, "/api/failover-plans") => replica::plans(ctx),
        (Post, "/api/failover-plans") | (Put, "/api/failover-plans/{id}") => replica::plan_save(ctx, c),
        (Delete, "/api/failover-plans/{id}") => replica::plan_delete(ctx, c),
        (Post, "/api/failover-plans/{id}/run") => replica::plan_run(ctx, c),
        (Get, "/api/update") => update::status(ctx),
        (Post, "/api/update/check") => update::check(ctx, c),
        (Post, "/api/update/download") => update::download(ctx, c),
        _ => err(501, code::CAPABILITY, "Noch nicht umgesetzt"),
    }
}
