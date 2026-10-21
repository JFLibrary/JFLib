// t_tray -- the symbol in the notification area and system notifications (lib/plat/tray.rs)
//
//   plattest --test tray       the contract in the dry run (jfgui_TRAY_DRYRUN=1: printed, not
//                              shown): set, update, notify ids, close, remove, no events
//   plattest --test tray-bus   (Linux; tools/platshared/tray_bus.py starts it on a fake session
//                              bus) a symbol with a menu and a notification; the fake bus clicks
//                              the notification, activates the symbol and picks the menu row 2:
//                              NoteClicked, Activate and Menu(2) must arrive within 10 s
//   plattest --test tray-win   (Windows, by hand: the symbol appears for a moment) the real
//                              Shell_NotifyIconW calls: add, update, remove

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::tray::{self, Note, TI_CHECKED, TI_SEPARATOR, Tray, TrayEvent, TrayIcon, TrayItem};

use crate::T;

fn item(id: u32, label: &str, flags: u32) -> TrayItem {
    TrayItem { id, label: String::from(label), flags, sub: Vec::new() }
}

fn sample() -> Tray {
    let px = vec![0xFF1E_78E6u32; 16 * 16];
    Tray {
        icon: Some(TrayIcon { w: 16, h: 16, px }),
        tooltip: String::from("plattest"),
        menu: vec![
            item(1, "&Öffnen", 0),
            item(0, "", TI_SEPARATOR),
            TrayItem { id: 0, label: String::from("Mehr"), flags: 0, sub: vec![item(3, "Sub_eins", TI_CHECKED)] },
            item(2, "Beenden", 0),
        ],
    }
}

pub fn t_tray(t: &mut T) {
    plat::setenv("jfgui_TRAY_DRYRUN", Some("1"));
    t.check(tray::tray_supported(), "tray: supported in the dry run");
    t.check(tray::tray_set(&sample()), "tray: set");
    t.check(tray::tray_set(&Tray { tooltip: String::from("neu"), ..sample() }), "tray: update");
    let a = tray::notify(&Note { title: String::from("Titel"), body: String::from("Text"), icon: None });
    let b = tray::notify(&Note { title: String::from("Zwei"), body: String::new(), icon: None });
    t.check(a != 0 && b != 0 && a != b, "tray: notifications get their own ids");
    tray::notify_close(a);
    tray::tray_remove();
    t.check(tray::tray_take().is_empty(), "tray: no events without a user");
    plat::setenv("jfgui_TRAY_DRYRUN", None);
}

// wait up to 10 s until every event of want arrived
fn wait_for(want: &[TrayEvent]) -> Vec<TrayEvent> {
    let mut got = Vec::new();
    for _ in 0..1000 {
        got.extend(tray::tray_take());
        if want.iter().all(|w| got.contains(w)) {
            break;
        }
        plat::sleep_ms(10);
    }
    got
}

pub fn t_tray_bus(t: &mut T) {
    t.check(tray::tray_supported(), "tray-bus: a session bus is announced");
    t.check(tray::tray_set(&sample()), "tray-bus: set");
    let id = tray::notify(&Note { title: String::from("Hallo"), body: String::from("vom Test"), icon: Some(TrayIcon { w: 2, h: 1, px: vec![0xFF00_00FF, 0x8000_FF00] }) });
    t.check(id != 0, "tray-bus: notify");
    let got = wait_for(&[TrayEvent::NoteClicked(id), TrayEvent::Activate, TrayEvent::Menu(2)]);
    t.check(got.contains(&TrayEvent::NoteClicked(id)), "tray-bus: the notification was clicked");
    t.check(got.contains(&TrayEvent::Activate), "tray-bus: the symbol was activated");
    t.check(got.contains(&TrayEvent::Menu(2)), "tray-bus: menu row 2 was chosen");
    t.check(!got.iter().any(|e| matches!(e, TrayEvent::NoteClosed(_))), "tray-bus: a clicked notification is not reported closed");
    tray::tray_remove();
    plat::sleep_ms(300); // the release of the name reaches the bus
    crate::out("tray-bus: done\n");
}

pub fn t_tray_win(t: &mut T) {
    t.check(tray::tray_supported(), "tray-win: Shell_NotifyIconW is there");
    t.check(tray::tray_set(&sample()), "tray-win: the symbol is added");
    t.check(tray::tray_set(&Tray { tooltip: String::from("plattest (2)"), ..sample() }), "tray-win: updated");
    t.check(tray::tray_set(&Tray { icon: None, ..sample() }), "tray-win: the program's own icon");
    tray::tray_remove();
    t.check(tray::tray_take().is_empty(), "tray-win: no events without a user");
}
