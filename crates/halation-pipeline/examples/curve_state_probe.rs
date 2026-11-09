use halation_core::{CurveChannel, DevelopParameters, ToneCurve};
use halation_gpu::GpuDevelopEngine;
use halation_pipeline::process_buffer;
use halation_raw::decode_file;
use std::path::Path;

fn stats(label: &str, rgba: &[u8]) -> [f64; 3] {
    let n = (rgba.len() / 4) as f64;
    let mut m = [0.0f64; 3];
    for px in rgba.chunks_exact(4) {
        for k in 0..3 {
            m[k] += px[k] as f64;
        }
    }
    for v in &mut m {
        *v /= n;
    }
    println!("{label:<30} mean=({:6.2}, {:6.2}, {:6.2})", m[0], m[1], m[2]);
    m
}

fn main() {
    let gpu = GpuDevelopEngine::new().ok();
    let gpu = gpu.as_ref();
    let path = r"C:\Users\sstha\Pictures\Screenshots\Screenshot 2026-11-07 155709.png";
    let asset = decode_file(Path::new(path)).expect("decode");
    let src = halation_pipeline::make_preview(&asset.buffer, 1200);

    let base = DevelopParameters::default();
    let mut node2 = DevelopParameters::default();
    node2.curve.rgb = vec![[0.0, 0.0], [0.23809509, 0.33499998], [1.0, 1.0]];

    let mut strong = DevelopParameters::default();
    strong.curve.rgb = vec![[0.0, 0.0], [0.25, 0.75], [1.0, 1.0]];

    let mut red_only = DevelopParameters::default();
    red_only.curve.red = vec![[0.0, 0.0], [0.25, 0.75], [1.0, 1.0]];

    let mut blue_only = DevelopParameters::default();
    blue_only.curve.blue = vec![[0.0, 0.0], [0.25, 0.75], [1.0, 1.0]];

    let mut all_channels = DevelopParameters::default();
    let c = vec![[0.0, 0.0], [0.25, 0.75], [1.0, 1.0]];
    all_channels.curve = ToneCurve { rgb: c.clone(), red: c.clone(), green: c.clone(), blue: c.clone() };

    // First render (warms the GPU cache).
    let a = process_buffer(&src, &base, gpu);
    stats("default #1", &a.display_rgba);
    let b = process_buffer(&src, &node2, gpu);
    stats("rgb node2", &b.display_rgba);
    let c1 = process_buffer(&src, &base, gpu);
    stats("default #2", &c1.display_rgba);
    let d = process_buffer(&src, &strong, gpu);
    stats("rgb strong", &d.display_rgba);
    let e = process_buffer(&src, &base, gpu);
    stats("default #3", &e.display_rgba);

    let r = process_buffer(&src, &red_only, gpu);
    stats("red strong", &r.display_rgba);
    let bl = process_buffer(&src, &blue_only, gpu);
    stats("blue strong", &bl.display_rgba);
    let ac = process_buffer(&src, &all_channels, gpu);
    stats("all channels strong", &ac.display_rgba);

    // Neutrality of a neutral patch: use a 3x1 synthetic image, 18% gray in ProPhoto.
    let patch = halation_core::ImageBufferF32::new(
        3,
        1,
        vec![[0.18, 0.18, 0.18], [0.046, 0.046, 0.046], [0.5, 0.5, 0.5]],
    );
    let mut curve = DevelopParameters::default();
    curve.curve.insert_point(CurveChannel::Rgb, 0.5, 0.75);
    let p = process_buffer(&patch, &curve, gpu);
    println!("gray patch rgb curve: {:?}", &p.display_rgba);
    let p = process_buffer(&patch, &base, gpu);
    println!("gray patch default : {:?}", &p.display_rgba);

    // Exact repeatability: render the same params twice after a different render and compare bytes.
    let x = process_buffer(&src, &node2, gpu);
    let y = process_buffer(&src, &node2, gpu);
    println!("repeatable: {}", x.display_rgba == y.display_rgba);
    println!("default stable across intervening curves: {}", a.display_rgba == c1.display_rgba && a.display_rgba == e.display_rgba);
    let _ = CurveChannel::Rgb;
}
