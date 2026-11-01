// WGSL "Declaration and Scope": type aliases must not form a cycle
// expect: error
alias A = array<B, 2>;
alias B = A;
