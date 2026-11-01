// WGSL "Limits": a type alone is no use
// expect: valid
alias A = array<u32, 100000>;
