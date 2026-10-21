// views -- the pages of the console and their shared helpers
//
//   page(app, ui)         the current page
//   select(app, page, id) select an item on a page (search, links between pages)
//   role_name(role)       the role in words

pub mod creds;
pub mod dashboard;
pub mod dialogs;
pub mod fleet;
pub mod hosts;
pub mod jobs;
pub mod login;
pub mod monitoring;
pub mod recovery;
pub mod replication;
pub mod repos;
pub mod restore;
pub mod settings;
pub mod setup;
pub mod staged;
pub mod tape;

use bc_proto::ids;
use bc_proto::model::Role;
use ui::Ui;

use crate::app::{Console, Page};
use crate::tr::t;

pub fn page(app: &mut Console, ui: &mut Ui) {
    match app.page {
        Page::Dashboard => dashboard::frame(app, ui),
        Page::Jobs => jobs::frame(app, ui),
        Page::Restore => restore::frame(app, ui),
        Page::Hosts => hosts::frame(app, ui),
        Page::Repositories => repos::frame(app, ui),
        Page::Credentials => creds::frame(app, ui),
        Page::Replication => replication::frame(app, ui),
        Page::Tape => tape::frame(app, ui),
        Page::Recovery => recovery::frame(app, ui),
        Page::Monitoring => monitoring::frame(app, ui),
        Page::Settings => settings::frame(app, ui),
    }
}

/// select an item of a page by its id
pub fn select(app: &mut Console, page: Page, id: &str) {
    let k = ids::key(id);
    match page {
        Page::Jobs => app.jobs.grid.select_only(Some(k)),
        Page::Hosts => app.hosts.grid.select_only(Some(k)),
        Page::Repositories => app.repos.sel = alloc::string::String::from(id),
        Page::Restore => app.restore.job = alloc::string::String::from(id),
        Page::Monitoring => monitoring::open_fleet(app, id),
        _ => {}
    }
}

pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Viewer => t("Betrachter"),
        Role::Restore => t("Wiederherstellung"),
        Role::Operator => t("Operator"),
        Role::Admin => t("Administrator"),
    }
}

/// the page body: a vertical box with the page padding (call body_end)
pub fn body_begin(ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, ui::BOX_V | ui::BOX_FIXED | ui::pad(24), 16);
}

pub fn body_end(ui: &mut Ui) {
    ui.box_end();
}

/// a scrolling page body (dashboard, settings)
pub fn scroll_body_begin(ui: &mut Ui, id: u32) {
    ui.next_size(-1, -1);
    ui.scroll_begin(id, ui::SC_V | ui::BOX_FIXED);
    ui.box_begin(0, ui::BOX_V | ui::pad(24), 16);
}

pub fn scroll_body_end(ui: &mut Ui) {
    ui.box_end();
    ui.scroll_end();
}
