const API = [
  '[meshgen API]',
  'pub struct GVertex { pos:[f32;3], normal:[f32;3], uv:[f32;2], color:[f32;3] }',
  'pub struct Mesh { verts:Vec<GVertex>, indices:Vec<u32> }',
  'Mesh::append_transformed(&self, out_verts, out_indices, m:glam::Mat4, tint:[f32;3], light_dir:glam::Vec3, ambient:f32, diffuse:f32)',
  'pub fn beveled_box(w:f32,h:f32,d:f32,r:f32,seg:u32)->Mesh',
  'pub fn frustum(r0:f32,r1:f32,height:f32,seg:u32,caps:bool)->Mesh',
  'pub fn cylinder(r:f32,height:f32,seg:u32)->Mesh',
  'pub fn sphere(seg:u32,rings:u32)->Mesh',
  'pub fn torus_arc(ring_r:f32,tube_r:f32,t0:f32,t1:f32,seg_ring:u32,seg_tube:u32)->Mesh',
  '',
  '[坐标约定] 枪局部坐标：y上、x右、z前(枪口朝+Z)。圆柱/锥台默认沿Y，转+Z用已有常量 rz()（Mat4::from_rotation_x(-FRAC_PI_2)）。',
  '',
  '[assemble 助手] pub fn assemble(parts:&[(Mat4, Mesh, [f32;3])])->(Vec<GVertex>,Vec<u32>)',
  'pub struct GunMesh { pub verts:Vec<GVertex>, pub indices:Vec<u32>, pub display_name:&static str, pub length:f32 }',
  '',
  '[立体感设计规范（必须严格遵守）]',
  '1. 厚实度：枪管/枪身主件半径或厚度必须厚实——步枪/机枪管 r>=0.015，手枪/冲锋枪管 r>=0.011；',
  '   机匣/护木/枪托等主体件宽度 >=0.055、高度 >=0.06，绝不用薄片或细丝当主体。',
  '2. 部件数量：每把枪 14~22 个部件，覆盖：枪管、护木、机匣、弹匣/弹鼓、握把、枪托、',
  '   扳机护圈、准星/表尺或瞄具、枪口装置（制退器/消音器/消焰器）、拉机柄等。',
  '3. 材质对比（同一把枪至少 4 种颜色区分层次）：',
  '   亮钢 [0.62,0.65,0.70] / 深钢 [0.30,0.33,0.37] / 黑聚合 [0.16,0.17,0.19] / 深黑 [0.10,0.10,0.12]',
  '   木色 [0.50,0.34,0.17] / 橄榄绿 [0.28,0.35,0.22] / 沙色 [0.48,0.39,0.26]。',
  '4. 层次感：部件沿枪轴前后错落、上下分阶（机匣高于枪管、护木包住枪管上部、弹匣下垂、',
  '   枪托在后方），避免所有部件挤在同一平面。',
  '5. 圆润过渡：主体件用 beveled_box 圆角（r>=0.008），圆柱件用 16~24 段保证圆滑。',
  '6. 光照：assemble 统一烘焙（环境光 0.32 / 漫反射 0.78，方向光左上），只需给材质色。',
  '',
  '[输出要求]',
  '只输出一个完整 Rust 文件内容（每把枪一个 pub fn <name>() -> crate::engine::guns::GunMesh）。',
  '文件头：use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc}; use crate::engine::guns::{assemble, GunMesh, rz}; use glam::Mat4;',
  '每把枪函数体：let parts = vec![(矩阵, mesh, tint), ...]; let (verts, indices) = assemble(&parts); GunMesh { verts, indices, display_name: "中文名", length: 总长 }。',
  '矩阵可用局部闭包 let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z)); 与 rz() 组合。',
  '不要输出解释/markdown围栏/额外文字，只要 Rust 代码。',
].join('\n');
function group(guns, spec) {
  return '你是资深枪械3D建模Rust程序员。用程序化网格为游戏生成立体感十足的枪械模型（数学方式，不用外部模型）。\n\n' + API + '\n\n[本组枪械建模要点]\n' + spec + '\n\n[枪械清单]\n' + guns + '\n请严格按照清单为每把枪实现 pub fn，函数名必须与清单一致。';
}
const allGroups = [
  { name: 'assault_red', guns: 'pub fn ak12m()->GunMesh // AK-12M 风暴：AK系，厚实长护木，30发弧形弹匣，折叠聚合物托，铁瞄+表尺\npub fn ak104()->GunMesh // AK-104 短剑：紧凑AK，短粗管+消焰器，弧形弹匣，折叠托\npub fn ash12()->GunMesh // ASh-12 破城锤：重型突击，粗短枪管+大型枪口制退器，20发短弹匣，粗壮枪身', spec: 'AK系：机匣为厚实长圆角盒(0.065宽0.075高0.27长,圆角0.012)，护木厚实(0.06宽0.05高0.24长)包住枪管上方，枪管粗(r0.015)带准星，导气管粗圆柱，弹匣弧形(两段圆角盒错位+微倾)突出体积，枪托厚实(聚合物折叠托:主托板0.06×0.12×0.09+细连杆两根)，握把粗斜(0.045×0.14×0.05)，扳机护圈粗环，枪口制退器粗筒。ASh-12枪身明显更粗壮(机匣0.075宽)，大型制退器(0.05半径0.09长)。每把枪16-22部件，至少4种材质色，立体层次分明。' },
  { name: 'assault_blue', guns: 'pub fn hk416()->GunMesh // HK416 A8 游隼：AR系，全长导轨护木，30发直弹匣，伸缩托，红点镜\npub fn mk18()->GunMesh // MK18 隼爪：短管AR，紧凑，粗消音器，直弹匣，短护木', spec: 'AR系：上下机匣分色(上钢下黑聚合)，顶部粗导轨条贯穿，方形护木厚实(0.06×0.06×0.22)带散热槽，直弹匣(上直下微弧两段)突出，缓冲管+厚实伸缩托(底板0.06×0.12×0.015)，A2粗握把，鸟笼消焰器(粗短筒带槽)，红点镜(粗盒+镜片圆柱)。MK18短管+粗消音器(0.03半径0.18长)粗壮。每把枪16-22部件，至少4种材质色。' },
  { name: 'smg_red', guns: 'pub fn pp19()->GunMesh // PP-19-01 勇士：冲锋枪，大螺旋弹筒(粗圆筒)在机匣下方，短管，折叠托\npub fn pp9()->GunMesh // PP-9 胡蜂：微声冲锋，一体粗消音器，细长机匣，折叠托\npub fn vss()->GunMesh // VSS Vintorez：特种微声，一体粗消音器，厚木托+木握把，10发弹匣\npub fn asval()->GunMesh // AS Val：特种突击，一体粗消音器，聚合物折叠托，20发弹匣', spec: '冲锋/特种：机匣窄长但厚实(0.055宽0.06高0.22长)，PP-19弹筒为大圆柱(r0.035长0.2)竖挂机匣下前部，消音器类枪管粗壮(r0.022-0.026)，护木厚实，VSS/AS Val枪身一体粗壮，木色/橄榄绿材质，折叠托厚实。每把枪14-20部件，4种以上材质色，立体感强。' },
  { name: 'smg_blue', guns: 'pub fn mpx()->GunMesh // MPX 燕鸥：冲锋枪，AR风格短护木，直弹匣，伸缩托\npub fn mp5sd()->GunMesh // MP5SD 雨燕：微声冲锋，粗一体消音器，手枪握把，固定厚托\npub fn p90()->GunMesh // P90：无托PDW，顶部扁平弹匣，流线粗枪身，握把一体\npub fn mp7()->GunMesh // MP7：紧凑PDW，短粗枪身，折叠托', spec: 'MPX：AR式厚实机匣+方形短护木+直弹匣。MP5SD：粗消音器(r0.024长0.3)一体，厚实机匣。P90：无托流线粗枪身(0.06宽0.07高0.5长整体)，顶部扁平弹匣(0.045×0.03×0.35)醒目，握把粗。MP7：短粗枪身(0.055宽0.06高0.38长)，粗管，折叠托。每把枪14-20部件，4种以上材质色。' },
  { name: 'dmr', guns: 'pub fn svd12()->GunMesh // SVD-12M 支点：精确射手，厚木托带托腮，10发弹匣，粗管+瞄准镜\npub fn m110a1()->GunMesh // M110A1 信使：AR系精确，长粗管，全长导轨，可调托\npub fn mk14p()->GunMesh // MK14P 仲裁者：M14系，厚木托，20发直弹匣，长粗管', spec: '精确射手：粗长管(r0.018长0.45)，SVD厚木托(斜切+托腮凸起0.05高)，M110长护木导轨+可调托，MK14厚木托+机匣前弹匣。大瞄准镜(粗圆柱0.022半径+两端镜头)架在镜桥上。每把枪16-22部件，4种以上材质色，层次分明。' },
  { name: 'sniper', guns: 'pub fn sv98()->GunMesh // SV-98M 针叶：栓动狙击，厚木托，粗管，大瞄准镜\npub fn m2010()->GunMesh // M2010 ESR 界标：栓动狙击，聚合物折叠托，粗管+制退器\npub fn mrad()->GunMesh // MRAD 巨石：栓动狙击，模块化托，可调托腮，粗管', spec: '栓动狙击：粗长枪管(r0.02长0.55)，枪托厚实含托腮(主托0.06×0.14×0.25+托腮0.05高凸起)，机匣上方大瞄准镜(粗圆柱0.024半径+两端镜头圆盘)，枪口制退器(短粗筒0.035半径)，拉机柄粗杆。全长1.1-1.3m。每把枪16-22部件，4种以上材质色。' },
  { name: 'antimaterial', guns: 'pub fn osv96()->GunMesh // OSV-96 削岩：反器材，12.7x108，粗长管+大型制退器，无托布局\npub fn m82a1()->GunMesh // M82A1：反器材，.50，大型制退器，可调托，粗壮枪身，10发弹匣', spec: '反器材：粗长管(r0.028长0.7)+大型多槽制退器(0.05半径0.1长)，枪身粗壮(机匣0.08宽)，OSV无托弹匣在握把后方，M82托带缓冲管+厚底板。每把枪16-22部件，4种以上材质色。' },
  { name: 'lmg_red', guns: 'pub fn rpk16()->GunMesh // RPK-16 桦木：轻机枪，AK系长机匣，45发长弹匣/弹鼓，长管，折叠托\npub fn pkm()->GunMesh // PKM 钢线：通用机枪，弹链盒，粗管，两脚架\npub fn pkp()->GunMesh // PKP 佩切涅格：通用机枪，重管带散热套筒，弹链', spec: '机枪系：AK式厚实机匣(略大0.07宽)，RPK长管+弹鼓(扁圆柱0.045半径0.09厚)，PKM/PKP机匣右侧弹链盒(扁盒0.07×0.16×0.05)醒目，枪管带散热套筒(粗圆柱0.03套细管0.02)，前端两脚架(两根粗杆0.008半径)，厚实木/聚合物托。每把枪16-22部件，4种以上材质色。' },
  { name: 'lmg_blue', guns: 'pub fn m249()->GunMesh // M249 SAAR 蜂群：轻机枪，5.56，机匣带提把，方形厚护木，两脚架\npub fn m240l()->GunMesh // M240L 铁砧：通用机枪，7.62，细长管，枪托带缓冲', spec: 'M249：AR式厚实机匣+方形护木+弹箱(机匣下扁盒0.07×0.18×0.06)+粗提把。M240L：细长厚实机匣+弹链盒+固定托。每把枪16-22部件，4种以上材质色。' },
  { name: 'hmg', guns: 'pub fn rope12()->GunMesh // 绳结 12.7mm 重机枪：粗管+大制退器，重枪身\npub fn m2a1()->GunMesh // M2A1 硬汉：.50重机枪，粗管带散热环，重托，大弹箱', spec: '重机枪：粗长管(r0.03长0.8)+大制退器，枪身粗壮(机匣0.08宽0.09高)，M2枪管带散热环(多个细环0.033半径套在管上)，大型弹箱(矩形盒0.08×0.2×0.07)在机匣侧，厚实重托。每把枪16-22部件，4种以上材质色。' },
  { name: 'shotgun', guns: 'pub fn saiga12()->GunMesh // 圆木 Saiga-12：半自动霰弹，AK系，管式弹仓，厚木托\npub fn m1014()->GunMesh // M1014 破门：半自动霰弹，伸缩托，管式弹仓，粗管\npub fn aa12()->GunMesh // AA12 风暴：全自动霰弹，重型枪身，箱式弹匣，粗管', spec: '霰弹枪：粗管(r0.02长0.5)，管式弹仓在枪管下方(细长圆柱0.014半径)醒目，Saiga AK式厚实机匣+厚木托，M1014粗伸缩托，AA12粗壮枪身+厚盒弹匣。每把枪16-22部件，4种以上材质色。' },
  { name: 'pistols', guns: 'pub fn mp443()->GunMesh // MP-443 乌鸦：手枪，9x19，双动，塑料厚握把\npub fn rsh12()->GunMesh // RSh-12 撞锤：左轮，12.7x55，大转轮\npub fn m18()->GunMesh // M18 信标：手枪，9x19，紧凑\npub fn mk23()->GunMesh // Mk23 海豹：重型手枪，.45，带螺纹枪管', spec: '手枪：粗枪管(r0.013长0.13)，套筒厚实(圆角盒0.034宽0.04高0.2长,圆角0.008)，握把粗(斜梯形0.03×0.11×0.035)，扳机护圈粗环，RSh-12大转轮(粗圆柱0.035半径0.045长)在机匣后部醒目。每把枪10-16部件，4种以上材质色。' },
];
const groups = allGroups.slice((args && args.start) || 0, ((args && args.start) || 0) + ((args && args.count) || 6));
const tasks = groups.map(g => ({ label: g.name, prompt: group(g.guns, g.spec) }));
const results = await parallel(tasks.map(t => () => agent(t.prompt, { label: t.label, phase: '建模' })));
const files = {};
tasks.forEach((t, i) => { if (results[i]) files[t.label + '.rs'] = results[i]; });
return { files };