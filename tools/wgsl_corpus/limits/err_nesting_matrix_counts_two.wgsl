// WGSL "Limits": array x 14 + matrix = 16
// expect: error
var<private> v: array<array<array<array<array<array<array<array<array<array<array<array<array<array<mat2x2f, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>;
