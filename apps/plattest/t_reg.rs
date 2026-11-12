// t_reg -- plattest --test reg (Windows, by hand only: it writes to the user's registry):
// plat::inst's keys and values under HKCU\Software\Classes\JFLib-plattest-<random> (no file
// type points at it), removed again (reg_create / set_sz, reg_delete_value, reg_delete_tree: the
// setup's file types); reg_delete_tree refuses a key outside the places of a registration
// (HKCU\Software\JFLib-plattest-<random>, which stays and goes with RegDeleteKeyW)

use crate::T;

#[cfg(windows)]
pub fn t_reg(t: &mut T) {
    let id = plat::time_us() & 0xFFFF_FFFF;
    let base = alloc::format!("Software\\Classes\\JFLib-plattest-{id:x}");
    let sub = alloc::format!("{base}\\shell\\open\\command");
    let ok = plat::inst::reg_create(&sub).map(|k| k.set_sz("", "\"x.exe\" \"%1\"")).is_some()
        && plat::inst::reg_create(&base).map(|k| {
            k.set_sz("", "Test");
            k.set_sz("Extra", "1");
        }).is_some();
    t.check(ok, "reg: keys and values written");
    t.check(plat::inst::reg_delete_value(&base, "Extra"), "reg: one value deleted");
    t.check(!plat::inst::reg_delete_value(&base, "Extra"), "reg: a missing value is reported");
    t.check(!plat::inst::reg_delete(&base), "reg: RegDeleteKeyW refuses a key with subkeys");
    t.check(!plat::inst::reg_delete_tree(&alloc::format!("{base}\\")), "reg: a name with an empty part is refused");
    t.check(plat::inst::reg_delete_tree(&base), "reg: the tree is deleted");
    t.check(!plat::inst::reg_delete_tree(&base), "reg: a missing tree is reported");
    // a key of no registration: refused, it stays
    let outside = alloc::format!("Software\\JFLib-plattest-{id:x}");
    let made = plat::inst::reg_create(&outside).map(|k| k.set_sz("", "Test")).is_some();
    t.check(made && !plat::inst::reg_delete_tree(&outside), "reg: a tree outside Software\\Classes is refused");
    t.check(plat::inst::reg_delete(&outside), "reg: ... and stays (RegDeleteKeyW removes it)");
    plat::inst::assoc_changed();
}

#[cfg(not(windows))]
pub fn t_reg(t: &mut T) {
    t.check(true, "reg: Windows only");
}
