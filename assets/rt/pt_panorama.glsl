#version 460
#extension GL_EXT_ray_query : require

// 钢铁前线 · 全景路径追踪参考帧（2026-08-31 重写：glslang 编译，取代手工拼装 SPIR-V）
// 用途：为光照烘焙提供"硬件 RT 真值"参照。NEE（太阳直接光阴影射线）+ 漫反射弹跳，
//      1 spp 即低噪。盒体场景 identity 变换，命中面法线可由来射方向主轴精确还原。

layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform accelerationStructureEXT TLAS;
layout(set = 0, binding = 1, rgba8) uniform writeonly image2D OutImg;
// 每盒材质：albedo.rgb + 光泽度（与游戏 WorldMarker 同色，PT 才能当烘焙参照）
layout(set = 0, binding = 2, std430) readonly buffer Mats { vec4 boxMats[]; };

// 5 x vec4 = 80B，Rust 侧 [[f32;4];5] 逐字段对齐，无填充歧义
// 相机直接传 forward 向量（不传 yaw/pitch）=> 与 engine/camera.rs 的基底严格同源，无前后手风险
layout(push_constant) uniform PC {
    vec4 a; // (resX, resY, tanHalfFov, bounces)
    vec4 b; // camPos.xyz
    vec4 c; // fwd.xyz      = camera.forward()
    vec4 d; // sunDir.xyz   表面->太阳
    vec4 e; // sunColor.rgb, exposure
} pc;

const vec3 SKY_ZENITH  = vec3(0.28, 0.42, 0.66);
const vec3 SKY_HORIZON = vec3(0.72, 0.74, 0.76);
const float SUN_COS = 0.9997;   // 太阳圆盘角阈值
const float PI = 3.14159265;

uint hitPrim;
vec3 hitPos;
vec3 hitNrm;

vec3 skyColor(vec3 rd) {
    float t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 col = mix(SKY_HORIZON, SKY_ZENITH, t * t);
    if (dot(rd, normalize(pc.d.xyz)) > SUN_COS) col += vec3(12.0);
    return col * 0.9;
}

// true = 命中。不剔除任何朝向（凸盒从外部入射，closest-hit 即入射面，与绕序无关）
// 铁律：committed 必须为 true —— 传 false(0) 取的是「候选」记录，proceed 结束后已被清空，
//       恒返回 NoIntersection（这正是 2026-08-30 pt_frame.comp 全图无命中的根因）。
bool traceRay(vec3 ro, vec3 rd, float tmax) {
    rayQueryEXT rq;
    rayQueryInitializeEXT(rq, TLAS, gl_RayFlagsNoneEXT, 0xFF, ro, 0.001, rd, tmax);
    while (rayQueryProceedEXT(rq)) {}
    if (rayQueryGetIntersectionTypeEXT(rq, true) != gl_RayQueryCommittedIntersectionTriangleEXT)
        return false;
    hitPos = ro + rd * rayQueryGetIntersectionTEXT(rq, true);
    // 面法线：ray_tracer::box_triangles/box_indices 的不变量——盒 6 面按 -X,+X,-Y,+Y,-Z,+Z
    // 各 2 三角顺序展开，故 (primitive % 12) / 2 就是面号。
    // （旧「来射方向主轴」近似在浅角度下会把地面法线错判成 ±Z，导致 ndl<0、地面全黑）
    hitPrim = uint(rayQueryGetIntersectionPrimitiveIndexEXT(rq, true));
    uint f = (hitPrim % 12u) / 2u;
    hitNrm = f == 0u ? vec3(-1.0, 0.0, 0.0)
             : f == 1u ? vec3(1.0, 0.0, 0.0)
             : f == 2u ? vec3(0.0, -1.0, 0.0)
             : f == 3u ? vec3(0.0, 1.0, 0.0)
             : f == 4u ? vec3(0.0, 0.0, -1.0)
             : vec3(0.0, 0.0, 1.0);
    return true;
}

float hash11(uint p) {
    p = p * 747796405u + 2891336453u;
    p = ((p >> 5u) ^ p) * 1274126177u;
    return float(((p >> 16u) ^ p) & 0xFFFFu) * (1.0 / 65535.0);
}

vec3 cosSample(vec2 e) {
    float r = sqrt(max(0.0, 1.0 - e.y));
    float phi = 2.0 * PI * e.x;
    return vec3(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - r * r)));
}

vec3 lambertianBounce(vec3 n, float seed) {
    uint s = floatBitsToUint(seed);
    vec2 e = vec2(hash11(s ^ 0x9E3779B9u), hash11((s ^ 0x85EBCA6Bu) + 13u));
    vec3 up = abs(n.x) < 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 t = normalize(cross(up, n));
    vec3 bt = cross(n, t);
    vec3 d = cosSample(e);
    return normalize(t * d.x + bt * d.y + n * d.z);
}

// 材质来自 binding 2 的每盒 SSBO（与游戏 WorldMarker 同色），越界返回中性灰
vec3 albedoOf(uint boxIdx) {
    if (int(boxIdx) >= boxMats.length()) return vec3(0.5);
    return boxMats[int(boxIdx)].rgb;
}

void main() {
    ivec2 gid = ivec2(gl_GlobalInvocationID.xy);
    if (gid.x >= int(pc.a.x) || gid.y >= int(pc.a.y)) return;

    vec3 fwd = normalize(pc.c.xyz);
    vec3 rgt = normalize(cross(fwd, vec3(0.0, 1.0, 0.0)));
    vec3 up  = cross(rgt, fwd);

    float ux = (float(gid.x) + 0.5) / pc.a.x * 2.0 - 1.0;
    float uy = 1.0 - (float(gid.y) + 0.5) / pc.a.y * 2.0;
    float tan = pc.a.z;
    vec3 rd = normalize(fwd + rgt * (ux * tan) + up * (uy * tan));
    vec3 ro = pc.b.xyz;

    uint bounces = uint(max(pc.a.w, 1.0));
    vec3 sunDir = normalize(pc.d.xyz);
    vec3 lum = vec3(0.0);
    vec3 throughput = vec3(1.0);
    float seed = float(gid.y * 4096 + gid.x);

    for (uint b = 0u; b < bounces; b++) {
        if (!traceRay(ro, rd, 500.0)) { lum += throughput * skyColor(rd); break; }
        vec3 alb = albedoOf(hitPrim / 12u);

        float ndl = max(dot(hitNrm, sunDir), 0.0);
        if (ndl > 0.0 && !traceRay(hitPos + hitNrm * 0.002, sunDir, 499.0))
            lum += throughput * alb * pc.e.rgb * ndl * 2.2;

        throughput *= alb;
        ro = hitPos + hitNrm * 0.002;
        rd = lambertianBounce(hitNrm, seed + float(b) * 7.13);
        if (max(throughput.r, max(throughput.g, throughput.b)) < 0.01) break;
        if (b + 1u == bounces && !traceRay(ro, rd, 500.0))
            lum += throughput * skyColor(rd) * 0.5;
    }

    lum *= pc.e.w;
    lum = clamp((lum * (2.51 * lum + 0.03)) / (lum * (2.43 * lum + 0.59) + 0.14), 0.0, 1.0);
    lum = mix(lum * 12.92,
              1.055 * pow(max(lum, vec3(1e-4)), vec3(1.0 / 2.4)) - 0.055,
              step(vec3(0.0031308), lum));
    imageStore(OutImg, gid, vec4(lum, 1.0));
}
