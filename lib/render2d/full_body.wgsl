    // (the body of fs_full and of full_body, written once: lib.rs puts it into both; see fs_full's head in stroke.wgsl)
    let q = i.pos.xy;
    let h = i.par.x;
    let fl = i.misc.x;
    let col = unpack4x8unorm(i.misc.y);
    let cap = (fl >> 4u) & 15u;
    let dashed = (fl & M_DASHED) != 0u;
    let arc = (fl & M_ARC) != 0u;
    let si = i.misc.z;
    // the pattern drawn exactly, as a flat tone of its duty (its caps counted as the area they add), or mixed (tf; a
    // solid arc: 1)
    var sc = 1.0;
    var tf = 1.0;
    var duty = 1.0;
    if dashed {
        // (dash units per pixel: the vertex shader's, the block's units inside an insert)
        sc = select(1.0, bitcast<f32>(i.misc.w), ((styles[si].flags >> 20u) & 15u) == UNIT_DRAWING);
        let period = styles[si].period;
        tf = clamp((DASH_EXACT - period / sc) / (DASH_EXACT - DASH_FLAT), 0.0, 1.0);
        let capl = select(select(0.0, 2.0 * h, cap == CAP_SQUARE), 1.5707963 * h, cap == CAP_ROUND) * sc;
        duty = min(1.0, styles[si].duty + styles[si].ons * capl / period);
    }
    var g: Ring;
    g.m = vec2f(1.0, 0.0);
    g.t = vec2f(0.0, 1.0);
    g.dist = 0.0;
    g.r = 1.0;
    var d = vec2f(1.0, 0.0);
    var dp = vec2f(0.0);
    var dn = vec2f(0.0);
    var near_s = false;
    var near_e = false;
    // the solid stroke's coverage where no end is near (its body): the strip alone
    var body = -1.0;
    if arc {
        // the circle from F (see the head)
        let f = i.seg.xy;
        let up = i.seg.zw;
        let rp = i.par.y;
        let orient = i.par.z;
        let qf = q - f;
        let wv = qf + rp * up;
        let lw = length(wv);
        g.dist = (dot(qf, qf) + 2.0 * rp * dot(qf, up)) / (lw + rp);
        if lw > 0.0 {
            g.m = wv / lw;
        } else {
            g.m = up;
        }
        g.t = orient * vec2f(-g.m.y, g.m.x);
        g.r = rp;
        let da = unit_or_zero((i.nb.xy - f) + rp * up);
        let db = unit_or_zero((i.nb.zw - f) + rp * up);
        let sa = dot(q - i.nb.xy, orient * vec2f(-da.y, da.x));
        let sb = dot(q - i.nb.zw, orient * vec2f(-db.y, db.x));
        let capa = (fl & 4096u) != 0u;
        let capb = (fl & 8192u) != 0u;
        if !capa && sa < 0.0 {
            discard;
            return vec4f(0.0);
        }
        if !capb && sb >= 0.0 {
            discard;
            return vec4f(0.0);
        }
        let reach = 0.71 + select(h, 0.0, cap == CAP_BUTT);
        if !((capa && sa < reach) || (capb && sb > -reach)) {
            body = max(cov(g.dist - h, g.m) + select(1.0, cov(-g.dist - h, -g.m), rp > h) - 1.0, 0.0);
        }
    } else {
        let s = i.seg.xy;
        let e = i.seg.zw;
        let sk = fl & 3u;
        let ek = (fl >> 2u) & 3u;
        let jk = (fl >> 8u) & 15u;
        let own = i.par.zw;
        // a pixel near a join is the nearer segment's (the incoming one's when equal): see the head
        var dprev = 3.4e38;
        var dnext = 3.4e38;
        if sk == JOIN {
            dprev = dist_seg(q - s, i.nb.xy);
            if !(dist_seg(q - s, own) < dprev) {
                discard;
                return vec4f(0.0);
            }
        }
        if ek == JOIN {
            dnext = dist_seg(q - e, i.nb.zw);
            if !(dist_seg(q - e, -own) <= dnext) {
                discard;
                return vec4f(0.0);
            }
        }
        d = unit_or_zero(own);
        if !any(d != vec2f(0.0)) {
            d = vec2f(1.0, 0.0);
        }
        dp = unit_or_zero(-i.nb.xy);
        dn = unit_or_zero(i.nb.zw);
        // which pieces may touch the pixel (as in fs)
        let rr = h * 1.415 + 0.71;
        if sk == JOIN {
            let cs = dot(dp, d);
            let wr = h * select(1.0, max(sqrt(2.0 / max(1.0 + cs, 1e-6)), 1.0), jk == JOIN_MITER && miter_ok(cs, i.par.y)) + 0.71;
            near_s = dprev < rr * rr || length(q - s) < wr;
        }
        if ek == JOIN {
            let cs = dot(d, dn);
            let wr = h * select(1.0, max(sqrt(2.0 / max(1.0 + cs, 1e-6)), 1.0), jk == JOIN_MITER && miter_ok(cs, i.par.y)) + 0.71;
            near_e = dnext < rr * rr || length(q - e) < wr;
        }
        let u0 = dot(q - s, d);
        let u1 = dot(q - e, d);
        near_s = near_s || (sk == JOIN && u0 < 0.71);
        near_e = near_e || (ek == JOIN && u1 > -0.71);
        let lim = 0.71 + select(0.0, h, cap != CAP_BUTT);
        let sl = strip_l(q, s, d, h);
        let sr = strip_r(q, s, d, h);
        if !near_s && !near_e && !(sk == CAP && u0 < lim) && !(ek == CAP && u1 > -lim) {
            body = max(cov(sl.z, sl.xy) + cov(sr.z, sr.xy) - 1.0, 0.0);
        } else if !dashed && h <= TINY_H && length(own) <= TINY_LEN {
            // short thin strokes near their ends: as in fs
            var cn = 0.0;
            var weight = 1.0;
            if near_s {
                let sm = dp + d;
                let ls = length(sm);
                let m = select(vec2f(-dp.y, dp.x), sm / ls, ls > 1e-6);
                let b = cov(-dot(q - s, m), m);
                let np = vec2f(-dp.y, dp.x);
                let vp = dot(q - s, np);
                cn += (1.0 - b) * max(cov(vp - h, np) + cov(-vp - h, -np) - 1.0, 0.0);
                weight *= b;
            }
            if near_e {
                let sm = d + dn;
                let ls = length(sm);
                let m = select(vec2f(-d.y, d.x), sm / ls, ls > 1e-6);
                let b = cov(dot(q - e, m), -m);
                let nx = vec2f(-dn.y, dn.x);
                let vn = dot(q - e, nx);
                cn += (1.0 - b) * max(cov(vn - h, nx) + cov(-vn - h, -nx) - 1.0, 0.0);
                weight *= b;
            }
            var o = max(cov(sl.z, sl.xy) + cov(sr.z, sr.xy) - 1.0, 0.0);
            let ext = select(select(0.0, h, cap == CAP_SQUARE), 0.7854 * h, cap == CAP_ROUND);
            if sk == CAP {
                o *= cov(-u0 - ext, -d);
            }
            if ek == CAP {
                o *= cov(u1 - ext, d);
            }
            body = clamp(cn + weight * o, 0.0, 1.0);
        }
    }
    // the pattern (pass 0), the solid stroke near its ends (pass 1): one call of pieces (inlined once)
    var c = max(body, 0.0);
    var cd = 0.0;
    var passes = select(0u, 1u, dashed && tf < 1.0) | select(0u, 2u, tf > 0.0 && body < 0.0);
    loop {
        if passes == 0u {
            break;
        }
        let pz = firstTrailingBit(passes);
        passes &= passes - 1u;
        let v = pieces(i, q, arc, pz == 1u, si, sc, d, dp, dn, near_s, near_e, g);
        if pz == 0u {
            cd = v;
        } else {
            c = v;
        }
    }
    if dashed {
        c = mix(cd, c * duty, tf);
    }
    if !(c > 0.0) {
        discard;
        return vec4f(0.0);
    }
    let al = col.w * c;
    return vec4f(vec3f(col.z, col.y, col.x) * al, al);
