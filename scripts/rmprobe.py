# -*- coding: utf-8 -*-
import io
s = io.open('build.rs', encoding='utf-8').read()
s = s.replace('''/* 【临时探针】naga 对 WGSL ray-query 支持
const RQ_PROBE: &str = r#"
enable ray_query;
@compute @workgroup_size(8, 8)
fn rq(@builtin(global_invocation_id) gid: vec3<u32>) {
    let q = rayQueryInitializeEXT(
        vec4<u32>(0u, 0u, 0u, 0u), 0u, 0u,
        vec3<f32>(0.0), 0.001, vec3<f32>(0.0, 1.0, 0.0), 1000.0);
}
"#;
''' , '')
s = s.replace('''    match naga::front::wgsl::parse_str(RQ_PROBE) {
        Ok(_) => println!("cargo:warning=RQ_PARSE_OK"),
        Err(e) => println!("cargo:warning=RQ_PARSE_ERR: {}", format!("{:?}", e).chars().take(120).collect::<String>()),
    }
''', '')
io.open('build.rs', 'w', encoding='utf-8', newline='').write(s)
print('probe removed', 'RQ_PROBE' not in s)
