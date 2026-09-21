// Draws the frame image, then the hands over it, shaded like WiLoR's demo (pyrender's metallic-roughness PBR).

struct Uniforms {
    proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var frame_tex: texture_2d<f32>;
@group(0) @binding(2) var frame_samp: sampler;

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_bg(@builtin(vertex_index) index: u32) -> BgOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let xy = corners[index];
    var out: BgOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

@fragment
fn fs_bg(in: BgOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(frame_tex, frame_samp, in.uv).rgb, 1.0);
}

struct HandOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) frag_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_hand(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
) -> HandOut {
    var out: HandOut;
    out.pos = u.proj * vec4<f32>(pos, 1.0);
    // The camera sits at the origin, so world space is view space.
    out.frag_pos = pos;
    out.normal = normal;
    out.color = color;
    return out;
}

const PI: f32 = 3.141592653589793;
const MIN_ROUGHNESS: f32 = 0.04;

// pyrender's default material for a mesh that only has vertex colors.
const BASE_COLOR: f32 = 0.3;
const METALLIC: f32 = 0.2;
const ROUGHNESS: f32 = 0.8;
// The scene's ambient light.
const AMBIENT: f32 = 0.3;

// The four directional lights pyrender keeps from the demo's rig (it sorts by distance and keeps four):
// the headlight along the view direction and the three "Raymond" lights 30 degrees off it at 120 degree spacing.
// Each entry is the unit vector towards the light.
const LIGHT_COUNT: i32 = 4;
const LIGHT_DIRS = array<vec3<f32>, 4>(
    vec3<f32>(0.0, 0.0, 1.0),
    vec3<f32>(0.5, 0.0, 0.8660254037844387),
    vec3<f32>(-0.25, 0.4330127018922193, 0.8660254037844387),
    vec3<f32>(-0.25, -0.4330127018922193, 0.8660254037844387),
);

fn brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, f0: vec3<f32>, c_diff: vec3<f32>) -> vec3<f32> {
    let h = normalize(l + v);
    let nl = clamp(dot(n, l), 0.001, 1.0);
    let nv = clamp(abs(dot(n, v)), 0.001, 1.0);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);

    // Fresnel (Schlick)
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - vh, 0.0, 1.0), 5.0);
    // Geometric occlusion (Smith)
    let r = ROUGHNESS + 1.0;
    let k = r * r / 8.0;
    let g1 = nv / (nv * (1.0 - k) + k);
    let g2 = nl / (nl * (1.0 - k) + k);
    let g = g1 * g2;
    // Microfacet distribution (GGX)
    let a = ROUGHNESS * ROUGHNESS;
    let a2 = a * a;
    let nh2 = nh * nh;
    let denom = nh2 * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * denom * denom);

    let diffuse = (vec3<f32>(1.0) - fresnel) * c_diff / PI;
    let specular = fresnel * g * d / (4.0 * nl * nv + 0.001);
    return nl * (diffuse + specular);
}

@fragment
fn fs_hand(in: HandOut) -> @location(0) vec4<f32> {
    let c_diff = vec3<f32>(BASE_COLOR * (1.0 - MIN_ROUGHNESS) * (1.0 - METALLIC));
    let f0 = mix(vec3<f32>(MIN_ROUGHNESS), vec3<f32>(BASE_COLOR), METALLIC);

    let n = normalize(in.normal);
    let v = normalize(-in.frag_pos);

    var lit = vec3<f32>(0.0);
    for (var i = 0; i < LIGHT_COUNT; i = i + 1) {
        lit += brdf(n, v, LIGHT_DIRS[i], f0, c_diff);
    }
    // The point light at the camera: intensity 1 falling off with the square of the distance.
    let to_light = -in.frag_pos;
    let dist = length(to_light);
    lit += brdf(n, v, to_light / dist, f0, c_diff) / (dist * dist);
    lit += vec3<f32>(BASE_COLOR) * AMBIENT;

    // The vertex color modulates the lit result, then the whole thing is gamma encoded.
    let color = lit * in.color.rgb;
    return vec4<f32>(clamp(pow(color, vec3<f32>(1.0 / 2.2)), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
