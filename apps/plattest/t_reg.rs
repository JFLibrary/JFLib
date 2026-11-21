// t_reg -- plattest --test reg (Windows, by hand only: it writes to the user's registry):
// plat::inst's keys and values under HKCU\Software\Classes\JFLib-plattest-<random> (no file
// type points at it), removed again (reg_create / set_sz, reg_delete_value, reg_delete_tree: the
// setup's file types); reg_delete_tree refuses a key outside the places of a registration
// (HKCU\Software\JFLib-plattest-<random>, which stays and goes with RegDeleteKeyW).  With
// administrator rights the same below HKLM (reg_*_in(Root::Machine): a setup for all users),
// else that part is skipped

use crate::T;

#[cfg(windows)]
pub fn t_reg(t: &mut T) {
    use plat::inst::Root;
    // the functions of HKCU are those of Root::User
    let id = plat::time_us() & 0xFFFF_FFFF;
    let base = alloc::format!("Software\\Classes\\JFLib-plattest-{id:x}");
    let ok = plat::inst::reg_create(&base).map(|k| k.set_sz("", "Test")).is_some();
    t.check(ok && plat::inst::reg_delete_tree_in(Root::User, &base), "reg: reg_create and reg_delete_tree_in(Root::User) name the same key");
    cycle(t, Root::User, "HKCU");
    if plat::process::elevated() {
        cycle(t, Root::Machine, "HKLM");
    } else {
        t.info("  reg: HKLM skipped (no administrator rights)\n");
    }
    plat::inst::assoc_changed();
}

/// write, read back by deleting, refuse: the keys a setup writes, below root
#[cfg(windows)]
fn cycle(t: &mut T, root: plat::inst::Root, name: &str) {
    use plat::inst::{reg_create_in, reg_delete_in, reg_delete_tree_in, reg_delete_value_in};
    let id = plat::time_us() & 0xFFFF_FFFF;
    let base = alloc::format!("Software\\Classes\\JFLib-plattest-{id:x}");
    let sub = alloc::format!("{base}\\shell\\open\\command");
    let ok = reg_create_in(root, &sub).map(|k| k.set_sz("", "\"x.exe\" \"%1\"")).is_some()
        && reg_create_in(root, &base).map(|k| {
            k.set_sz("", "Test");
            k.set_sz("Extra", "1");
        }).is_some();
    t.check(ok, &alloc::format!("reg {name}: keys and values written"));
    t.check(reg_delete_value_in(root, &base, "Extra"), &alloc::format!("reg {name}: one value deleted"));
    t.check(!reg_delete_value_in(root, &base, "Extra"), &alloc::format!("reg {name}: a missing value is reported"));
    t.check(!reg_delete_in(root, &base), &alloc::format!("reg {name}: RegDeleteKeyW refuses a key with subkeys"));
    t.check(!reg_delete_tree_in(root, &alloc::format!("{base}\\")), &alloc::format!("reg {name}: a name with an empty part is refused"));
    t.check(reg_delete_tree_in(root, &base), &alloc::format!("reg {name}: the tree is deleted"));
    t.check(!reg_delete_tree_in(root, &base), &alloc::format!("reg {name}: a missing tree is reported"));
    // a key of no registration: refused, it stays
    let outside = alloc::format!("Software\\JFLib-plattest-{id:x}");
    let made = reg_create_in(root, &outside).map(|k| k.set_sz("", "Test")).is_some();
    t.check(made && !reg_delete_tree_in(root, &outside), &alloc::format!("reg {name}: a tree outside Software\\Classes is refused"));
    t.check(reg_delete_in(root, &outside), &alloc::format!("reg {name}: ... and stays (RegDeleteKeyW removes it)"));
}

#[cfg(not(windows))]
pub fn t_reg(t: &mut T) {
    t.check(true, "reg: Windows only");
}
