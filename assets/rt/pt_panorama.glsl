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
// 🏢 道具逐三角属性表（2026-09-19 道具进 BLAS，用户决策）：每三角 2×u32 =
//    量化面法线（(v·127+127) 每轴）+ 平均顶点色，set_props 里构建，**device-local**。
//    为什么不让着色器直接随机读道具主 VB/IB：那是 HOST_VISIBLE 内存，每次命中跨 PCIe
//    ——pt3 实测 126fps→1.5fps。道具未上传时 Rust 侧绑占位缓冲，那时 BLAS 没有道具
//    几何，道具分支按几何索引必然不可达。
layout(set = 0, binding = 4, std430) readonly buffer PropTris { uint propAttr[]; };

// 7 x vec4 = 112B，Rust 侧 [[f32;4];7] 逐字段对齐，无填充歧义
// 相机直接传 forward 向量（不传 yaw/pitch）=> 与 engine/camera.rs 的基底严格同源，无前后手风险
layout(push_constant) uniform PC {
    vec4 a; // (resX, resY, tanHalfFov, bounces)
    vec4 b; // camPos.xyz
    vec4 c; // fwd.xyz      = camera.forward()
    vec4 d; // sunDir.xyz   表面->太阳
    vec4 e; // sunColor.rgb, exposure
    vec4 f; // (frameIndex, resetFlag, sppTarget, moveAmount)
    vec4 g; // 预留（原 boxTriEnd——分流已改用几何索引，见 traceRay）
} pc;

// 天空 = 补光源（烘焙参照语义），不是显示天空：数值对齐光栅半球环境项
// （lighting.rs ambient (0.5,0.55,0.6)×0.55 ≈ 0.30 支），余弦加权半球均值 ≈ 0.30。
// 显示天空的色差（光栅是艺术清屏色）是已知且刻意的——参照判表面不判天空。
const vec3 SKY_ZENITH  = vec3(0.27, 0.30, 0.36);
const vec3 SKY_HORIZON = vec3(0.32, 0.33, 0.34);
const float SUN_COS = 0.9997;   // 太阳圆盘角阈值
const float PI = 3.14159265;

uint hitPrim;
vec3 hitPos;
vec3 hitNrm;
bool hitIsProp;
vec3 hitAlb;

// 道具逐三角属性解包：w0 = 量化面法线（(v-127)/127），w1 = 平均顶点色（/255）
vec3 ptNormal(uint lp) {
    uint w = propAttr[lp * 2u];
    return (vec3(uvec3(w & 0xFFu, (w >> 8u) & 0xFFu, (w >> 16u) & 0xFFu)) - 127.0) / 127.0;
}
vec3 ptAlbedo(uint lp) {
    uint w = propAttr[lp * 2u + 1u];
    return vec3(uvec3(w & 0xFFu, (w >> 8u) & 0xFFu, (w >> 16u) & 0xFFu)) / 255.0;
}

vec3 skyColor(vec3 rd) {
    float t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 col = mix(SKY_HORIZON, SKY_ZENITH, t * t);
    if (dot(rd, normalize(pc.d.xyz)) > SUN_COS) col += vec3(12.0);
    return col;
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
    // 🏢 按**几何索引**分流（0=盒、1=道具）。ray query 返回的图元索引是**几何内局部**
    //    编号，不跨几何连续——pt3 的灰树冠事故：旧「hitPrim 与全局边界 pc.g.x 比较」
    //    的假设把道具最前的 21480 个三角当成盒，albedoOf 越界拿 0.5 中性灰。
    uint gi = rayQueryGetIntersectionGeometryIndexEXT(rq, true);
    if (gi == 0u) {
        // 盒体路径：面号表（ray_tracer::box_triangles 不变量：6 面按 -X,+X,-Y,+Y,-Z,+Z
        // 各 2 三角顺序展开，故 (primitive % 12) / 2 就是面号）。
        // （旧「来射方向主轴」近似在浅角度下会把地面法线错判成 ±Z，导致 ndl<0、地面全黑）
        hitIsProp = false;
        uint f = (hitPrim % 12u) / 2u;
        hitNrm = f == 0u ? vec3(-1.0, 0.0, 0.0)
                 : f == 1u ? vec3(1.0, 0.0, 0.0)
                 : f == 2u ? vec3(0.0, -1.0, 0.0)
                 : f == 3u ? vec3(0.0, 1.0, 0.0)
                 : f == 4u ? vec3(0.0, 0.0, -1.0)
                 : vec3(0.0, 0.0, 1.0);
    } else {
        // 🏢 道具路径：法线从属性表解包后翻到**迎向来射**一侧（道具是闭合壳，外表面
        //    即命中面，与绕序无关）；albedo = 平均顶点色（逐摆放 tint 已烘进去）。
        hitIsProp = true;
        vec3 n = ptNormal(hitPrim);
        if (dot(n, rd) > 0.0) n = -n;
        hitNrm = normalize(n);
        hitAlb = ptAlbedo(hitPrim);
    }
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

    // 2026-09-01v2：运动自适应 spp！移动/跳跃 => 瞬时多采样（短时域窗口），静止 => 长时域累积
    float move = clamp(pc.f.w, 0.0, 1.0);
    uint SPP = uint(round(mix(16.0, 64.0, move)));
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
            vec3 alb = hitIsProp ? hitAlb : albedoOf(hitPrim / 12u);
            float ndl = max(dot(hitNrm, sunDir), 0.0);
            // 2026-09-01v3：偏移 0.02 防阴影内棱线（acne）；太阳盘 jitter 2 点 = 软边 + 更准
            if (ndl > 0.0) {
                vec3 sh_o = hitPos + hitNrm * 0.02;
                vec3 sd1 = normalize(sunDir + rgt * 0.0012 + up * 0.0012);
                vec3 sd2 = normalize(sunDir - rgt * 0.0012 - up * 0.0012);
                float lit1 = traceRay(sh_o, sd1, 499.0) ? 0.0 : 1.0;
                float lit2 = traceRay(sh_o, sd2, 499.0) ? 0.0 : 1.0;
                // 两点抖动取和×0.5 = 均值归一：全照 = 1.0×sun×ndl，与光栅
                // evaluate_directional 同尺度（旧 ×1.1 使全照太阳高 2.2 倍）
                lq += tq * alb * pc.e.rgb * ndl * (lit1 + lit2) * 0.5;
            }
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
    // 运动自适应时域窗口：运动大 => 窗口短（10 帧，快速丢弃旧视角=去拖影）+ spp 高（瞬时降噪）；
    // 静止 => 窗口长（64 帧，时域收割=干净）
    float win = mix(64.0f, 1.0f, move);
    float a = min(acc.a + float(SPP), win);
    float alpha = 1.0 / a;
    acc = vec4(mix(acc.rgb, lum, alpha), a);
    imageStore(AccImg, gid, acc);

    vec3 outc = acc.rgb / max(acc.a, 1.0);
    // 色调映射与光栅 apply_lighting 同源（build.rs: 1-exp(-x*1.55) 指数压缩，不截顶）——
    // 参照帧与实机帧必须走同一条曲线，否则分区偏差表测的是曲线差而不是光照差
    outc = vec3(1.0) - exp(-clamp(outc, vec3(0.0), vec3(16.0)) * 1.55);
    outc = mix(outc * 12.92,
               1.055 * pow(max(outc, vec3(1e-4)), vec3(1.0 / 2.4)) - 0.055,
               step(vec3(0.0031308), outc));
    imageStore(OutImg, gid, vec4(outc, 1.0));
}
