// files -- the files of a device at run time (Os::files, a jfn_model::files::Store): what the
// services and the command lines read and write, the reports that keep the project in step
//
//   Engine::files(dev) / file_store(dev)    the records / the store of a device
//   Action::Files { dev, files }             the project's files of a device (load, undo): the
//                                            store takes them without a report
//   Engine::file_put / file_del / file_mkdir (crate)   changes from the command lines: each one
//                                            reports Report::Files with the path and every record
//   OsApi::file_read / file_write / file_append / file_remove / file_list / file_stat /
//   file_norm / file_mtime                   the same for services (TFTP, FTP, later SNMP,
//                                            AAA, mail); writes and removals report too
//
// A file's time is the device's clock in seconds.  The store survives power off and a service's
// end, as a disk does; a device that starts again from the project gets the project's files.
// How long an access takes is the business of the service (OsApi::disk_read / disk_write).

use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::{File, FsErr, Stat, Store};

use crate::action::ActErr;
use crate::driver::Report;
use crate::kernel::Engine;
use crate::os::OsApi;

impl Engine {
    /// the files of a device as the project keeps them (in path order)
    pub fn files(&self, dev: u32) -> Vec<File> {
        self.file_store(dev).map(Store::records).unwrap_or_default()
    }
    /// the file store of a device (None without an operating system)
    pub fn file_store(&self, dev: u32) -> Option<&Store> {
        self.os(self.node_index(dev)?).map(|o| &o.files)
    }
    /// Action::Files: the store takes the project's records
    pub(crate) fn files_set(&mut self, dev: u32, files: &[File]) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
        os.files.load(files);
        Ok(())
    }
    /// a device's store changed at `path`: the app takes the records as a step of the project
    pub(crate) fn files_changed(&mut self, ni: usize, path: &str) {
        let dev = self.dev_of(ni);
        let files = self.os(ni).map(|o| o.files.records()).unwrap_or_default();
        let at = self.now;
        self.cli.reports.push(Report::Files { dev, at, path: String::from(path), files });
    }
    /// the device's clock in seconds (file times)
    pub(crate) fn file_time(&self, ni: usize) -> i64 {
        let now = self.now;
        self.os(ni).map(|o| o.clock.read(now)).unwrap_or(0).div_euclid(1_000_000_000)
    }
    pub(crate) fn store(&self, ni: usize) -> Option<&Store> {
        self.os(ni).map(|o| &o.files)
    }
    /// writes a file for a command line (and reports it)
    pub(crate) fn file_put(&mut self, ni: usize, path: &str, data: &[u8], append: bool) -> Result<String, FsErr> {
        let t = self.file_time(ni);
        let os = self.os_mut(ni).ok_or(FsErr::NotFound)?;
        let p = os.files.norm(path)?;
        if append {
            os.files.append(&p, data, t)?;
        } else {
            os.files.write(&p, data, t)?;
        }
        self.files_changed(ni, &p);
        Ok(p)
    }
    /// removes a file (or with `tree` a directory and what is in it) for a command line
    pub(crate) fn file_del(&mut self, ni: usize, path: &str, tree: bool) -> Result<String, FsErr> {
        let os = self.os_mut(ni).ok_or(FsErr::NotFound)?;
        let p = os.files.norm(path)?;
        if tree && os.files.stat(&p).is_some_and(|s| s.dir) {
            os.files.remove_tree(&p)?;
        } else {
            os.files.remove(&p)?;
        }
        self.files_changed(ni, &p);
        Ok(p)
    }
    /// makes a directory for a command line
    pub(crate) fn file_mkdir(&mut self, ni: usize, path: &str, parents: bool) -> Result<String, FsErr> {
        let t = self.file_time(ni);
        let os = self.os_mut(ni).ok_or(FsErr::NotFound)?;
        let p = os.files.norm(path)?;
        let had = os.files.exists(&p);
        os.files.mkdir(&p, t, parents)?;
        if !had {
            self.files_changed(ni, &p);
        }
        Ok(p)
    }
    /// removes an empty directory for a command line
    pub(crate) fn file_rmdir(&mut self, ni: usize, path: &str) -> Result<String, FsErr> {
        let os = self.os_mut(ni).ok_or(FsErr::NotFound)?;
        let p = os.files.norm(path)?;
        os.files.rmdir(&p)?;
        self.files_changed(ni, &p);
        Ok(p)
    }
}

impl Engine {
    /// the problems of the files: TFTP and FTP servers whose directory is missing
    pub(crate) fn file_problems(&self, out: &mut Vec<crate::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        for dev in self.devices() {
            let name = self.node(dev).map(|n| n.name.clone()).unwrap_or_default();
            let mut missing = self.tftp_missing_roots(dev);
            missing.extend(self.ftp_missing_dirs(dev));
            for (svc, dir) in missing {
                let msg = jfn_model::msg!("Dienst {} auf {}: das Verzeichnis {} gibt es nicht.", svc, name, dir);
                out.push(Problem { kind: ProblemKind::FilesDirMissing, severity: Severity::Error, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
            }
        }
    }
}

impl OsApi<'_> {
    /// the device's file store
    pub fn files(&self) -> Option<&Store> {
        self.eng.store(self.ni)
    }
    /// a path made canonical (Store::norm)
    pub fn file_norm(&self, path: &str) -> Result<String, FsErr> {
        self.files().ok_or(FsErr::NotFound)?.norm(path)
    }
    pub fn file_stat(&self, path: &str) -> Option<Stat> {
        self.files()?.stat(path)
    }
    /// the bytes of a file (a copy)
    pub fn file_read(&self, path: &str) -> Result<Vec<u8>, FsErr> {
        self.files().ok_or(FsErr::NotFound)?.read(path).map(Vec::from)
    }
    pub fn file_list(&self, dir: &str) -> Result<Vec<(String, Stat)>, FsErr> {
        self.files().ok_or(FsErr::NotFound)?.list(dir)
    }
    /// the device's clock in seconds (file times)
    pub fn file_mtime(&self) -> i64 {
        self.eng.file_time(self.ni)
    }
    /// writes a file (made or replaced); the project hears of it
    pub fn file_write(&mut self, path: &str, data: &[u8]) -> Result<String, FsErr> {
        self.eng.file_put(self.ni, path, data, false)
    }
    /// adds bytes at the end of a file
    pub fn file_append(&mut self, path: &str, data: &[u8]) -> Result<String, FsErr> {
        self.eng.file_put(self.ni, path, data, true)
    }
    /// removes a file
    pub fn file_remove(&mut self, path: &str) -> Result<String, FsErr> {
        self.eng.file_del(self.ni, path, false)
    }
    /// makes a directory (with its parents); the project hears of it
    pub fn file_mkdir(&mut self, path: &str, parents: bool) -> Result<String, FsErr> {
        self.eng.file_mkdir(self.ni, path, parents)
    }
}
