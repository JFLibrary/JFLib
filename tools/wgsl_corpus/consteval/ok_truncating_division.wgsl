// WGSL "Arithmetic Expressions": integer division truncates towards zero, % takes the sign of the dividend
// expect: valid
const_assert -7 / 2 == -3;
const_assert -7 % 3 == -1;
const_assert 7 % -3 == 1;
