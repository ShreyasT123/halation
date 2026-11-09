use halation_core::{DevelopParameters, ImageBufferF32};
use halation_gpu::GpuDevelopEngine;
use halation_pipeline::process_buffer;
use halation_raw::decode_file;
use std::path::Path;

fn render(label: &str, path: &str, p: &DevelopParameters, gpu: Option<&GpuDevelopEngine>) {
    let asset = match decode_file(Path::new(path)) {
        Ok(a) => a,
        Err(e) => {
            println!("{label}: decode failed: {e}");
            return;
        }
    };
    let src = halation_pipeline::make_preview(&asset.buffer, 1200);
    let out = process_buffer(&src, p, gpu);
    let dest = std::env::temp_dir().join(format!("halation-{label}.png"));
    let _ = image::save_buffer(&dest, &out.display_rgba, out.width, out.height, image::ColorType::Rgba8);
    let mean = out.display_rgba.chunks_exact(4).fold([0u64; 3], |mut a, px| {
        a[0] += px[0] as u64;
        a[1] += px[1] as u64;
        a[2] += px[2] as u64;
        a
    });
    let n = (out.display_rgba.len() / 4) as u64;
    println!("{label:<14} used_gpu={} {dest:?} mean=({}, {}, {})", out.used_gpu, mean[0] / n, mean[1] / n, mean[2] / n);
}

fn main() {
    let gpu = GpuDevelopEngine::new().ok();
    let gpu = gpu.as_ref();

    let asset1 = r"C:\Users\sstha\Pictures\Screenshots\Screenshot 2026-11-07 155709.png";
    let asset2 = r"C:\Users\sstha\Pictures\Screenshots\Screenshot 2026-10-31 152250.png";
    let asset3 = r"C:\Users\sstha\Pictures\Screenshots\Screenshot 2026-06-26 201939.png";

    let d = DevelopParameters::default();
    render("a1-default", asset1, &d, gpu);
    render("a2-default", asset2, &d, gpu);
    render("a3-default", asset3, &d, gpu);

    let mut p1 = DevelopParameters::default();
    p1.exposure = -0.19;
    p1.shadows = 39.0;
    p1.curve.red = vec![[0.0, 0.0], [0.49047634, 0.4897619], [1.0, 1.0]];
    p1.curve.green = vec![[0.0, 0.0], [0.5023808, 0.4945237], [1.0, 1.0]];
    p1.curve.blue = vec![[0.0, 0.0], [0.51428574, 0.48738086], [1.0, 1.0]];
    render("a1-curves", asset1, &p1, gpu);

    let mut p2 = DevelopParameters::default();
    p2.curve.green = vec![[0.0, 0.0], [0.48333347, 0.4945237], [1.0, 1.0]];
    render("a2-curves", asset2, &p2, gpu);

    let mut p3 = DevelopParameters::default();
    p3.curve.red = vec![[0.0, 0.0], [0.5023808, 0.50166655], [1.0, 1.0]];
    p3.curve.blue = vec![[0.0, 0.0], [0.5023808, 0.4754762], [1.0, 1.0]];
    render("a3-curves", asset3, &p3, gpu);
}
