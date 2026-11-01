// WGSL "Uniformity Analysis": the continuing block follows the non-uniform continue
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { if i == 0u { continue; } continuing { workgroupBarrier(); break if true; } }
}
