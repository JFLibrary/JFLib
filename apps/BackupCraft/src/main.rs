// BackupCraft -- the console: manages BackupCraft servers (jobs, restore, servers and agents,
// repositories, monitoring, settings).  The server, the agent and bcctl are further
// executables of this app (BINS); they share bc_proto.
//
//   python build_rs.py run BackupCraft -- --fixture          demo data, no server needed
//   build/win/BackupCraft.exe --server https://backup.example:7642
//   build/win/BackupCraft.exe --fixture --page jobs --shot jobs.png --size 1440x900
//
// Files: app.rs (state, pages, frame, commands), net.rs (requests, polling, typed data),
// fixture.rs (an in-process demo server), live.rs (the HTTP connection), shell.rs (title bar,
// navigation, activity bar), views/ (the pages), wizards/ (job, restore, repository, agent,
// first run), widgets/ (schedule editor, remote file picker, inspector parts), fmt.rs,
// icons.rs (domain icons for every style), tr.rs + locales/ui.tsv (German / English).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod app;
mod fixture;
mod fmt;
mod icons;
mod live;
mod net;
mod shell;
mod tr;
mod views;
mod widgets;
mod wizards;

use ui::*;

const DESC: AppDesc = AppDesc {
    title: "BackupCraft",
    flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR,
    w: 1440,
    h: 900,
    min_w: 1024,
    min_h: 640,
    ..AppDesc::NONE
};

ui::main!(ui::launch(&DESC, app::Console::new(), &style::STYLE));
