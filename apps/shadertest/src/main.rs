// shadertest -- self-checks of lib/shader without a GPU: golden bytes of a tiny module, the
// builder (deduplication, capabilities, interfaces per version), the disassembly of every
// benchmark shader against apps/shadertest/golden, the validator (it accepts every benchmark
// shader and rejects one broken module per rule), determinism, the std140 / std430 layout
// helper, the CPU references of move and cull, an interpreter that runs the benchmark
// shaders' SPIR-V on the CPU and compares them with those references, and -- when the Vulkan
// SDK is installed -- spirv-val and spirv-dis as references (skipped with a note otherwise).
// Console program; exit code 1 when a check fails.
//
//   python build_rs.py test shadertest
//   build/win/shadertest.exe [--check] [--only GROUP]
//   build/win/shadertest.exe --write DIR      the shaders (.spv for SPIR-V 1.3, _16.spv for
//                                             1.6), their disassembly (.spvasm, the golden
//                                             files) and the broken modules (bad_*.spv)
//   build/win/shadertest.exe --sdk DIR        the folder with spirv-val / spirv-dis (default:
//                                             %VULKAN_SDK%\Bin, the newest C:\VulkanSDK\*\Bin,
//                                             then PATH)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

mod basics;
mod broken;
mod interp;
mod refs;
mod sdk;

/// the result collector of a check group
pub struct T {
    pub group: &'static str,
    pub ok: u32,
    pub failed: u32,
}

impl T {
    pub fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}\n", self.group, name).as_bytes());
        }
    }
    pub fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn run() -> i32 {
    if let Some(dir) = plat::arg_value("--write") {
        return basics::write_all(&dir);
    }
    let only = plat::arg_value("--only").map(String::from);
    let groups: [(&'static str, fn(&mut T)); 9] = [
        ("golden", basics::golden),
        ("builder", basics::builder),
        ("dis", basics::dis),
        ("layout", basics::layout),
        ("validate", broken::run),
        ("refs", refs::run),
        ("interp", interp::run),
        ("determinism", basics::determinism),
        ("sdk", sdk::run),
    ];
    let mut failed = 0;
    let mut total = 0;
    for (name, f) in groups {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        out(&format!("{:<12} {} ok, {} failed\n", name, t.ok, t.failed));
        failed += t.failed;
        total += t.ok + t.failed;
    }
    out(&format!("shadertest: {} of {} checks passed\n", total - failed, total));
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
