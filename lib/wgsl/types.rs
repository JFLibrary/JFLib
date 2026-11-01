// types -- the type table of the checker: interning, the type properties of spec section 6
// (constructible, storable, host-shareable, creation-fixed / fixed footprint, abstract),
// WGSL's memory layout (14.4: align, size, stride, struct offsets), the address space layout
// constraints (14.4.5), ConversionRank and concretization (6.1.2)
//
//   Types::new()                 the scalars at TypeIds 0..7 (ir::BOOL ... ir::F16), ir::VOID, every
//                                vector and matrix interned up front (vec(n, s), mat(c, r, s))
//   intern(Type) -> TypeId       equal types get equal ids
//   rank(src, dst) -> u32        INF when no automatic conversion exists
//   concretize(t)                AbstractInt -> i32, AbstractFloat -> f32, recursively
//   layout(t) -> Option<(align, size)>   None for types without a memory layout
//   layout_error(t, space, standard)     the first violation of the address space's layout
//                                constraints, as a German message

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ir::*;

pub const INF: u32 = u32::MAX;

pub struct Types {
    pub list: Vec<Type>,
    pub structs: Vec<Struct>,
    map: BTreeMap<Type, TypeId>,
    vecs: [[TypeId; 7]; 3],
    mats: [[[TypeId; 3]; 3]; 3],
    builtin_structs: BTreeMap<BuiltinStruct, TypeId>,
}

fn sidx(s: Scalar) -> usize {
    s as usize
}

fn fidx(s: Scalar) -> usize {
    match s {
        Scalar::F16 => 1,
        Scalar::AbstractFloat => 2,
        _ => 0,
    }
}

impl Types {
    pub fn new() -> Types {
        let mut t = Types {
            list: Vec::new(),
            structs: Vec::new(),
            map: BTreeMap::new(),
            vecs: [[0; 7]; 3],
            mats: [[[0; 3]; 3]; 3],
            builtin_structs: BTreeMap::new(),
        };
        for s in Scalar::ALL {
            t.intern(Type::Scalar(s));
        }
        t.intern(Type::Void);
        for n in 2..=4u8 {
            for s in Scalar::ALL {
                t.vecs[n as usize - 2][sidx(s)] = t.intern(Type::Vector(n, s));
            }
        }
        for c in 2..=4u8 {
            for r in 2..=4u8 {
                for s in [Scalar::F32, Scalar::F16, Scalar::AbstractFloat] {
                    t.mats[c as usize - 2][r as usize - 2][fidx(s)] = t.intern(Type::Matrix(c, r, s));
                }
            }
        }
        t
    }

    /// the table of a checked module again (the back end's constant evaluation: the ids are
    /// the module's, since the checker interned in the same order)
    pub fn from_module(types: &[Type], structs: &[Struct]) -> Types {
        let mut t = Types::new();
        t.structs = structs.to_vec();
        for (i, ty) in types.iter().enumerate() {
            if i >= t.list.len() {
                t.list.push(*ty);
                t.map.insert(*ty, i as TypeId);
            }
        }
        for (i, s) in structs.iter().enumerate() {
            if let (Some(b), Some(&id)) = (s.builtin, t.map.get(&Type::Struct(i as u32))) {
                t.builtin_structs.insert(b, id);
            }
        }
        t
    }

    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(&id) = self.map.get(&ty) {
            return id;
        }
        let id = self.list.len() as TypeId;
        self.list.push(ty);
        self.map.insert(ty, id);
        id
    }

    pub fn get(&self, t: TypeId) -> Type {
        self.list.get(t as usize).copied().unwrap_or(Type::Scalar(Scalar::Bool))
    }

    pub fn name(&self, t: TypeId) -> String {
        type_name(&self.list, &self.structs, t)
    }

    pub fn vec(&self, n: u8, s: Scalar) -> TypeId {
        self.vecs[(n.clamp(2, 4) - 2) as usize][sidx(s)]
    }

    /// matCxR<s>; s must be a float scalar
    pub fn mat(&self, c: u8, r: u8, s: Scalar) -> TypeId {
        self.mats[(c.clamp(2, 4) - 2) as usize][(r.clamp(2, 4) - 2) as usize][fidx(s)]
    }

    /// scalar s (n = 1) or vecN<s>
    pub fn vec_or_scalar(&self, n: u8, s: Scalar) -> TypeId {
        if n <= 1 { s.id() } else { self.vec(n, s) }
    }

    pub fn array(&mut self, elem: TypeId, count: ArrayCount) -> TypeId {
        let stride = match self.layout(elem) {
            Some((a, s)) if !self.is_abstract(elem) => round_up(a, s),
            _ => 0,
        };
        self.intern(Type::Array { elem, count, stride })
    }

    pub fn ptr(&mut self, space: AddressSpace, store: TypeId, access: Access) -> TypeId {
        self.intern(Type::Pointer { space, store, access })
    }

    pub fn reference(&mut self, space: AddressSpace, store: TypeId, access: Access) -> TypeId {
        self.intern(Type::Reference { space, store, access })
    }

    /// add a user struct (its layout already computed)
    pub fn add_struct(&mut self, s: Struct) -> TypeId {
        let i = self.structs.len() as u32;
        self.structs.push(s);
        self.intern(Type::Struct(i))
    }

    /// the result structures of frexp, modf and atomicCompareExchangeWeak
    pub fn builtin_struct(&mut self, b: BuiltinStruct) -> TypeId {
        if let Some(&t) = self.builtin_structs.get(&b) {
            return t;
        }
        let (name, members): (String, Vec<(&str, TypeId)>) = match b {
            BuiltinStruct::Frexp(n, s) => {
                let exp_s = if s == Scalar::AbstractFloat { Scalar::AbstractInt } else { Scalar::I32 };
                let w = if n > 1 { format!("vec{}_", n) } else { String::new() };
                let sn = if s == Scalar::AbstractFloat { "abstract" } else { s.name() };
                (format!("__frexp_result_{}{}", w, sn), alloc::vec![("fract", self.vec_or_scalar(n, s)), ("exp", self.vec_or_scalar(n, exp_s))])
            }
            BuiltinStruct::Modf(n, s) => {
                let w = if n > 1 { format!("vec{}_", n) } else { String::new() };
                let sn = if s == Scalar::AbstractFloat { "abstract" } else { s.name() };
                let t = self.vec_or_scalar(n, s);
                (format!("__modf_result_{}{}", w, sn), alloc::vec![("fract", t), ("whole", t)])
            }
            BuiltinStruct::AtomicCompareExchange(s) => {
                (format!("__atomic_compare_exchange_result<{}>", s.name()), alloc::vec![("old_value", s.id()), ("exchanged", BOOL)])
            }
        };
        let mut ms = Vec::new();
        let mut off = 0;
        let mut align = 1;
        for (mname, t) in members {
            let (a, sz) = self.layout(t).unwrap_or((4, 4));
            off = round_up(a, off);
            ms.push(StructMember { name: String::from(mname), ty: t, offset: off, size: sz, align: a, io: Io::default(), span: Default::default() });
            off += sz;
            align = align.max(a);
        }
        let s = Struct { name, members: ms, align, size: round_up(align, off), runtime_sized: false, builtin: Some(b), span: Default::default() };
        let t = self.add_struct(s);
        self.builtin_structs.insert(b, t);
        t
    }

    // ---- shapes

    /// the scalar of a scalar, vector or matrix
    pub fn scalar_of(&self, t: TypeId) -> Option<Scalar> {
        match self.get(t) {
            Type::Scalar(s) | Type::Vector(_, s) | Type::Matrix(_, _, s) => Some(s),
            _ => None,
        }
    }

    /// 1 for a scalar, N for vecN, None otherwise
    pub fn width(&self, t: TypeId) -> Option<u8> {
        match self.get(t) {
            Type::Scalar(_) => Some(1),
            Type::Vector(n, _) => Some(n),
            _ => None,
        }
    }

    pub fn is_scalar(&self, t: TypeId) -> bool {
        matches!(self.get(t), Type::Scalar(_))
    }

    /// the store type of a reference or pointer
    pub fn store_of(&self, t: TypeId) -> Option<(AddressSpace, TypeId, Access)> {
        match self.get(t) {
            Type::Reference { space, store, access } | Type::Pointer { space, store, access } => Some((space, store, access)),
            _ => None,
        }
    }

    pub fn is_ref(&self, t: TypeId) -> bool {
        matches!(self.get(t), Type::Reference { .. })
    }

    pub fn is_ptr(&self, t: TypeId) -> bool {
        matches!(self.get(t), Type::Pointer { .. })
    }

    pub fn is_handle(&self, t: TypeId) -> bool {
        matches!(self.get(t), Type::Texture(..) | Type::Sampler { .. })
    }

    // ---- properties (spec 6.2 / 6.5)

    pub fn is_abstract(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Scalar(s) | Type::Vector(_, s) | Type::Matrix(_, _, s) => s.is_abstract(),
            Type::Array { elem, .. } => self.is_abstract(elem),
            Type::Struct(i) => matches!(
                self.structs.get(i as usize).and_then(|s| s.builtin),
                Some(BuiltinStruct::Frexp(_, Scalar::AbstractFloat)) | Some(BuiltinStruct::Modf(_, Scalar::AbstractFloat))
            ),
            _ => false,
        }
    }

    pub fn is_constructible(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..) => true,
            Type::Array { elem, count: ArrayCount::Fixed(_), .. } => self.is_constructible(elem),
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.members.iter().all(|m| self.is_constructible(m.ty))),
            _ => false,
        }
    }

    pub fn is_plain(&self, t: TypeId) -> bool {
        matches!(self.get(t), Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..) | Type::Atomic(_) | Type::Array { .. } | Type::Struct(_))
    }

    pub fn is_host_shareable(&self, t: TypeId) -> bool {
        if self.is_abstract(t) {
            return false;
        }
        match self.get(t) {
            Type::Scalar(s) | Type::Vector(_, s) => s.is_numeric(),
            Type::Matrix(..) | Type::Atomic(_) => true,
            Type::Array { elem, count, .. } => {
                matches!(count, ArrayCount::Fixed(_) | ArrayCount::Runtime) && self.is_host_shareable(elem)
            }
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.members.iter().all(|m| self.is_host_shareable(m.ty))),
            _ => false,
        }
    }

    pub fn creation_fixed(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..) | Type::Atomic(_) => true,
            Type::Array { elem, count: ArrayCount::Fixed(_), .. } => self.creation_fixed(elem),
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.members.iter().all(|m| self.creation_fixed(m.ty))),
            _ => false,
        }
    }

    pub fn fixed_footprint(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Array { elem, count, .. } => count != ArrayCount::Runtime && self.creation_fixed(elem),
            _ => self.creation_fixed(t),
        }
    }

    pub fn contains_atomic(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Atomic(_) => true,
            Type::Array { elem, .. } => self.contains_atomic(elem),
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.members.iter().any(|m| self.contains_atomic(m.ty))),
            _ => false,
        }
    }

    pub fn contains_runtime_array(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Array { count: ArrayCount::Runtime, .. } => true,
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.runtime_sized),
            _ => false,
        }
    }

    /// arrays or structs with array members (excluded from var<immediate>)
    pub fn contains_array(&self, t: TypeId) -> bool {
        match self.get(t) {
            Type::Array { .. } => true,
            Type::Struct(i) => self.structs.get(i as usize).is_some_and(|s| s.members.iter().any(|m| self.contains_array(m.ty))),
            _ => false,
        }
    }

    // ---- conversions (6.1.2)

    pub fn rank(&self, src: TypeId, dst: TypeId) -> u32 {
        if src == dst {
            return 0;
        }
        match (self.get(src), self.get(dst)) {
            (Type::Reference { store, access, .. }, _) if store == dst && access.readable() => 0,
            (Type::SwizzleView { elem, k, .. }, Type::Vector(n, s)) if elem == s && k == n => 0,
            (Type::Scalar(a), Type::Scalar(b)) => scalar_rank(a, b),
            (Type::Vector(n, a), Type::Vector(m, b)) if n == m => scalar_rank(a, b),
            (Type::Matrix(c, r, a), Type::Matrix(c2, r2, b)) if c == c2 && r == r2 => scalar_rank(a, b),
            (Type::Array { elem: a, count: ca, .. }, Type::Array { elem: b, count: cb, .. }) if ca == cb && matches!(ca, ArrayCount::Fixed(_)) => self.rank(a, b),
            (Type::Struct(a), Type::Struct(b)) => {
                let (sa, sb) = (self.structs.get(a as usize).and_then(|s| s.builtin), self.structs.get(b as usize).and_then(|s| s.builtin));
                match (sa, sb) {
                    (Some(BuiltinStruct::Frexp(n, Scalar::AbstractFloat)), Some(BuiltinStruct::Frexp(m, s)))
                    | (Some(BuiltinStruct::Modf(n, Scalar::AbstractFloat)), Some(BuiltinStruct::Modf(m, s)))
                        if n == m =>
                    {
                        match s {
                            Scalar::F32 => 1,
                            Scalar::F16 => 2,
                            _ => INF,
                        }
                    }
                    _ => INF,
                }
            }
            _ => INF,
        }
    }

    /// the concrete type an abstract (or reference / swizzle view) type turns into
    pub fn concretize(&mut self, t: TypeId) -> TypeId {
        match self.get(t) {
            Type::Reference { store, .. } => store,
            Type::SwizzleView { elem, k, .. } => self.vec(k, elem),
            Type::Scalar(s) => conc(s).id(),
            Type::Vector(n, s) => self.vec(n, conc(s)),
            Type::Matrix(c, r, s) => self.mat(c, r, conc(s)),
            Type::Array { elem, count, .. } if self.is_abstract(elem) => {
                let e = self.concretize(elem);
                self.array(e, count)
            }
            Type::Struct(i) => match self.structs.get(i as usize).and_then(|s| s.builtin) {
                Some(BuiltinStruct::Frexp(n, Scalar::AbstractFloat)) => self.builtin_struct(BuiltinStruct::Frexp(n, Scalar::F32)),
                Some(BuiltinStruct::Modf(n, Scalar::AbstractFloat)) => self.builtin_struct(BuiltinStruct::Modf(n, Scalar::F32)),
                _ => t,
            },
            _ => t,
        }
    }

    /// t with every scalar replaced by s (same shape)
    pub fn with_scalar(&mut self, t: TypeId, s: Scalar) -> TypeId {
        match self.get(t) {
            Type::Scalar(_) => s.id(),
            Type::Vector(n, _) => self.vec(n, s),
            Type::Matrix(c, r, _) => self.mat(c, r, s),
            Type::Array { elem, count, .. } => {
                let e = self.with_scalar(elem, s);
                self.array(e, count)
            }
            _ => t,
        }
    }

    // ---- layout (14.4)

    /// (AlignOf, SizeOf); runtime-sized arrays have size 0, structs their stored size
    pub fn layout(&self, t: TypeId) -> Option<(u32, u32)> {
        match self.get(t) {
            Type::Scalar(s) => Some(match s {
                Scalar::F16 => (2, 2),
                Scalar::AbstractInt | Scalar::AbstractFloat => (8, 8),
                _ => (4, 4),
            }),
            Type::Atomic(i) => self.layout(i),
            Type::Vector(n, s) => {
                let e = s.size();
                let align = if n == 2 { 2 * e } else { 4 * e };
                Some((align, n as u32 * e))
            }
            Type::Matrix(c, r, s) => {
                let (a, sz) = self.layout(self.vec(r, s))?;
                Some((a, c as u32 * round_up(a, sz)))
            }
            Type::Array { elem, count, stride } => {
                let (a, _) = self.layout(elem)?;
                let n = match count {
                    ArrayCount::Fixed(n) => n,
                    _ => 0,
                };
                Some((a, n.saturating_mul(stride)))
            }
            Type::Struct(i) => self.structs.get(i as usize).map(|s| (s.align, s.size)),
            _ => None,
        }
    }

    /// RequiredAlignOf(t, space) (14.4.5)
    pub fn required_align(&self, t: TypeId, uniform_strict: bool) -> u32 {
        let a = self.layout(t).map(|l| l.0).unwrap_or(1);
        match self.get(t) {
            Type::Array { .. } | Type::Struct(_) if uniform_strict => round_up(16, a),
            _ => a,
        }
    }

    /// the first violation of the layout constraints of an address space (uniform_strict:
    /// the uniform rules without uniform_buffer_standard_layout), None when t satisfies them
    pub fn layout_error(&self, t: TypeId, uniform_strict: bool) -> Option<(String, Span)> {
        match self.get(t) {
            Type::Array { elem, stride, .. } => {
                let ra = self.required_align(elem, uniform_strict);
                if uniform_strict && stride % 16 != 0 {
                    return Some((
                        format!(
                            "im uniform-Adressraum muss der Abstand der Array-Elemente ein Vielfaches von 16 sein, `{}` hat {} (setze z. B. @size(16) an ein Struct-Member oder nimm vec4)",
                            self.name(t),
                            stride
                        ),
                        Span::default(),
                    ));
                }
                if ra > 0 && stride % ra != 0 {
                    return Some((format!("der Abstand {} der Elemente von `{}` ist kein Vielfaches von {}", stride, self.name(t), ra), Span::default()));
                }
                self.layout_error(elem, uniform_strict)
            }
            Type::Struct(i) => {
                let s = self.structs.get(i as usize)?;
                for (k, m) in s.members.iter().enumerate() {
                    let ra = self.required_align(m.ty, uniform_strict);
                    if ra > 0 && m.offset % ra != 0 {
                        return Some((
                            format!(
                                "das Member `{}` von `{}` liegt bei Offset {}, braucht aber ein Vielfaches von {}{}",
                                m.name,
                                s.name,
                                m.offset,
                                ra,
                                if uniform_strict { " (uniform-Adressraum; @align(16) hilft)" } else { "" }
                            ),
                            m.span,
                        ));
                    }
                    if uniform_strict {
                        if let (Type::Struct(_), Some(next)) = (self.get(m.ty), s.members.get(k + 1)) {
                            let need = round_up(16, self.layout(m.ty).map(|l| l.1).unwrap_or(0));
                            if next.offset - m.offset < need {
                                return Some((
                                    format!(
                                        "im uniform-Adressraum muss nach dem Struct-Member `{}` von `{}` ein Abstand von {} Byte bis zum nächsten Member liegen (setze @align(16) an `{}`)",
                                        m.name, s.name, need, next.name
                                    ),
                                    next.span,
                                ));
                            }
                        }
                    }
                    if let Some(e) = self.layout_error(m.ty, uniform_strict) {
                        return Some(e);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

pub fn round_up(a: u32, x: u32) -> u32 {
    if a <= 1 { x } else { x.div_ceil(a).saturating_mul(a) }
}

pub fn conc(s: Scalar) -> Scalar {
    match s {
        Scalar::AbstractInt => Scalar::I32,
        Scalar::AbstractFloat => Scalar::F32,
        s => s,
    }
}

pub fn scalar_rank(a: Scalar, b: Scalar) -> u32 {
    use Scalar::*;
    if a == b {
        return 0;
    }
    match (a, b) {
        (AbstractFloat, F32) => 1,
        (AbstractFloat, F16) => 2,
        (AbstractInt, I32) => 3,
        (AbstractInt, U32) => 4,
        (AbstractInt, AbstractFloat) => 5,
        (AbstractInt, F32) => 6,
        (AbstractInt, F16) => 7,
        _ => INF,
    }
}

use crate::ast::Span;
