// WGSL "Limits": 16: over the spec's minimum (Chrome allows 255)
// expect: error
var<private> v: array<array<array<array<array<array<array<array<array<array<array<array<array<array<array<array<u32, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>;
