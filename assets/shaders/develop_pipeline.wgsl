struct DevelopUniforms {
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
};

@group(0) @binding(0) var<uniform> uniforms: DevelopUniforms;
@group(0) @binding(1) var input_texture: texture_2d<f32>;
@group(0) @binding(2) var output_texture: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = textureDimensions(input_texture);
    if (global_id.x >= dims.x || global_id.y >= dims.y) {
        return;
    }
    let coord = vec2<i32>(global_id.xy);
    var color = textureLoad(input_texture, coord, 0);
    
    // EV exposure adjustment
    color = vec4<f32>(color.rgb * pow(2.0, uniforms.exposure), color.a);
    
    textureStore(output_texture, coord, color);
}
