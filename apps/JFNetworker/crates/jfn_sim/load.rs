// load -- an engine from a project
//
//   Engine::from_project(&Project) -> Result<Engine, ActErr>
//
// Every device and every cable of the project starts at time 0 (devices that are switched off
// stay off).  Then each device's startup configuration runs through its command line, the
// same parser a user types into (IOS: in global configuration mode, Linux: as shell
// commands); what it does not take is reported on the console and in boot_errors.  A device
// without one starts with its defaults (switch ports in VLAN 1, router interfaces shut down).

use jfn_model::Project;

use crate::action::ActErr;
use crate::kernel::Engine;

impl Engine {
    pub fn from_project(p: &Project) -> Result<Engine, ActErr> {
        let mut e = Engine::new(p.seed);
        e.strict_mdix = p.strict_mdix;
        for d in &p.devices {
            e.add_node(d)?;
        }
        for l in &p.links {
            e.add_link(l)?;
        }
        for d in &p.devices {
            e.cli_boot(d.id, &d.startup_config);
        }
        Ok(e)
    }
}
