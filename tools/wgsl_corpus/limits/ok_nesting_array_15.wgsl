// WGSL "Limits": composite nesting depth 15 (the spec's minimum)
// expect: valid
var<private> v: array<array<array<array<array<array<array<array<array<array<array<array<array<array<array<u32, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>;
