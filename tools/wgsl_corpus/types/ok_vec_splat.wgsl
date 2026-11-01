// WGSL "Constructible Types": a vector from one scalar splats it
// expect: valid
const v = vec3(2);
const_assert v.z == 2;
