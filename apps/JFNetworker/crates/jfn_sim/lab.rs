// lab -- the actions of the distributed-systems lab and what its user interface reads
//
//   Engine::lab_apply(Action)      Fault, Heal, HealAll (fault.rs); LbBackend, LbAlgorithm
//                                  (lb.rs); KvSet (kv.rs); CacheClear (cache.rs); WorkerSet
//                                  (mq.rs)
//
// The services of the lab live in their own files (lb.rs, kv.rs, cache.rs, mq.rs); this one
// routes the actions to the service of the right kind (a stopped one is changed as well).

use alloc::format;
use alloc::string::String;

use crate::action::{ActErr, Action};
use crate::kernel::Engine;
use crate::os::Service;

impl Engine {
    /// a running service of type T by name: (node slot, service id)
    fn lab_svc<T: Service + 'static>(&self, dev: u32, name: &str) -> Result<(usize, u32), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let os = self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let s = os.svc(name).ok_or_else(|| ActErr::Refused(format!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        if !s.svc.as_ref().is_some_and(|x| x.as_any().downcast_ref::<T>().is_some()) {
            return Err(ActErr::Refused(format!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", name)));
        }
        Ok((ni, s.id))
    }

    /// changes a service of type T; a running one gets its OsApi, a stopped one is changed
    /// as it is
    fn lab_change<T: Service + 'static>(&mut self, dev: u32, name: &str, f: impl FnOnce(&mut T, Option<&mut crate::os::OsApi>) -> Result<(), String>) -> Result<(), ActErr> {
        let (ni, id) = self.lab_svc::<T>(dev, name)?;
        let running = self.os(ni).is_some_and(|o| o.running(id));
        let mut out: Result<(), String> = Ok(());
        if running {
            let mut f = Some(f);
            self.svc_call(ni, id, None, |s, api| {
                if let (Some(t), Some(f)) = (s.as_any_mut().downcast_mut::<T>(), f.take()) {
                    out = f(t, Some(api));
                }
            });
        } else if let Some(t) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)).and_then(|s| s.svc.as_mut()).and_then(|x| x.as_any_mut().downcast_mut::<T>()) {
            out = f(t, None);
        }
        out.map_err(ActErr::Refused)
    }

    /// the actions of the lab (from svcact.rs's os_apply)
    pub(crate) fn lab_apply(&mut self, a: Action) -> Result<(), ActErr> {
        match a {
            Action::Fault { fault, dur } => self.fault_start(fault, dur).map(|_| ()),
            Action::Heal { id } => self.fault_heal(id),
            Action::HealAll => self.fault_heal_all(),
            Action::LbBackend { dev, svc, op } => self.lab_change::<crate::lb::Lb>(dev, &svc, |lb, api| match api {
                Some(api) => lb.change(api, op),
                None => lb.change_stopped(op),
            }),
            Action::LbAlgorithm { dev, svc, algo } => self.lab_change::<crate::lb::Lb>(dev, &svc, |lb, _| {
                lb.set_algo(algo);
                Ok(())
            }),
            Action::KvSet { dev, svc, consistency, w, r } => self.lab_change::<crate::kv::KvNode>(dev, &svc, |kv, _| {
                kv.set(consistency, w, r);
                Ok(())
            }),
            Action::CacheClear { dev, svc } => self.lab_change::<crate::cache::Cache>(dev, &svc, |c, _| {
                c.clear();
                Ok(())
            }),
            Action::WorkerSet { dev, svc, concurrency, time_ns, error_ppm } => self.lab_change::<crate::mq::Worker>(dev, &svc, |w, api| {
                match api {
                    Some(api) => w.set(api, concurrency, time_ns, error_ppm),
                    None => {
                        if let Some(c) = concurrency {
                            w.concurrency = c.clamp(1, 10_000);
                        }
                        if let Some(t) = time_ns {
                            w.time = t;
                        }
                        if let Some(e) = error_ppm {
                            w.error_ppm = e.min(1_000_000);
                        }
                    }
                }
                Ok(())
            }),
            _ => Err(ActErr::Refused(String::from("Diese Aktion kennt der Simulator nicht."))),
        }
    }
}
