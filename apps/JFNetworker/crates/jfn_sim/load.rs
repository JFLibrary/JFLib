// load -- an engine from a project
//
//   Engine::from_project(&Project) -> Result<Engine, ActErr>
//
// Every device and every cable of the project starts at time 0 (devices that are switched off
// stay off).  Then each device's startup configuration runs through its command line, the
// same parser a user types into (IOS: in global configuration mode, Linux: as shell
// commands); what it does not take is reported on the console and in boot_errors.  A device
// without one starts with its defaults (switch ports in VLAN 1, router interfaces shut down).
// Last, WLAN stations with a network to join start their scan (wlan.rs), on the project's
// positions and scale.  Every cable starts with the length the project gives it (by hand or
// from its places, jfn_model::phys::length) and the delay that follows from that
// (phys::delay_ns: an older project keeps its delays); too long, it stays down (phys.rs).

use jfn_model::Project;

use crate::action::ActErr;
use crate::kernel::Engine;

impl Engine {
    pub fn from_project(p: &Project) -> Result<Engine, ActErr> {
        let mut e = Engine::new(p.seed);
        e.strict_mdix = p.strict_mdix;
        e.wlan.dip_per_m = p.dip_per_m;
        for d in &p.devices {
            e.add_node(d)?;
        }
        let tree = jfn_model::phys::Tree::of(p);
        for l in &p.links {
            let mut x = l.clone();
            x.len_m = jfn_model::phys::length(p, &tree, l).m;
            x.delay_ns = jfn_model::phys::delay_ns(p, &tree, l);
            e.add_link(&x)?;
        }
        for d in &p.devices {
            e.cli_boot(d.id, &d.startup_config);
        }
        e.wlan_boot();
        Ok(e)
    }
}
