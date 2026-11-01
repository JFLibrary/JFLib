// WGSL "override Declarations": const_assert cannot test an override
// expect: error
override a = 1;
const_assert a == 1;
