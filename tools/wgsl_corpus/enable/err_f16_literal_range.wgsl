// WGSL "Enable Extensions": 65504h is the largest half, 65536h overflows
// expect: error
enable f16;
const a = 65536.0h;
