//! Adobe-compatible XMP sidecars (Sprint 7): `IMG_0001.ARW` ⇄ `IMG_0001.xmp`.
//!
//! * Interop: the standard `crs:` Camera Raw keys (`Exposure2012`, `Contrast2012`, `Highlights2012`,
//!   `Shadows2012`, `Whites2012`, `Blacks2012`, `Temperature`, `Tint`, `Vibrance`, `Saturation`, tone
//!   curves as `rdf:Seq`, colour grading) plus `xmp:Rating` (−1 = reject).
//! * Exactness: the full-precision parameter set is also stored as JSON in `halation:ParamsJson`, so
//!   Halation → XMP → Halation is lossless (curve points are only 0–255 integers in `crs:`).
//! * External edits win: if the `crs:` values no longer agree with the JSON (Lightroom touched the
//!   file), the `crs:` values are taken and only unsupported fields (local masks) come from the JSON.
//! * Writing into an existing sidecar preserves every attribute and element that is not ours.

use halation_core::{ColorGrading, CurveChannel, DevelopParameters, ParamId, ToneCurve, Wheel};
use std::path::{Path, PathBuf};

pub const NS_CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
pub const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
pub const NS_HALATION: &str = "http://ns.halation.app/1.0/";

pub fn sidecar_path(image: &Path) -> PathBuf {
    image.with_extension("xmp")
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmpState {
    pub params: DevelopParameters,
    /// 0..=5
    pub rating: u8,
    /// `Pick`, `Reject` or `None`.
    pub flag: String,
}

// ------------------------------------------------------------------------------------------------
// XML helpers
// ------------------------------------------------------------------------------------------------

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// `name="value"` pairs of a start tag body (quote-aware).
fn parse_attrs(tag: &str) -> Vec<(String, String)> {
    let b = tag.as_bytes();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = &tag[start..i];
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'=') {
            i += 1;
        }
        if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
            continue;
        }
        let quote = b[i];
        let vstart = i + 1;
        i = vstart;
        while i < b.len() && b[i] != quote {
            i += 1;
        }
        if !name.is_empty() {
            out.push((name.to_owned(), unescape(&tag[vstart..i.min(b.len())])));
        }
        i += 1;
    }
    out
}

/// Index of the `>` that closes the start tag beginning at `from` (quote-aware).
fn tag_end(text: &str, from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text[from..].char_indices() {
        match (quote, c) {
            (None, '"') | (None, '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(from + i),
            _ => {}
        }
    }
    None
}

/// Value of attribute `name` on the first `rdf:Description`, or of element `<name>…</name>`.
fn value_of(text: &str, name: &str) -> Option<String> {
    if let Some(s) = text.find("<rdf:Description") {
        let e = tag_end(text, s)?;
        if let Some((_, v)) = parse_attrs(&text[s + "<rdf:Description".len()..e]).into_iter().find(|(k, _)| k == name) {
            return Some(v);
        }
    }
    let open = format!("<{name}>");
    let at = text.find(&open)? + open.len();
    let end = text[at..].find(&format!("</{name}>"))?;
    Some(unescape(text[at..at + end].trim()))
}

fn number(text: &str, name: &str) -> Option<f32> {
    value_of(text, name)?.trim().trim_start_matches('+').parse().ok()
}

// ------------------------------------------------------------------------------------------------
// Mapping
// ------------------------------------------------------------------------------------------------

const SLIDERS: [(ParamId, &str); 10] = [
    (ParamId::Exposure, "crs:Exposure2012"),
    (ParamId::Contrast, "crs:Contrast2012"),
    (ParamId::Highlights, "crs:Highlights2012"),
    (ParamId::Shadows, "crs:Shadows2012"),
    (ParamId::Whites, "crs:Whites2012"),
    (ParamId::Blacks, "crs:Blacks2012"),
    (ParamId::Temperature, "crs:Temperature"),
    (ParamId::Tint, "crs:Tint"),
    (ParamId::Vibrance, "crs:Vibrance"),
    (ParamId::Saturation, "crs:Saturation"),
];

/// (wheel accessor, hue key, saturation key, luminance key)
type WheelKeys = (fn(&ColorGrading) -> &Wheel, fn(&mut ColorGrading) -> &mut Wheel, &'static str, &'static str, &'static str);
const WHEELS: [WheelKeys; 4] = [
    (|g| &g.shadows, |g| &mut g.shadows, "crs:SplitToningShadowHue", "crs:SplitToningShadowSaturation", "crs:ColorGradeShadowLum"),
    (|g| &g.midtones, |g| &mut g.midtones, "crs:ColorGradeMidtoneHue", "crs:ColorGradeMidtoneSat", "crs:ColorGradeMidtoneLum"),
    (|g| &g.highlights, |g| &mut g.highlights, "crs:SplitToningHighlightHue", "crs:SplitToningHighlightSaturation", "crs:ColorGradeHighlightLum"),
    (|g| &g.global, |g| &mut g.global, "crs:ColorGradeGlobalHue", "crs:ColorGradeGlobalSat", "crs:ColorGradeGlobalLum"),
];

const CURVES: [(CurveChannel, &str); 4] = [
    (CurveChannel::Rgb, "crs:ToneCurvePV2012"),
    (CurveChannel::Red, "crs:ToneCurvePV2012Red"),
    (CurveChannel::Green, "crs:ToneCurvePV2012Green"),
    (CurveChannel::Blue, "crs:ToneCurvePV2012Blue"),
];

const OWNED_ATTR_PREFIXES: [&str; 8] =
    ["crs:Exposure2012", "crs:Contrast2012", "crs:Highlights2012", "crs:Shadows2012", "crs:Whites2012", "crs:Blacks2012", "crs:Tint", "crs:Temperature"];
const OWNED_MORE: [&str; 8] =
    ["crs:Vibrance", "crs:Saturation", "crs:Version", "crs:ProcessVersion", "crs:HasSettings", "crs:SplitToning", "crs:ColorGrade", "halation:"];

fn owns(key: &str) -> bool {
    key == "xmp:Rating" || OWNED_ATTR_PREFIXES.iter().chain(OWNED_MORE.iter()).any(|p| key.starts_with(p))
}

fn our_attributes(state: &XmpState) -> Vec<(String, String)> {
    let p = &state.params;
    let mut a: Vec<(String, String)> = vec![
        ("xmp:Rating".into(), if state.flag == "Reject" { "-1".into() } else { state.rating.to_string() }),
        ("crs:Version".into(), "15.0".into()),
        ("crs:ProcessVersion".into(), "11.0".into()),
        ("crs:HasSettings".into(), "True".into()),
    ];
    for (id, key) in SLIDERS {
        a.push((key.into(), id.get(p).to_string()));
    }
    for (get, _, hue, sat, lum) in WHEELS {
        let w = get(&p.grading);
        if !w.is_neutral() {
            a.push((hue.into(), (w.hue.round() as i32).rem_euclid(360).to_string()));
            a.push((sat.into(), ((w.sat * 100.0).round() as i32).to_string()));
            a.push((lum.into(), ((w.lum * 100.0).round() as i32).to_string()));
        }
    }
    a.push(("halation:Flag".into(), state.flag.clone()));
    a.push(("halation:ParamsJson".into(), serde_json::to_string(p).unwrap_or_default()));
    a
}

fn our_children(p: &DevelopParameters) -> String {
    let mut out = String::new();
    for (channel, name) in CURVES {
        let pts = p.curve.points(channel);
        if pts.iter().all(|q| (q[0] - q[1]).abs() < 1.0e-5) {
            continue;
        }
        out.push_str(&format!("   <{name}>\n    <rdf:Seq>\n"));
        for q in pts {
            out.push_str(&format!("     <rdf:li>{}, {}</rdf:li>\n", (q[0] * 255.0).round() as i32, (q[1] * 255.0).round() as i32));
        }
        out.push_str(&format!("    </rdf:Seq>\n   </{name}>\n"));
    }
    out
}

fn curve_from_xmp(text: &str, name: &str) -> Option<Vec<[f32; 2]>> {
    let open = format!("<{name}>");
    let at = text.find(&open)? + open.len();
    let block = &text[at..at + text[at..].find(&format!("</{name}>"))?];
    let pts: Vec<[f32; 2]> = block
        .split("<rdf:li>")
        .skip(1)
        .filter_map(|item| {
            let body = item.split("</rdf:li>").next()?;
            let (x, y) = body.split_once(',')?;
            Some([x.trim().parse::<f32>().ok()? / 255.0, y.trim().parse::<f32>().ok()? / 255.0])
        })
        .collect();
    (pts.len() >= 2).then_some(pts)
}

fn params_from_crs(text: &str) -> Option<DevelopParameters> {
    let mut p = DevelopParameters::default();
    let mut any = false;
    for (id, key) in SLIDERS {
        if let Some(v) = number(text, key) {
            id.set(&mut p, v);
            any = true;
        }
    }
    if !any {
        return None;
    }
    for (get, get_mut, hue, sat, lum) in WHEELS {
        let _ = get;
        let w = get_mut(&mut p.grading);
        if let Some(v) = number(text, hue) {
            w.hue = v;
        }
        if let Some(v) = number(text, sat) {
            w.sat = v / 100.0;
        }
        if let Some(v) = number(text, lum) {
            w.lum = v / 100.0;
        }
    }
    let mut curve = ToneCurve::default();
    for (channel, name) in CURVES {
        if let Some(pts) = curve_from_xmp(text, name) {
            match channel {
                CurveChannel::Rgb => curve.rgb = pts,
                CurveChannel::Red => curve.red = pts,
                CurveChannel::Green => curve.green = pts,
                CurveChannel::Blue => curve.blue = pts,
            }
        }
    }
    p.curve = curve;
    Some(p.sanitized())
}

fn agrees(a: &DevelopParameters, b: &DevelopParameters) -> bool {
    ParamId::ALL.iter().all(|&id| (id.get(a) - id.get(b)).abs() < 1.0e-3)
}

// ------------------------------------------------------------------------------------------------
// Read / write
// ------------------------------------------------------------------------------------------------

pub fn parse(text: &str) -> Option<XmpState> {
    let json = value_of(text, "halation:ParamsJson").and_then(|s| serde_json::from_str::<DevelopParameters>(&s).ok());
    let from_crs = params_from_crs(text);
    let params = match (json, from_crs) {
        (Some(j), Some(c)) if agrees(&j, &c) => j,
        (Some(j), Some(mut c)) => {
            c.local = j.local;
            c
        }
        (Some(j), None) => j,
        (None, Some(c)) => c,
        (None, None) => return None,
    };
    let raw_rating = number(text, "xmp:Rating").unwrap_or(0.0) as i32;
    let flag = value_of(text, "halation:Flag").unwrap_or_else(|| if raw_rating < 0 { "Reject".into() } else { "None".into() });
    Some(XmpState { params, rating: raw_rating.clamp(0, 5) as u8, flag })
}

/// True when another program changed the sidecar: its `crs:` values contradict Halation's exact JSON,
/// or the JSON is missing altogether (a foreign / Lightroom-written packet).
pub fn externally_modified(text: &str) -> bool {
    let json = value_of(text, "halation:ParamsJson").and_then(|s| serde_json::from_str::<DevelopParameters>(&s).ok());
    match (json, params_from_crs(text)) {
        (Some(j), Some(c)) => !agrees(&j, &c),
        (None, Some(_)) => true,
        _ => false,
    }
}

fn fresh(attrs: &[(String, String)], children: &str) -> String {
    let body: String = attrs.iter().map(|(k, v)| format!("\n    {k}=\"{}\"", escape(v))).collect();
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Halation\">\n \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:xmp=\"{NS_XMP}\"\n    xmlns:crs=\"{NS_CRS}\"\n    xmlns:halation=\"{NS_HALATION}\"{body}>\n{children}  </rdf:Description>\n \
         </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>\n"
    )
}

/// Merge our keys into `existing`, keeping everything else. `None` if it has no `rdf:Description`.
fn merge(existing: &str, attrs: &[(String, String)], children: &str) -> Option<String> {
    const OPEN: &str = "<rdf:Description";
    let s = existing.find(OPEN)?;
    let e = tag_end(existing, s)?;
    let self_closing = existing[..e].ends_with('/');
    let head = &existing[s + OPEN.len()..if self_closing { e - 1 } else { e }];
    let mut kept: Vec<(String, String)> = parse_attrs(head).into_iter().filter(|(k, _)| !owns(k)).collect();
    for (prefix, ns) in [("xmlns:xmp", NS_XMP), ("xmlns:crs", NS_CRS), ("xmlns:halation", NS_HALATION)] {
        if !kept.iter().any(|(k, _)| k == prefix) {
            kept.push((prefix.into(), ns.into()));
        }
    }
    kept.extend(attrs.iter().cloned());
    let tag: String = kept.iter().map(|(k, v)| format!("\n    {k}=\"{}\"", escape(v))).collect();

    let (inner, tail) = if self_closing {
        (String::new(), existing[e + 1..].to_owned())
    } else {
        let close = existing[e..].find("</rdf:Description>")? + e;
        let mut inner = existing[e + 1..close].to_owned();
        for (_, name) in CURVES {
            while let (Some(a), Some(b)) = (inner.find(&format!("<{name}>")), inner.find(&format!("</{name}>"))) {
                if b < a {
                    break;
                }
                inner.replace_range(a..b + name.len() + 3, "");
            }
        }
        (inner, existing[close + "</rdf:Description>".len()..].to_owned())
    };
    Some(format!("{}{OPEN}{tag}>\n{}{children}  </rdf:Description>{tail}", &existing[..s], inner.trim_end_matches([' ', '\n', '\r']).to_owned() + if inner.trim().is_empty() { "" } else { "\n" }))
}

/// Serialize `state`, merging into `existing` sidecar text when given.
pub fn render(existing: Option<&str>, state: &XmpState) -> String {
    let (attrs, children) = (our_attributes(state), our_children(&state.params));
    existing.and_then(|t| merge(t, &attrs, &children)).unwrap_or_else(|| fresh(&attrs, &children))
}

pub fn write_sidecar(image: &Path, state: &XmpState) -> std::io::Result<PathBuf> {
    let path = sidecar_path(image);
    let existing = std::fs::read_to_string(&path).ok();
    let tmp = path.with_extension("xmp.tmp");
    std::fs::write(&tmp, render(existing.as_deref(), state))?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn read_sidecar(image: &Path) -> Option<XmpState> {
    parse(&std::fs::read_to_string(sidecar_path(image)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> XmpState {
        let mut p = DevelopParameters::default();
        p.exposure = 0.5;
        p.contrast = -15.0;
        p.temperature = 6480.0;
        p.curve.insert_point(CurveChannel::Rgb, 0.333_333, 0.25);
        p.grading.highlights = Wheel { hue: 41.5, sat: 0.37, lum: 0.1 };
        XmpState { params: p, rating: 4, flag: "Pick".into() }
    }

    #[test]
    fn halation_roundtrip_is_exact() {
        let state = sample();
        let back = parse(&render(None, &state)).unwrap();
        assert_eq!(back, state);
        assert_eq!(serde_json::to_string(&back.params).unwrap(), serde_json::to_string(&state.params).unwrap());
    }

    #[test]
    fn exposes_standard_adobe_keys() {
        let text = render(None, &sample());
        for key in ["crs:Exposure2012=\"0.5\"", "crs:Contrast2012=\"-15\"", "crs:Temperature=\"6480\"", "xmp:Rating=\"4\"", "<crs:ToneCurvePV2012>"] {
            assert!(text.contains(key), "missing {key}");
        }
    }

    #[test]
    fn external_edits_win_over_the_json() {
        let text = render(None, &sample()).replace("crs:Exposure2012=\"0.5\"", "crs:Exposure2012=\"1.25\"");
        assert_eq!(parse(&text).unwrap().params.exposure, 1.25);
    }

    #[test]
    fn merging_preserves_foreign_content() {
        let existing = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
            <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:crs=\"x\" crs:Exposure2012=\"9\" dc:creator=\"Ada\">\
            <dc:subject><rdf:Bag><rdf:li>trip</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>";
        let merged = render(Some(existing), &sample());
        assert!(merged.contains("dc:creator=\"Ada\"") && merged.contains("<rdf:li>trip</rdf:li>"));
        assert!(merged.contains("crs:Exposure2012=\"0.5\"") && !merged.contains("crs:Exposure2012=\"9\""));
        assert_eq!(parse(&merged).unwrap(), sample());
        assert_eq!(merged.matches("<crs:ToneCurvePV2012>").count(), 1);
        assert_eq!(parse(&render(Some(&merged), &sample())).unwrap(), sample());
    }
}
