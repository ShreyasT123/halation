use glam::Vec3;

pub fn white_balance(rgb: [f32; 3], temp: f32, tint: f32) -> [f32; 3] {
    let temp_offset = (temp - 5500.0) / 4500.0;
    let tint_offset = tint / 100.0;

    let v = Vec3::from_array(rgb);
    let r = v.x * (1.0 + temp_offset * 0.32);
    let g = v.y * (1.0 + tint_offset * 0.20);
    let b = v.z * (1.0 - temp_offset * 0.32);

    [r.max(0.0), g.max(0.0), b.max(0.0)]
}

pub fn apply_highlights_shadows(rgb: [f32; 3], highlights: f32, shadows: f32) -> [f32; 3] {
    let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    
    // Shadows affect luma < 0.35, Highlights affect luma > 0.65
    let shadow_mask = (1.0 - (luma / 0.35).min(1.0)).powi(2);
    let highlight_mask = (((luma - 0.65) / 0.35).clamp(0.0, 1.0)).powi(2);

    let shadow_gain = (shadows / 100.0) * 0.5;
    let highlight_gain = (highlights / 100.0) * 0.5;

    let factor = 1.0 + (shadow_gain * shadow_mask) + (highlight_gain * highlight_mask);

    [
        (rgb[0] * factor).max(0.0),
        (rgb[1] * factor).max(0.0),
        (rgb[2] * factor).max(0.0),
    ]
}

pub fn apply_contrast(rgb: [f32; 3], contrast: f32) -> [f32; 3] {
    let factor = (259.0 * (contrast + 255.0)) / (255.0 * (259.0 - contrast));
    [
        ((rgb[0] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
        ((rgb[1] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
        ((rgb[2] - 0.5) * factor + 0.5).clamp(0.0, 1.0),
    ]
}
