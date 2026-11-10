// Halation develop pipeline — one compute pass over linear ProPhoto RGB, output display-encoded RGBA8.
//
// Mirrors halation-color::PreparedDevelop::develop_masked stage for stage (including extras.rs and
// adjust.rs). If you change a constant or an operator here, change it there too (and vice versa).
//
// Stage order: white balance -> exposure -> highlights/shadows -> whites/blacks -> contrast ->
//   tone curve -> colour grading -> parametric curve -> vibrance/saturation -> dehaze ->
//   texture/clarity/sharpen -> HSL mixer / B&W -> local adjustment (mask) ->
//   vignette/grain -> display transform (3D LUT, or matrix + gamut map + sRGB OETF) -> dither.
//
// Saved with a .txt suffix so it always previews / downloads; wgpu::include_wgsl! does not care.

struct DevelopUniforms {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    temperature: f32,
    tint: f32,
    vibrance: f32,
    saturation: f32,
    num_spline_segments: u32,
    _pad: f32,
};

// Column-major 3x3 with each column padded to 16 bytes (GpuMat3x3).
struct PaddedMat3 {
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
};

struct Matrices {
    white_balance: PaddedMat3,
    to_display: PaddedMat3,
};

struct ExtraUniforms {
    curve_counts: vec4<u32>, // segments for Rgb, Red, Green, Blue
    shadows: vec4<f32>,      // (dr, dg, db, dlum) gain terms per grading zone
    midtones: vec4<f32>,
    highlights: vec4<f32>,
    overall: vec4<f32>,
    local_a: vec4<f32>,      // exposure EV, contrast, highlights, shadows
    local_b: vec4<f32>,      // saturation, temperature, mask enabled, mask inverted
    flags: vec4<u32>,        // display LUT, dither, grading, clip overlay
    grading: vec4<f32>,      // (blending / 100, balance / 100, 0, 0)
};

// Must match halation_core::GpuAdjustUniforms (256 bytes, std140-safe).
struct AdjustUniforms {
    effects: vec4<f32>,  // texture (scaled), clarity, dehaze, image aspect
    vignette: vec4<f32>, // amount, midpoint, roundness, feather
    grain: vec4<f32>,    // amount, size, roughness, unused
    detail: vec4<f32>,   // sharpen (scaled), detail, masking, unused
    air: vec4<f32>,      // atmospheric light rgb, detail-map-present flag
    mixer_hue: array<vec4<f32>, 2>,
    mixer_sat: array<vec4<f32>, 2>,
    mixer_lum: array<vec4<f32>, 2>,
    bw_mix: array<vec4<f32>, 2>,
    parametric_amounts: vec4<f32>, // highlights, lights, darks, shadows (/100)
    parametric_split: vec4<f32>,   // shadow / midtone / highlight splits (/100)
    flags: vec4<u32>,              // any adjust, B&W enabled, mixer enabled, parametric enabled
};

struct SplineSegment {
    coeffs: vec4<f32>, // a, b, c, d  (polynomial in x - domain.x)
    domain: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: DevelopUniforms;
@group(0) @binding(1) var<uniform> m: Matrices;
@group(0) @binding(2) var input_tex: texture_2d<f32>;
@group(0) @binding(3) var output_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var<uniform> ex: ExtraUniforms;
@group(0) @binding(5) var<storage, read> curve_segments: array<SplineSegment>;
@group(0) @binding(6) var display_lut: texture_3d<f32>;
@group(0) @binding(7) var mask_tex: texture_2d<f32>;
@group(0) @binding(8) var mask_samp: sampler;
@group(0) @binding(9) var<uniform> am: AdjustUniforms;
@group(0) @binding(10) var detail_tex: texture_2d<f32>;

// ProPhoto RGB linear luminance row (sums to 1.0). Rec.709 weights are only used on linear sRGB below.
const PROPHOTO_LUMA: vec3<f32> = vec3<f32>(0.28804, 0.71187, 0.00009);
const REC709_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
const MID_GRAY: f32 = 0.18;
const HIGHLIGHT_SHADOW_RANGE_EV: f32 = 1.5;
const MIN_LUMA: f32 = 1.0e-6;
const MAX_SEGMENTS: u32 = 16u;
const SHAPER_GAMMA: f32 = 2.4;
const TAU: f32 = 6.283185307179586;
const SQRT_2: f32 = 1.4142135623730951;
const BAND_COUNT: u32 = 8u;
const BAND_HUES: array<f32, 8> = array<f32, 8>(0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0);

fn mat3_from(p: PaddedMat3) -> mat3x3<f32> {
    return mat3x3<f32>(p.c0.xyz, p.c1.xyz, p.c2.xyz);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, PROPHOTO_LUMA);
}

fn srgb_oetf(l: vec3<f32>) -> vec3<f32> {
    let lo = l * 12.92;
    let hi = 1.055 * pow(l, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, l <= vec3<f32>(0.0031308));
}

fn srgb_oetf1(l: f32) -> f32 {
    if (l <= 0.0031308) {
        return l * 12.92;
    }
    return 1.055 * pow(l, 1.0 / 2.4) - 0.055;
}

fn srgb_eotf1(v: f32) -> f32 {
    if (v <= 0.04045) {
        return v / 12.92;
    }
    return pow((v + 0.055) / 1.055, 2.4);
}

// ---- highlights / shadows -------------------------------------------------------------------

fn tone_hs(c: vec3<f32>, highlights_ev: f32, shadows_ev: f32) -> vec3<f32> {
    let l = dot(c, PROPHOTO_LUMA);
    if (l <= MIN_LUMA) {
        return c;
    }
    let stops = log2(l / MID_GRAY);
    let w_shadow = 1.0 - smoothstep(-6.0, 0.0, stops);
    let w_highlight = smoothstep(0.0, 3.0, stops);
    return c * exp2(shadows_ev * w_shadow + highlights_ev * w_highlight);
}

fn contrast_pow(c: vec3<f32>, gamma: f32) -> vec3<f32> {
    return MID_GRAY * pow(max(c, vec3<f32>(1.0e-10)) / MID_GRAY, vec3<f32>(gamma));
}

fn vib_sat(c: vec3<f32>, vibrance: f32, saturation: f32) -> vec3<f32> {
    let l = dot(c, PROPHOTO_LUMA);
    let max_c = max(c.r, max(c.g, c.b));
    let min_c = min(c.r, min(c.g, c.b));
    let current = (max_c - min_c) / (max_c + 0.001);
    let gain = (1.0 + vibrance * (1.0 - current)) * (1.0 + saturation);
    return max(vec3<f32>(l) + (c - vec3<f32>(l)) * gain, vec3<f32>(0.0));
}

// ---- tone curve (analytic cubic segments on the sRGB-encoded axis) ----------------------------

fn eval_curve(channel: u32, x: f32) -> f32 {
    let n = ex.curve_counts[channel];
    let base = channel * MAX_SEGMENTS;
    let xc = clamp(x, 0.0, 1.0);
    for (var i = 0u; i < n; i = i + 1u) {
        let seg = curve_segments[base + i];
        if (xc >= seg.domain.x && xc <= seg.domain.y) {
            let t = xc - seg.domain.x;
            return ((seg.coeffs.x * t + seg.coeffs.y) * t + seg.coeffs.z) * t + seg.coeffs.w;
        }
    }
    return xc;
}

fn curve_channel(v: f32, channel: u32) -> f32 {
    if (v > 1.0) {
        return v;
    }
    var e = srgb_oetf1(max(v, 0.0));
    if (ex.curve_counts.x > 0u) {
        e = eval_curve(0u, e);
    }
    if (ex.curve_counts[channel] > 0u) {
        e = eval_curve(channel, e);
    }
    return srgb_eotf1(e);
}

// ---- 3-way colour grading ----------------------------------------------------------------------

fn grade(c: vec3<f32>) -> vec3<f32> {
    let l = dot(c, PROPHOTO_LUMA);
    let spread = 0.5 + clamp(ex.grading.x, 0.0, 1.0);
    let shift = clamp(ex.grading.y, -1.0, 1.0) * 2.0;
    var ws = 1.0;
    var wm = 0.0;
    var wh = 0.0;
    if (l > MIN_LUMA) {
        let s = log2(l / MID_GRAY);
        ws = 1.0 - smoothstep(-4.0 * spread - shift, -shift, s);
        wh = smoothstep(-shift, 3.0 * spread - shift, s);
        wm = max(1.0 - ws - wh, 0.0);
    }
    let gain = vec3<f32>(1.0)
        + (ex.shadows.xyz + vec3<f32>(ex.shadows.w)) * ws
        + (ex.midtones.xyz + vec3<f32>(ex.midtones.w)) * wm
        + (ex.highlights.xyz + vec3<f32>(ex.highlights.w)) * wh
        + (ex.overall.xyz + vec3<f32>(ex.overall.w));
    return max(c * gain, vec3<f32>(0.0));
}

// ---- parametric curve --------------------------------------------------------------------------

fn parametric_channel(v: f32) -> f32 {
    if (v > 1.0) {
        return v;
    }
    let e = srgb_oetf1(max(v, 0.0));
    let amt = am.parametric_amounts;
    let s = am.parametric_split;
    // regions: shadows [0,s0], darks [s0,s1], lights [s1,s2], highlights [s2,1]
    var shift = region_shift(e, 0.0, s.x, amt.w);
    shift += region_shift(e, s.x, s.y, amt.z);
    shift += region_shift(e, s.y, s.z, amt.y);
    shift += region_shift(e, s.z, 1.0, amt.x);
    return srgb_eotf1(clamp(e + shift, 0.0, 1.0));
}

fn region_shift(e: f32, a: f32, b: f32, amount: f32) -> f32 {
    if (e > a && e < b) {
        let t = (e - a) / (b - a);
        return amount * 0.3 * (b - a) * 0.5 * (1.0 - cos(TAU * t));
    }
    return 0.0;
}

fn parametric(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(parametric_channel(c.x), parametric_channel(c.y), parametric_channel(c.z));
}

// ---- dehaze / texture / clarity / sharpening ---------------------------------------------------

fn dehaze_stage(c: vec3<f32>, dark: f32) -> vec3<f32> {
    let amount = am.effects.z;
    if (amount == 0.0) {
        return c;
    }
    let a = am.air.rgb;
    if (amount > 0.0) {
        let t = clamp(1.0 - 0.95 * amount * clamp(dark, 0.0, 1.0), 0.15, 1.0);
        return max((c - a) / t + a, vec3<f32>(0.0));
    }
    let k = -amount * 0.6;
    return c * (1.0 - k) + a * k;
}

fn detail_stage(c: vec3<f32>, d: vec4<f32>) -> vec3<f32> {
    let texture_amt = am.effects.x;
    let clarity = am.effects.y;
    let sharpen = am.detail.x;
    if (texture_amt == 0.0 && clarity == 0.0 && sharpen == 0.0) {
        return c;
    }
    let y = max(luma(c), MIN_LUMA);
    let e = srgb_oetf1(min(y, 1.0));
    let mid = 4.0 * e * (1.0 - e);
    var stops = texture_amt * d.x + clarity * d.y * (0.35 + 0.65 * mid);
    if (sharpen > 0.0) {
        let edge = smoothstep(0.02, 0.15, abs(d.x));
        let mask = 1.0 + (edge - 1.0) * am.detail.z;
        let signal = d.z + (d.x - d.z) * am.detail.y * 0.6;
        stops += sharpen * signal * mask;
    }
    return c * exp2(clamp(stops, -3.0, 3.0));
}

// ---- HSL mixer / B&W ---------------------------------------------------------------------------

fn lane(a: vec4<f32>, b: vec4<f32>, i: u32) -> f32 {
    let v = select(a, b, i >= 4u);
    let j = i % 4u;
    if (j == 0u) {
        return v.x;
    }
    if (j == 1u) {
        return v.y;
    }
    if (j == 2u) {
        return v.z;
    }
    return v.w;
}

fn rgb_to_hsl(c: vec3<f32>) -> vec3<f32> {
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    let l = 0.5 * (mx + mn);
    let d = mx - mn;
    if (d < 1.0e-6) {
        return vec3<f32>(0.0, 0.0, l);
    }
    let s = d / max(1.0 - abs(2.0 * l - 1.0), 1.0e-6);
    var h = 0.0;
    if (mx == c.r) {
        let raw = (c.g - c.b) / d;
        h = raw - floor(raw / 6.0) * 6.0;
    } else if (mx == c.g) {
        h = (c.b - c.r) / d + 2.0;
    } else {
        h = (c.r - c.g) / d + 4.0;
    }
    return vec3<f32>(h * 60.0, clamp(s, 0.0, 1.0), l);
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> vec3<f32> {
    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let h360 = h - floor(h / 360.0) * 360.0;
    let hp = h360 / 60.0;
    let hp2 = hp - floor(hp / 2.0) * 2.0;
    let x = c * (1.0 - abs(hp2 - 1.0));
    let sector = u32(hp);
    var rgb = vec3<f32>(c, 0.0, x);
    if (sector == 0u) {
        rgb = vec3<f32>(c, x, 0.0);
    } else if (sector == 1u) {
        rgb = vec3<f32>(x, c, 0.0);
    } else if (sector == 2u) {
        rgb = vec3<f32>(0.0, c, x);
    } else if (sector == 3u) {
        rgb = vec3<f32>(0.0, x, c);
    } else if (sector == 4u) {
        rgb = vec3<f32>(x, 0.0, c);
    }
    let m = l - 0.5 * c;
    return rgb + vec3<f32>(m);
}

fn band_weight(hue: f32, i: u32) -> f32 {
    let h = hue - floor(hue / 360.0) * 360.0;
    var w = 0.0;
    // Weight as the left endpoint of arc `i`.
    let lo = BAND_HUES[i];
    let next_hi = BAND_HUES[(i + 1u) % BAND_COUNT];
    let hi = select(next_hi, next_hi + 360.0, next_hi <= lo);
    let hh = select(h, h + 360.0, h < lo);
    if (hh >= lo && hh < hi) {
        w += 1.0 - (hh - lo) / (hi - lo);
    }
    // Weight as the right endpoint of the previous arc.
    let p = (i + BAND_COUNT - 1u) % BAND_COUNT;
    let p_lo = BAND_HUES[p];
    let p_hi0 = BAND_HUES[i];
    let p_hi = select(p_hi0, p_hi0 + 360.0, p_hi0 <= p_lo);
    let p_h = select(h, h + 360.0, h < p_lo);
    if (p_h >= p_lo && p_h < p_hi) {
        w += (p_h - p_lo) / (p_hi - p_lo);
    }
    return w;
}

fn mixer_bw(c: vec3<f32>) -> vec3<f32> {
    if (am.flags.y == 0u && am.flags.z == 0u) {
        return c;
    }
    if (max(c.r, max(c.g, c.b)) > 1.0) {
        return c;
    }
    let e = vec3<f32>(srgb_oetf1(max(c.x, 0.0)), srgb_oetf1(max(c.y, 0.0)), srgb_oetf1(max(c.z, 0.0)));
    let hsl = rgb_to_hsl(e);
    let h = hsl.x;
    let s = hsl.y;
    let l = hsl.z;
    var wsum = 0.0;
    for (var i = 0u; i < BAND_COUNT; i = i + 1u) {
        wsum += band_weight(h, i);
    }
    let inv = 1.0 / max(wsum, 1.0e-6);
    if (am.flags.y == 1u) {
        let y = luma(c);
        var gain = 0.0;
        for (var i = 0u; i < BAND_COUNT; i = i + 1u) {
            gain += band_weight(h, i) * lane(am.bw_mix[0], am.bw_mix[1], i);
        }
        gain *= inv;
        let g = max(y * max(1.0 + s * gain * 0.8, 0.0), 0.0);
        return vec3<f32>(g);
    }
    var dh = 0.0;
    var ds = 0.0;
    var dl = 0.0;
    for (var i = 0u; i < BAND_COUNT; i = i + 1u) {
        let w = band_weight(h, i);
        dh += w * lane(am.mixer_hue[0], am.mixer_hue[1], i);
        ds += w * lane(am.mixer_sat[0], am.mixer_sat[1], i);
        dl += w * lane(am.mixer_lum[0], am.mixer_lum[1], i);
    }
    dh *= 30.0 * inv;
    ds *= inv;
    dl *= inv;
    let s2 = clamp(s * (1.0 + ds), 0.0, 1.0);
    let l2 = clamp(l + dl * 0.25 * s * (1.0 - abs(2.0 * l - 1.0)), 0.0, 1.0);
    let out = hsl_to_rgb(h + dh, s2, l2);
    return vec3<f32>(
        srgb_eotf1(clamp(out.x, 0.0, 1.0)),
        srgb_eotf1(clamp(out.y, 0.0, 1.0)),
        srgb_eotf1(clamp(out.z, 0.0, 1.0)),
    );
}

// ---- local adjustment ----------------------------------------------------------------------------

fn local_adjust(c: vec3<f32>, mask: f32) -> vec3<f32> {
    var k = mask;
    if (ex.local_b.w > 0.5) {
        k = 1.0 - mask;
    }
    if (k <= 0.0) {
        return c;
    }
    let range = HIGHLIGHT_SHADOW_RANGE_EV / 100.0;
    var a = c * exp2(clamp(ex.local_a.x, -5.0, 5.0));
    a = tone_hs(a, clamp(ex.local_a.z, -100.0, 100.0) * range, clamp(ex.local_a.w, -100.0, 100.0) * range);
    let gamma = 1.0 + clamp(ex.local_a.y, -100.0, 100.0) * 0.005;
    if (gamma != 1.0) {
        a = contrast_pow(a, gamma);
    }
    let saturation = clamp(ex.local_b.x, -100.0, 100.0) * 0.01;
    if (saturation != 0.0) {
        a = vib_sat(a, 0.0, saturation);
    }
    let warm = clamp(ex.local_b.y, -100.0, 100.0) * 0.01;
    if (warm != 0.0) {
        a = a * vec3<f32>(1.0 + 0.25 * warm, 1.0, 1.0 - 0.25 * warm);
    }
    return mix(c, a, min(k, 1.0));
}

// ---- vignette & grain --------------------------------------------------------------------------

fn hash(x_in: u32) -> u32 {
    var x = x_in;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn hash2(x: u32, y: u32) -> f32 {
    return f32(hash(x ^ hash(y + 0x9e3779b9u))) / 4294967295.0;
}

fn vignette_grain(c_in: vec3<f32>, px: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    var c = c_in;
    let vig = am.vignette;
    if (vig.x != 0.0) {
        let u = (px.x + 0.5) / max(size.x, 1.0);
        let v = (px.y + 0.5) / max(size.y, 1.0);
        var x = (u - 0.5) * 2.0;
        var y = (v - 0.5) * 2.0;
        let t = (vig.z + 1.0) * 0.5;
        let aspect = max(am.effects.w, 1.0e-4);
        if (aspect >= 1.0) {
            x *= 1.0 + (aspect - 1.0) * t;
        } else {
            y *= 1.0 + (1.0 / aspect - 1.0) * t;
        }
        let norm = max(1.0 + (max(aspect, 1.0 / aspect) - 1.0) * t, 1.0);
        let d = sqrt(x * x + y * y) / (SQRT_2 * norm);
        let inner = vig.y * 0.9;
        let mask = smoothstep(inner, inner + 0.1 + 0.9 * vig.w, d);
        c *= exp2(2.0 * vig.x * mask);
    }
    let grain = am.grain;
    if (grain.x > 0.0) {
        let size_px = 1.0 + grain.y * 3.0;
        let white = hash2(u32(px.x / size_px), u32(px.y / size_px)) * 2.0 - 1.0;
        let gx = px.x / (size_px * 2.0);
        let gy = px.y / (size_px * 2.0);
        let ix = floor(gx);
        let iy = floor(gy);
        let fx = smoothstep(0.0, 1.0, gx - ix);
        let fy = smoothstep(0.0, 1.0, gy - iy);
        let x0 = u32(ix);
        let y0 = u32(iy);
        let a = hash2(x0, y0);
        let b = hash2(x0 + 1u, y0);
        let cc = hash2(x0, y0 + 1u);
        let d = hash2(x0 + 1u, y0 + 1u);
        let top = a + (b - a) * fx;
        let bottom = cc + (d - cc) * fx;
        let smooth_n = (top + (bottom - top) * fy) * 2.0 - 1.0;
        let n = smooth_n + (white - smooth_n) * grain.z;
        let y = max(luma(c), MIN_LUMA);
        let e = srgb_oetf1(min(y, 1.0));
        let delta = n * grain.x * 0.08 * 4.0 * e * (1.0 - e);
        let gain = srgb_eotf1(clamp(e + delta, 0.0, 1.0)) / max(srgb_eotf1(e), 1.0e-6);
        c *= gain;
    }
    return c;
}

// ---- display LUT (tetrahedral interpolation, shaper = x^(1/2.4)) -----------------------------------

fn lut_texel(i: vec3<i32>, d: vec3<i32>) -> vec3<f32> {
    return textureLoad(display_lut, i + d, 0).rgb;
}

fn lut_apply(c: vec3<f32>) -> vec3<f32> {
    let size = textureDimensions(display_lut);
    let top = f32(size.x - 1u);
    let p = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / SHAPER_GAMMA)) * top;
    let base = min(floor(p), vec3<f32>(top - 1.0));
    let f = p - base;
    let i = vec3<i32>(base);
    let c000 = lut_texel(i, vec3<i32>(0, 0, 0));
    let c111 = lut_texel(i, vec3<i32>(1, 1, 1));
    var out: vec3<f32>;
    if (f.x >= f.y) {
        if (f.y >= f.z) {
            out = c000 + (lut_texel(i, vec3<i32>(1, 0, 0)) - c000) * f.x
                + (lut_texel(i, vec3<i32>(1, 1, 0)) - lut_texel(i, vec3<i32>(1, 0, 0))) * f.y
                + (c111 - lut_texel(i, vec3<i32>(1, 1, 0))) * f.z;
        } else if (f.x >= f.z) {
            out = c000 + (lut_texel(i, vec3<i32>(1, 0, 0)) - c000) * f.x
                + (lut_texel(i, vec3<i32>(1, 0, 1)) - lut_texel(i, vec3<i32>(1, 0, 0))) * f.z
                + (c111 - lut_texel(i, vec3<i32>(1, 0, 1))) * f.y;
        } else {
            out = c000 + (lut_texel(i, vec3<i32>(0, 0, 1)) - c000) * f.z
                + (lut_texel(i, vec3<i32>(1, 0, 1)) - lut_texel(i, vec3<i32>(0, 0, 1))) * f.x
                + (c111 - lut_texel(i, vec3<i32>(1, 0, 1))) * f.y;
        }
    } else if (f.z >= f.y) {
        out = c000 + (lut_texel(i, vec3<i32>(0, 0, 1)) - c000) * f.z
            + (lut_texel(i, vec3<i32>(0, 1, 1)) - lut_texel(i, vec3<i32>(0, 0, 1))) * f.y
            + (c111 - lut_texel(i, vec3<i32>(0, 1, 1))) * f.x;
    } else if (f.z >= f.x) {
        out = c000 + (lut_texel(i, vec3<i32>(0, 1, 0)) - c000) * f.y
            + (lut_texel(i, vec3<i32>(0, 1, 1)) - lut_texel(i, vec3<i32>(0, 1, 0))) * f.z
            + (c111 - lut_texel(i, vec3<i32>(0, 1, 1))) * f.x;
    } else {
        out = c000 + (lut_texel(i, vec3<i32>(0, 1, 0)) - c000) * f.y
            + (lut_texel(i, vec3<i32>(1, 1, 0)) - lut_texel(i, vec3<i32>(0, 1, 0))) * f.x
            + (c111 - lut_texel(i, vec3<i32>(1, 1, 0))) * f.z;
    }
    return clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Interleaved gradient noise: cheap, blue-ish spectrum, enough to break up 8-bit banding.
fn dither_noise(p: vec2<u32>) -> f32 {
    return fract(52.9829189 * fract(dot(vec2<f32>(p), vec2<f32>(0.06711056, 0.00583715))));
}

@compute @workgroup_size(16, 16)
fn cs_develop_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(input_tex);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let coord = vec2<i32>(id.xy);
    var c = textureLoad(input_tex, coord, 0).rgb;

    // 1. White balance (Bradford adaptation matrix, precomputed on the CPU).
    c = max(mat3_from(m.white_balance) * c, vec3<f32>(0.0));

    // 2. Exposure (linear energy scaling).
    c = c * exp2(clamp(u.exposure, -10.0, 10.0));

    // 3. Highlights / shadows.
    let range = HIGHLIGHT_SHADOW_RANGE_EV / 100.0;
    let highlights_ev = clamp(u.highlights, -100.0, 100.0) * range;
    let shadows_ev = clamp(u.shadows, -100.0, 100.0) * range;
    if (highlights_ev != 0.0 || shadows_ev != 0.0) {
        c = tone_hs(c, highlights_ev, shadows_ev);
    }

    // 4. Whites / blacks: black point -/+0.02, white point -/+0.25 (linear) at +/-100.
    let black_point = -clamp(u.blacks, -100.0, 100.0) * 0.0002;
    let white_point = 1.0 - clamp(u.whites, -100.0, 100.0) * 0.0025;
    let inv_range = 1.0 / max(white_point - black_point, 0.05);
    c = max((c - vec3<f32>(black_point)) * inv_range, vec3<f32>(0.0));

    // 5. Log-domain contrast about 18% gray.
    let gamma = 1.0 + clamp(u.contrast, -100.0, 100.0) * 0.005;
    if (gamma != 1.0) {
        c = contrast_pow(c, gamma);
    }

    // 6. Tone curve + 3-way grading.
    if (ex.curve_counts.x + ex.curve_counts.y + ex.curve_counts.z + ex.curve_counts.w > 0u) {
        c = vec3<f32>(curve_channel(c.r, 1u), curve_channel(c.g, 2u), curve_channel(c.b, 3u));
    }
    if (ex.flags.z == 1u) {
        c = grade(c);
    }

    // 7. Parametric curve.
    if (am.flags.w == 1u) {
        c = parametric(c);
    }

    // 8. Vibrance / saturation.
    let vibrance = clamp(u.vibrance, -100.0, 100.0) * 0.01;
    let saturation = clamp(u.saturation, -100.0, 100.0) * 0.01;
    if (vibrance != 0.0 || saturation != 0.0) {
        c = vib_sat(c, vibrance, saturation);
    }

    // 9. Neighbourhood stage: detail maps drive dehaze, texture, clarity and sharpening.
    if (am.flags.x == 1u) {
        var d = vec4<f32>(0.0);
        if (am.air.w > 0.5) {
            d = textureLoad(detail_tex, coord, 0);
        }
        c = dehaze_stage(c, d.w);
        c = detail_stage(c, d);
        c = mixer_bw(c);
    }

    // 10. Local adjustment through the mask raster (bilinear, normalised image space).
    if (ex.local_b.z > 0.5) {
        let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(dims);
        let mask = textureSampleLevel(mask_tex, mask_samp, uv, 0.0).r;
        c = local_adjust(c, mask);
    }

    // 11. Vignette / grain.
    if (am.flags.x == 1u) {
        c = vignette_grain(c, vec2<f32>(id.xy), vec2<f32>(dims));
    }

    // 12. Display transform.
    var shown: vec3<f32>;
    if (ex.flags.x == 1u) {
        shown = lut_apply(c);
    } else {
        var d = mat3_from(m.to_display) * c;
        let dl = dot(d, REC709_LUMA);
        let dmin = min(d.r, min(d.g, d.b));
        if (dmin < 0.0 && dl > 0.0) {
            d = vec3<f32>(dl) + (d - vec3<f32>(dl)) * (dl / (dl - dmin));
        }
        shown = srgb_oetf(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    if (ex.flags.y == 1u) {
        shown = shown + vec3<f32>((dither_noise(id.xy) - 0.5) / 255.0);
    }
    shown = clamp(shown, vec3<f32>(0.0), vec3<f32>(1.0));

    // 13. Clipping overlay: highlights red, shadows blue (same 8-bit thresholds as the CPU path).
    if (ex.flags.w == 1u) {
        let code = shown * 255.0 + vec3<f32>(0.5);
        if (max(code.r, max(code.g, code.b)) >= 254.0) {
            shown = vec3<f32>(1.0, 0.1333, 0.1333);
        } else if (min(code.r, min(code.g, code.b)) <= 1.0) {
            shown = vec3<f32>(0.1333, 0.3333, 1.0);
        }
    }

    textureStore(output_tex, coord, vec4<f32>(shown, 1.0));
}
