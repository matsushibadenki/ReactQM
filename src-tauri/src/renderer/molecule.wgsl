struct Camera { view_projection: mat4x4<f32>, eye: vec4<f32> };
struct Instance { model: mat4x4<f32>, normal: mat4x4<f32>, color: vec4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
struct VertexInput { @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @builtin(instance_index) index: u32 };
struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) world: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32> };
@vertex fn vs_main(input: VertexInput) -> VertexOutput {
    let instance = instances[input.index];
    let world = instance.model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.position = camera.view_projection * world;
    output.world = world.xyz;
    output.normal = (instance.normal * vec4<f32>(input.normal, 0.0)).xyz;
    output.color = instance.color;
    return output;
}
@fragment fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    let view = normalize(camera.eye.xyz - input.world);
    let light = normalize(vec3<f32>(-0.5, 0.8, 1.0));
    let fill = normalize(vec3<f32>(0.7, -0.2, -0.5));
    let diffuse = max(dot(normal, light), 0.0);
    let specular = pow(max(dot(normal, normalize(light + view)), 0.0), 48.0) * 0.3;
    let shade = 0.22 + diffuse * 0.66 + max(dot(normal, fill),0.0) * 0.14;
    return vec4<f32>(input.color.rgb * shade + vec3<f32>(specular), 1.0);
}
