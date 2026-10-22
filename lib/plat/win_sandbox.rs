// win_sandbox -- the Windows half of plat::sandbox (docs/PERMISSIONS.md "OS sandbox"): process
// mitigation policies derived from the manifest, set once at start-up (sandbox::apply, through
// boot.rs).  A full-trust Win32 process is not confined the way an AppContainer is (see the
// docs for why re-executing into one is not done here), but these policies still harden it as
// defence in depth and follow the manifest.
//
//   apply(&WinPlan) -> Status
//
// Always (level >= 1): image load (no remote / low-label images), extension points disabled,
// and -- unless the manifest allows native plugins -- dynamic code prohibited.  `strict`
// (level 2) also blocks child-process creation when no program is allowed (this stops
// plat::spawn_self too; see the docs).  Strict handle checks are deliberately not set: an
// invalid-handle reference anywhere would raise an exception and turn a benign race in a GUI
// into a crash.

use crate::policy::WinPlan;
use crate::sandbox::Status;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetProcessMitigationPolicy(policy: i32, buf: *const u32, len: usize) -> i32;
    fn GetProcessMitigationPolicy(process: isize, policy: i32, buf: *mut u32, len: usize) -> i32;
}

// PROCESS_MITIGATION_POLICY values
const EXTENSION_POINT_DISABLE: i32 = 6;
const DYNAMIC_CODE: i32 = 8;
const IMAGE_LOAD: i32 = 10;
const CHILD_PROCESS: i32 = 13;

// each of these policies is one DWORD of flags
const PROHIBIT_DYNAMIC_CODE: u32 = 1;
const DISABLE_EXTENSION_POINTS: u32 = 1;
const NO_REMOTE_IMAGES: u32 = 1;
const NO_LOW_LABEL_IMAGES: u32 = 2;
const NO_CHILD_PROCESS: u32 = 1;

const CURRENT_PROCESS: isize = -1;

fn set(policy: i32, value: u32) -> bool {
    // SAFETY: a single-DWORD mitigation policy buffer of the matching length.
    unsafe { SetProcessMitigationPolicy(policy, &value, 4) != 0 }
}

fn get(policy: i32) -> u32 {
    let mut v = 0u32;
    // SAFETY: the pseudo handle of this process; a DWORD out-parameter.
    let ok = unsafe { GetProcessMitigationPolicy(CURRENT_PROCESS, policy, &mut v, 4) };
    if ok != 0 {
        v
    } else {
        0
    }
}

/// set the process mitigation policies the plan asks for
pub fn apply(plan: &WinPlan) -> Status {
    let mut st = Status { level: plan.level, ..Status::NONE };
    let img = set(IMAGE_LOAD, NO_REMOTE_IMAGES | NO_LOW_LABEL_IMAGES);
    set(EXTENSION_POINT_DISABLE, DISABLE_EXTENSION_POINTS);
    if !plan.plugins_allowed {
        set(DYNAMIC_CODE, PROHIBIT_DYNAMIC_CODE);
    }
    if plan.level >= 2 && !plan.exec_allowed {
        set(CHILD_PROCESS, NO_CHILD_PROCESS);
    }
    st.win_mitigations = img;
    st.win_dynamic_code = get(DYNAMIC_CODE) & PROHIBIT_DYNAMIC_CODE != 0;
    st.exec_denied = get(CHILD_PROCESS) & NO_CHILD_PROCESS != 0;
    st
}
