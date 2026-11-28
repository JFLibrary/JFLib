// Android has no child processes for an app (docs/ANDROID.md, "Bewusst nicht", decision 8): from
// targetSdk 29 on it executes nothing from writable folders (W^X), only from the APK's folder of
// native libraries.  The module is there all the same, so apps build for every target without a
// cfg of their own: every start answers ENOTSUP (95), run_elevated an error text, elevated()
// false, find() None.  A Child cannot exist (no start succeeds).
use alloc::string::String;

use super::Command;

/// the error number of every start on Android: ENOTSUP / EOPNOTSUPP ("not supported")
pub const ENOTSUP: u32 = 95;

/// (never made on Android) a child process
pub struct Child {
    never: core::convert::Infallible,
}

impl Child {
    /// read what the child wrote to stdout (never on Android)
    pub fn read_stdout(&mut self, _bytes: &mut [u8]) -> Result<Option<usize>, u32> { match self.never {} }
    /// read what the child wrote to stderr (never on Android)
    pub fn read_stderr(&mut self, _bytes: &mut [u8]) -> Result<Option<usize>, u32> { match self.never {} }
    /// write to the child's stdin (never on Android)
    pub fn write_stdin(&mut self, _bytes: &[u8]) -> Result<Option<usize>, u32> { match self.never {} }
    /// the child's process id (never on Android)
    pub fn id(&self) -> u32 { match self.never {} }
    /// close the child's stdin (never on Android)
    pub fn close_stdin(&mut self) { match self.never {} }
    /// whether the child ended, its exit code (never on Android)
    pub fn try_wait(&mut self) -> Result<Option<i32>, u32> { match self.never {} }
    /// end the child and its tree (never on Android)
    pub fn terminate(&mut self) { match self.never {} }
}

pub(super) fn spawn(_c: &Command<'_>) -> Result<Child, u32> {
    Err(ENOTSUP)
}

pub(super) fn elevated() -> bool {
    false
}

pub(super) fn exec(_program: &str, _args: &[&str], _env: &[(&str, Option<&str>)]) -> u32 {
    ENOTSUP
}

pub(super) fn find(_name: &str) -> Option<String> {
    None
}

pub(super) fn launch(_program: &str, _args: &[&str]) -> Result<(), u32> {
    Err(ENOTSUP)
}

pub(super) fn run_elevated(_program: &str, _args: &[&str]) -> Result<i32, String> {
    Err(String::from("Android startet keine Programme (keine Kindprozesse)"))
}
