// WGSL "Limits": a matrix counts 2: array x 13 + matrix = 15
// expect: valid
var<private> v: array<array<array<array<array<array<array<array<array<array<array<array<array<mat2x2f, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>, 1>;
