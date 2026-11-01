// sdk -- spirv-val and spirv-dis of the Vulkan SDK as references when they are installed (a
// test-time aid only: lib/shader never needs them).  spirv-val (Vulkan 1.1 environment for
// SPIR-V 1.3, Vulkan 1.3 for 1.6) must accept every module shadertest builds and reject the
// broken modules of broken::cases() it is expected to reject; spirv-dis' text of the
// benchmark shaders is compared with dis::disassemble (differences are reported, not failed).
// Without the tools the group reports that it skipped.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::bench::Shader;

use crate::T;

const OUT: &str = "build/shadertest";

fn exe(dir: &str, name: &str) -> String {
    plat::fs::join(dir, &if cfg!(windows) { format!("{}.exe", name) } else { String::from(name) })
}

fn has_tools(dir: &str) -> bool {
    plat::file_exists(&exe(dir, "spirv-val")) == 1 && plat::file_exists(&exe(dir, "spirv-dis")) == 1
}

fn version_key(name: &str) -> Vec<u32> {
    name.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// the folder with spirv-val and spirv-dis: --sdk DIR, %VULKAN_SDK%\Bin, the newest
/// C:\VulkanSDK\*\Bin, the folder of spirv-val in PATH
pub fn find() -> Option<String> {
    let bin = if cfg!(windows) { "Bin" } else { "bin" };
    let mut tries: Vec<String> = Vec::new();
    if let Some(d) = plat::arg_value("--sdk") {
        tries.push(d);
    }
    if let Some(d) = plat::env("VULKAN_SDK") {
        tries.push(plat::fs::join(&d, bin));
    }
    if cfg!(windows) {
        if let Some(list) = plat::fs::read_dir("C:\\VulkanSDK") {
            let mut names: Vec<String> = list.into_iter().map(|e| e.name).collect();
            names.sort_by_key(|n| version_key(n));
            if let Some(n) = names.last() {
                tries.push(format!("C:\\VulkanSDK\\{}\\{}", n, bin));
            }
        }
    }
    if let Some(p) = plat::process::find("spirv-val") {
        let cut = p.rfind(['/', '\\']).unwrap_or(0);
        tries.push(String::from(&p[..cut]));
    }
    tries.into_iter().find(|d| has_tools(d))
}

/// run a tool -> (exit code, stdout, stderr)
fn tool(program: &str, args: &[&str]) -> Result<(i32, String, String), String> {
    let mut c = plat::process::Command { program, args, cwd: None, env: &[] }.spawn()
        .map_err(|e| format!("{} does not start (error {})", program, e))?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let start = plat::time_us();
    let mut buf = [0u8; 8192];
    loop {
        let mut idle = true;
        while let Ok(Some(n)) = c.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            idle = false;
        }
        while let Ok(Some(n)) = c.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            err.extend_from_slice(&buf[..n]);
            idle = false;
        }
        if let Ok(Some(code)) = c.try_wait() {
            while let Ok(Some(n)) = c.read_stdout(&mut buf) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
            while let Ok(Some(n)) = c.read_stderr(&mut buf) {
                if n == 0 {
                    break;
                }
                err.extend_from_slice(&buf[..n]);
            }
            let text = |b: &[u8]| String::from_utf8_lossy(b).replace('\r', "");
            return Ok((code, text(&out), text(&err)));
        }
        if plat::time_us() - start > 30_000_000 {
            c.terminate();
            return Err(format!("{} takes too long", program));
        }
        if idle {
            plat::sleep_ms(1);
        }
    }
}

fn env_for(words: &[u32]) -> &'static str {
    if (words.get(1).copied().unwrap_or(0) >> 8 & 0xff) >= 4 { "vulkan1.3" } else { "vulkan1.1" }
}

/// spirv-val on words (written to OUT/name.spv) -> (accepted, first message line)
fn validate(val: &str, name: &str, words: &[u32]) -> Result<(bool, String), String> {
    let path = plat::fs::join(OUT, &format!("{}.spv", name));
    if !plat::write_file(&path, &shader::to_bytes(words)) {
        return Err(format!("{} cannot be written", path));
    }
    let (code, o, e) = tool(val, &["--target-env", env_for(words), &path])?;
    let msg = e.lines().chain(o.lines()).find(|l| !l.trim().is_empty()).map(String::from).unwrap_or_default();
    Ok((code == 0, msg))
}

pub fn run(t: &mut T) {
    let Some(dir) = find() else {
        crate::out("   sdk: spirv-val / spirv-dis not found (--sdk, VULKAN_SDK, C:\\VulkanSDK, PATH): reference checks skipped\n");
        return;
    };
    if !plat::mkdir(OUT) {
        t.check(&format!("{} can be created", OUT), false);
        return;
    }
    let (val, dis) = (exe(&dir, "spirv-val"), exe(&dir, "spirv-dis"));
    if let Ok((_, v, _)) = tool(&val, &["--version"]) {
        crate::out(&format!("   sdk: {}\n", v.lines().next().unwrap_or("?")));
    }
    // every module shadertest builds is valid for spirv-val
    let mut good: Vec<(String, Vec<u32>)> = Vec::new();
    for sh in Shader::ALL {
        good.push((String::from(sh.name()), sh.build(3)));
        good.push((format!("{}_16", sh.name()), sh.build(6)));
    }
    good.push((String::from("tiny"), crate::basics::tiny()));
    good.push((String::from("features"), crate::basics::features()));
    good.push((String::from("doc_example"), crate::basics::doc_example()));
    good.push((String::from("frag_features"), crate::basics::frag_features(3)));
    good.push((String::from("frag_features_16"), crate::basics::frag_features(6)));
    for (name, w) in &good {
        match validate(&val, name, w) {
            Ok((ok, msg)) => {
                if !ok {
                    crate::out(&format!("   spirv-val {}: {}\n", name, msg));
                }
                t.check(&format!("spirv-val accepts {}", name), ok);
            }
            Err(e) => t.check(&format!("spirv-val {}: {}", name, e), false),
        }
    }
    // the broken modules: ours rejects all (validate group); spirv-val those it should
    let (mut rejected, mut total) = (0, 0);
    for c in crate::broken::cases() {
        let name = format!("bad_{}", c.name);
        match validate(&val, &name, &c.words) {
            Ok((ok, msg)) => {
                total += 1;
                rejected += (!ok) as u32;
                if c.spirv_val {
                    if ok {
                        crate::out(&format!("   spirv-val accepts {} (rule {:?})\n", name, c.rule));
                    }
                    t.check(&format!("spirv-val rejects {}", name), !ok);
                } else {
                    crate::out(&format!("   {} (rule {:?}) is beyond spirv-val: {}\n", name, c.rule,
                                        if ok { "accepted" } else { msg.as_str() }));
                }
            }
            Err(e) => t.check(&format!("spirv-val {}: {}", name, e), false),
        }
    }
    crate::out(&format!("   sdk: spirv-val rejects {} of {} broken modules, shader::val all of them\n", rejected, total));
    // spirv-dis against dis::disassemble
    let mut same = 0;
    for sh in Shader::ALL {
        for minor in [3u32, 6] {
            let name = if minor == 3 { String::from(sh.name()) } else { format!("{}_16", sh.name()) };
            let path = plat::fs::join(OUT, &format!("{}.spv", name));
            let theirs = match tool(&dis, &[&path]) {
                Ok((0, o, _)) => o,
                Ok((code, _, e)) => {
                    t.check(&format!("spirv-dis {}: exit {} {}", name, code, e.trim()), false);
                    continue;
                }
                Err(e) => {
                    t.check(&format!("spirv-dis {}: {}", name, e), false);
                    continue;
                }
            };
            let ours = shader::dis::disassemble(&sh.build(minor)).unwrap_or_default();
            let diff: Vec<(usize, &str, &str)> = ours.lines().zip(theirs.lines()).enumerate()
                .filter(|(_, (a, b))| a != b).map(|(k, (a, b))| (k + 1, a, b)).collect();
            let lens = (ours.lines().count(), theirs.lines().count());
            if diff.is_empty() && lens.0 == lens.1 {
                same += 1;
            } else {
                crate::out(&format!("   dis {}: {} lines differ from spirv-dis ({} / {} lines)\n", name, diff.len(), lens.0, lens.1));
                for (k, a, b) in diff.iter().take(3) {
                    crate::out(&format!("      line {}: ours   {}\n               theirs {}\n", k, a, b));
                }
            }
        }
    }
    crate::out(&format!("   sdk: dis::disassemble = spirv-dis for {} of 12 shader modules\n", same));
}
