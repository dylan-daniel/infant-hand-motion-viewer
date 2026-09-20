struct Uniforms {
    model: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    alpha: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) frag_pos: vec3<f32>,
    @location(1) normal_ref: vec3<f32>,
    @location(2) color: vec4<f32>,
};

fn transform(pos: vec3<f32>, normal: vec3<f32>, color: vec4<f32>) -> VsOut {
    let model_view = u.view * u.model;
    let view_pos = model_view * vec4<f32>(pos, 1.0);
    var out: VsOut;
    out.clip = u.proj * view_pos;
    out.frag_pos = view_pos.xyz;
    out.normal_ref = mat3x3<f32>(model_view[0].xyz, model_view[1].xyz, model_view[2].xyz) * normal;
    out.color = color;
    return out;
}

@vertex
fn vs_lit(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    return transform(pos, normal, color);
}

@vertex
fn vs_flat(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> VsOut {
    return transform(pos, vec3<f32>(0.0, 0.0, 1.0), color);
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, in.color.a * u.alpha);
}

const LIGHT0_POS = vec3<f32>(5.0, 10.0, 5.0);
const LIGHT0_DIFFUSE = vec3<f32>(0.85, 0.85, 0.85);
const LIGHT0_AMBIENT = vec3<f32>(0.15, 0.15, 0.15);
const LIGHT1_POS = vec3<f32>(-5.0, 6.0, -8.0);
const LIGHT1_DIFFUSE = vec3<f32>(0.3, 0.35, 0.5);
const SPECULAR = vec3<f32>(0.4, 0.4, 0.4);
const SHININESS = 32.0;

@fragment
fn fs_lit(in: VsOut) -> @location(0) vec4<f32> {
    var normal = normalize(cross(dpdx(in.frag_pos), dpdy(in.frag_pos)));
    if (dot(normal, in.normal_ref) < 0.0) {
        normal = -normal;
    }

    let frag = in.frag_pos;
    let eye = normalize(-frag);

    var lit = in.color.rgb * LIGHT0_AMBIENT;

    let dir0 = normalize(LIGHT0_POS - frag);
    lit += in.color.rgb * LIGHT0_DIFFUSE * max(dot(normal, dir0), 0.0);
    let half0 = normalize(dir0 + eye);
    lit += SPECULAR * LIGHT0_DIFFUSE * pow(max(dot(normal, half0), 0.0), SHININESS);

    let dir1 = normalize(LIGHT1_POS - frag);
    lit += in.color.rgb * LIGHT1_DIFFUSE * max(dot(normal, dir1), 0.0);

    return vec4<f32>(lit, in.color.a * u.alpha);
}
