// WGSL "Identifiers": identifiers are XID_Start XID_Continue* (Unicode)
// expect: valid
const Δx = 1;
const 変数 = Δx + 1;
const_assert 変数 == 2;
