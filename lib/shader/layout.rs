// layout -- std140 / std430 offsets, sizes and strides (Vulkan "Offset and Stride Assignment")
//
//   Rule::Std140   uniform buffers: the extended alignment (arrays, structs and matrices
//                  rounded up to 16 bytes)
//   Rule::Std430   storage buffers and push constants: the base alignment
//   align / size / stride / matrix_stride / offsets(members) over any Types table (the
//   builder's Module, the validator's parsed module); matrices are column-major
//
// Base alignment: a scalar of N bytes N; vec2 2N; vec3 and vec4 4N; an array that of its
// element; a struct the largest of its members; a column-major matrix that of its column.
// A struct's size is rounded up to its alignment, an array's stride is its element's size
// rounded up to the array's alignment, a matrix is an array of its columns.  Runtime arrays
// have no size (the last member of a buffer block); bool, images and pointers have no layout.

use alloc::vec::Vec;

use crate::module::{Ty, Types};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Std140,
    Std430,
}

fn up(x: u32, a: u32) -> u32 {
    x.div_ceil(a) * a
}

fn ext(a: u32, rule: Rule) -> u32 {
    if rule == Rule::Std140 { up(a, 16) } else { a }
}

/// the alignment of a type under rule (None: no layout)
pub fn align(t: &dyn Types, ty: u32, rule: Rule) -> Option<u32> {
    Some(match t.ty(ty)? {
        Ty::Int { width, .. } | Ty::Float { width } => width / 8,
        Ty::Vector { elem, n } => {
            let s = align(t, elem.0, rule)?;
            if *n == 2 { 2 * s } else { 4 * s }
        }
        Ty::Matrix { col, .. } => ext(align(t, col.0, rule)?, rule),
        Ty::Array { elem, .. } | Ty::RuntimeArray { elem } => ext(align(t, elem.0, rule)?, rule),
        Ty::Struct { members } => {
            let mut a = 1;
            for m in members {
                a = a.max(align(t, m.0, rule)?);
            }
            ext(a, rule)
        }
        _ => return None,
    })
}

/// the column stride of a column-major matrix type
pub fn matrix_stride(t: &dyn Types, mat: u32, rule: Rule) -> Option<u32> {
    match t.ty(mat)? {
        Ty::Matrix { col, .. } => Some(ext(align(t, col.0, rule)?, rule)),
        _ => None,
    }
}

/// the stride of an array of elem
pub fn stride(t: &dyn Types, elem: u32, rule: Rule) -> Option<u32> {
    let a = ext(align(t, elem, rule)?, rule);
    Some(up(size(t, elem, rule)?, a))
}

/// the size in bytes (None for runtime arrays and types without layout; a struct ending in
/// a runtime array: the size without it)
pub fn size(t: &dyn Types, ty: u32, rule: Rule) -> Option<u32> {
    Some(match t.ty(ty)? {
        Ty::Int { width, .. } | Ty::Float { width } => width / 8,
        Ty::Vector { elem, n } => n * size(t, elem.0, rule)?,
        Ty::Matrix { cols, .. } => cols * matrix_stride(t, ty, rule)?,
        Ty::Array { elem, len } => t.const_u32(len.0)? * stride(t, elem.0, rule)?,
        Ty::Struct { members } => {
            let raw: Vec<u32> = members.iter().map(|m| m.0).collect();
            let offs = offsets(t, &raw, rule)?;
            let mut end = 0;
            for (k, m) in raw.iter().enumerate() {
                if let Some(s) = size(t, *m, rule) {
                    end = end.max(offs[k] + s);
                }
            }
            up(end, align(t, ty, rule)?)
        }
        _ => return None,
    })
}

/// the offsets of struct members laid out one after another
pub fn offsets(t: &dyn Types, members: &[u32], rule: Rule) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(members.len());
    let mut at = 0;
    for (k, &m) in members.iter().enumerate() {
        at = up(at, align(t, m, rule)?);
        out.push(at);
        match (t.ty(m)?, size(t, m, rule)) {
            (Ty::RuntimeArray { .. }, _) if k + 1 == members.len() => {}
            (_, Some(s)) => at += s,
            _ => return None,
        }
    }
    Some(out)
}
