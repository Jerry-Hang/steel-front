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
// 时域累积缓冲（线性 HDR 累加：rgb=Σ样本，a=已累积 spp）。逐像素单写者，无需原子。
layout(set = 0, binding = 3, rgba32f) uniform image2D AccImg;

// 6 x vec4 = 96B，Rust 侧 [[f32;4];6] 逐字段对齐，无填充歧义
// 相机直接传 forward 向量（不传 yaw/pitch）=> 与 engine/camera.rs 的基底严格同源，无前后手风险
layout(push_constant) uniform PC {
    vec4 a; // (resX, resY, tanHalfFov, bounces)
    vec4 b; // camPos.xyz
    vec4 c; // fwd.xyz      = camera.forward()
    vec4 d; // sunDir.xyz   表面->太阳
    vec4 e; // sunColor.rgb, exposure
    vec4 f; // (frameIndex, resetFlag, sppTarget, unused)
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

// 余弦加权漫反射方向；种子 = 像素索引 × 帧索引（帧间必须去相关，否则时域累积不会收敛）
vec3 lambertianBounce(vec3 n, uint px, uint t) {
    uint s = (px * 0x9E3779B1u) ^ (t * 0x85EBCA6Du);
    vec2 e = vec2(hash11(s), hash11(s ^ 0x27D4EB2Du));
    vec3 up = abs(n.x) < 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(up, n));
    vec3 bt = cross(n, tx);
    vec3 d = cosSample(e);
    return normalize(tx * d.x + bt * d.y + n * d.z);
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
    uint pxSeed = uint(gid.y) * 2048u + uint(gid.x);
    uint frameSeed = uint(pc.f.x);

    // 2026-09-01：每帧多 spp 采样（吃满功耗 + 大幅降噪/减拖影）
    const uint SPP = 4u;
    vec3 lum = vec3(0.0);
    for (uint s = 0u; s < SPP; s++) {
        // 每个样本独立抖动（像素内偏移 + 每样本种子），避免重复纹理
        vec3 rs = rd + rgt * (((hash11(pxSeed ^ (frameSeed * 7u) ^ (s * 0x9E3779B1u)) - 0.5) * 2.0) * tan / pc.a.x)
                      + up * (((hash11(pxSeed ^ (frameSeed * 13u) ^ (s * 0x85EBCA6Du)) - 0.5) * 2.0) * tan / pc.a.y);
        vec3 rq = ro;
        vec3 tq = vec3(1.0);
        vec3 lq = vec3(0.0);
        uint seed = pxSeed ^ (frameSeed * 0x27D4EB2Fu) ^ (s * 0x165667B1u);
        for (uint b = 0u; b < bounces; b++) {
            if (!traceRay(rq, rs, 500.0)) { lq += tq * skyColor(rs); break; }
            vec3 alb = albedoOf(hitPrim / 12u);
            float ndl = max(dot(hitNrm, sunDir), 0.0);
            if (ndl > 0.0 && !traceRay(hitPos + hitNrm * 0.002, sunDir, 499.0))
                lq += tq * alb * pc.e.rgb * ndl * 2.2;
            tq *= alb;
            rq = hitPos + hitNrm * 0.002;
            rs = lambertianBounce(hitNrm, pxSeed, seed * 64u + b);
            if (max(tq.r, max(tq.g, tq.b)) < 0.01) break;
            if (b + 1u == bounces && !traceRay(rq, rs, 500.0))
                lq += tq * skyColor(rs) * 0.5;
        }
        lum += lq;
    }

    // 时域累积：线性 HDR 求和，a 通道记已累积样本数；色调映射只作用于运行均值
    // （否则每帧各自 ACES+sRGB 再平均会把高光压平、gamma 域相加也不物理）
    lum *= pc.e.w;
    vec4 acc = imageLoad(AccImg, gid);
    if (pc.f.y > 0.5) { acc = vec4(0.0); }
    acc = vec4(acc.rgb + lum, acc.a + 1.0);
    imageStore(AccImg, gid, acc);

    vec3 outc = acc.rgb / max(acc.a, 1.0);
    outc = clamp((outc * (2.51 * outc + 0.03)) / (outc * (2.43 * outc + 0.59) + 0.14), 0.0, 1.0);
    outc = mix(outc * 12.92,
               1.055 * pow(max(outc, vec3(1e-4)), vec3(1.0 / 2.4)) - 0.055,
               step(vec3(0.0031308), outc));
    imageStore(OutImg, gid, vec4(outc, 1.0));
}
