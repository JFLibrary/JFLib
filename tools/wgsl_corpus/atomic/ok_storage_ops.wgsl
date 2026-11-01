// WGSL "Atomic Built-in Functions": atomicLoad / Store / Exchange / Min / Max / And / Or / Xor / Sub on a storage atomic
// expect: valid
@group(0) @binding(0) var<storage, read_write> a: atomic<i32>;
@compute @workgroup_size(1) fn main() {
  atomicStore(&a, 1); _ = atomicLoad(&a) + atomicExchange(&a, 2) + atomicMin(&a, 0) + atomicMax(&a, 3);
  _ = atomicAnd(&a, 7) | atomicOr(&a, 8);
  _ = atomicXor(&a, 1) - atomicSub(&a, 1);
}
