struct DevelopUniforms {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    temperature: f32,
    tint: f32,
    wb0: vec4<f32>,
    wb1: vec4<f32>,
    wb2: vec4<f32>,
    tone_points: array<vec4<f32>, 20>,
    grading: array<vec4<f32>, 3>,
};

@group(0) @binding(0) var<uniform> u: DevelopUniforms;
@group(0) @binding(1) var input_texture: texture_2d<f32>;
@group(0) @binding(2) var output_texture: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var display_lut: texture_3d<f32>;
@group(0) @binding(4) var display_sampler: sampler;
@group(0) @binding(5) var blue_noise_texture: texture_2d<f32>;

fn curve_point(channel: u32, index: u32) -> vec2<f32> {
    return u.tone_points[channel * 5u + index].xy;
}

fn curve_slope(channel: u32, segment: u32) -> f32 {
    let a = curve_point(channel, segment);
    let b = curve_point(channel, segment + 1u);
    return (b.y - a.y) / max(b.x - a.x, 0.00001);
}

fn curve_tangent(channel: u32, point: u32) -> f32 {
    if (point == 0u) { return curve_slope(channel, 0u); }
    if (point >= 4u) { return curve_slope(channel, 3u); }
    let left = curve_slope(channel, point - 1u);
    let right = curve_slope(channel, point);
    if (left * right <= 0.0) { return 0.0; }
    return 2.0 * left * right / (left + right);
}

fn curve_sample(channel: u32, value: f32) -> f32 {
    var segment = 3u;
    for (var i = 0u; i < 4u; i += 1u) {
        if (value <= curve_point(channel, i + 1u).x) { segment = i; break; }
    }
    let a = curve_point(channel, segment);
    let b = curve_point(channel, segment + 1u);
    let width = max(b.x - a.x, 0.00001);
    let t = clamp((value - a.x) / width, 0.0, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    let y = (2.0*t3-3.0*t2+1.0)*a.y
        + (t3-2.0*t2+t)*width*curve_tangent(channel, segment)
        + (-2.0*t3+3.0*t2)*b.y
        + (t3-t2)*width*curve_tangent(channel, segment + 1u);
    return clamp(y, 0.0, 1.0);
}

@compute @workgroup_size(16, 16)
fn cs_develop_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(input_texture);
    if (id.x >= dims.x || id.y >= dims.y) { return; }

    var color = textureLoad(input_texture, vec2<i32>(id.xy), 0).rgb;

    // Linear exposure, matching the existing editor controls.
    color *= exp2(u.exposure);

    color = vec3<f32>(dot(u.wb0.xyz, color), dot(u.wb1.xyz, color), dot(u.wb2.xyz, color));
    color = max(color, vec3<f32>(0.0));

    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let shadow_mask = pow(max(1.0 - luma / 0.35, 0.0), 2.0);
    let highlight_mask = pow(clamp((luma - 0.65) / 0.35, 0.0, 1.0), 2.0);
    let range_factor = 1.0 + (u.shadows / 100.0 * 0.5 * shadow_mask) + (u.highlights / 100.0 * 0.5 * highlight_mask);
    color *= range_factor;
    color += color * (u.whites * 0.005 * highlight_mask);
    color += vec3<f32>(u.blacks * 0.0005 * shadow_mask);
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    let contrast_factor = (259.0 * (u.contrast + 255.0)) / (255.0 * (259.0 - u.contrast));
    color = clamp((color - vec3<f32>(0.5)) * contrast_factor + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(1.0));
    color.r = curve_sample(1u, color.r);
    color.g = curve_sample(2u, color.g);
    color.b = curve_sample(3u, color.b);
    color = vec3<f32>(curve_sample(0u, color.r), curve_sample(0u, color.g), curve_sample(0u, color.b));

    let grade_luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let grade_weights = vec3<f32>(
        clamp(1.0 - grade_luma / 0.42, 0.0, 1.0),
        clamp(1.0 - abs(grade_luma - 0.5) / 0.38, 0.0, 1.0),
        clamp((grade_luma - 0.58) / 0.42, 0.0, 1.0)
    );
    for (var zone = 0u; zone < 3u; zone += 1u) {
        let grade = u.grading[zone];
        let angle = grade.x * 6.28318530718;
        let tint = cos(vec3<f32>(angle, angle - 2.09439510239, angle + 2.09439510239));
        color += tint * grade.y * grade_weights[zone] * 0.11 + vec3<f32>(grade.z * grade_weights[zone] * 0.12);
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    color = textureSampleLevel(display_lut, display_sampler, color, 0.0).rgb;
    // Tile a 64x64 farthest-point ranked blue-noise pattern at half an 8-bit LSB amplitude.
    let noise_xy = vec2<i32>(id.xy % vec2<u32>(64u, 64u));
    let noise = textureLoad(blue_noise_texture, noise_xy, 0).r;
    color = clamp(color + vec3<f32>((noise - 0.5) / 255.0), vec3<f32>(0.0), vec3<f32>(1.0));
    textureStore(output_texture, vec2<i32>(id.xy), vec4<f32>(color, 1.0));
}
