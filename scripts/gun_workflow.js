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
  '[坐标约定] 枪局部坐标：y上、x右、z前(枪口朝+Z)。圆柱/锥台默认沿Y，转+Z用已有常量 RZ(Mat4::from_rotation_x(-FRAC_PI_2))。',
  '',
  '[assemble 助手] pub fn assemble(parts:&[(Mat4, Mesh, [f32;3])])->(Vec<GVertex>,Vec<u32>)',
  'pub struct GunMesh { pub verts:Vec<GVertex>, pub indices:Vec<u32>, pub display_name:&static str, pub length:f32 }',
  '',
  '[材质参考]',
  '深灰钢/枪机 [0.22,0.24,0.27] 钢 [0.45,0.48,0.52] 亮钢 [0.6,0.63,0.67] 黑聚合物 [0.13,0.14,0.16]',
  '深黑 [0.08,0.08,0.10] 木色 [0.45,0.30,0.16] 橄榄绿 [0.25,0.32,0.20] 沙漠棕 [0.45,0.36,0.24] 消音器深灰 [0.30,0.31,0.34]',
  '',
  '[输出要求]',
  '只输出一个完整 Rust 文件内容（每把枪一个 pub fn <name>() -> crate::engine::guns::GunMesh）。',
  '文件头：use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc}; use crate::engine::guns::{assemble, GunMesh, RZ}; use glam::Mat4;',
  '每把枪函数体：let parts = vec![(矩阵, mesh, tint), ...]; let (verts, indices) = assemble(&parts); GunMesh { verts, indices, display_name: "中文名", length: 总长 }。',
  'beveled_box 部件矩阵无需旋转(默认沿枪轴)。圆柱/锥台/球需 RZ 旋转使沿+Z。',
  '不要输出解释/markdown围栏/额外文字，只要 Rust 代码。',
].join('\n');
function group(guns, spec) {
  return '你是枪械3D建模Rust程序员。用程序化网格为游戏生成以下枪械模型（数学方式，不用外部模型）。\n\n' + API + '\n\n[本组枪械建模要点]\n' + spec + '\n\n[枪械清单]\n' + guns + '\n请严格按照清单为每把枪实现 pub fn，函数名必须与清单一致。';
}
const allGroups = [
  { name: 'assault_red', guns: 'pub fn ak12m()->GunMesh // AK-12M 风暴：AK系，长护木带散热孔，30发弧形弹匣，可折叠聚合物托，铁瞄，7.62x39\npub fn ak104()->GunMesh // AK-104 短剑：紧凑AK，短管，30发弧形弹匣，折叠托\npub fn ash12()->GunMesh // ASh-12 破城锤：重型突击，粗短枪管+大型枪口制退器，20发短弹匣，粗壮枪身', spec: 'AK系特征：机匣为长矩形圆角盒(约0.06宽0.07高0.26长)，前接护木(略窄矩形带散热缝可用细条表示)，枪管细圆柱，弹匣弧形(用略弯的圆角盒，旋转绕X约15度)位于机匣下方偏前，枪托在机匣后(折叠托为细杆+底板)，握把斜向后下，准星在前端上方小片，表尺在机匣顶部。AK-12M全长约0.94m，AK-104约0.82m，ASh-12约1.0m。每把枪至少10个部件，尺寸比例协调。' },
  { name: 'assault_blue', guns: 'pub fn hk416()->GunMesh // HK416 A8 游隼：AR系，全长导轨护木，30发直弹匣，伸缩托，提把/红点\npub fn mk18()->GunMesh // MK18 隼爪：短管AR，紧凑，10.3寸管，消音器可选，直弹匣，短护木', spec: 'AR系特征：机匣圆角盒，顶部有导轨(细长条)贯穿机匣与护木，护木方形断面(带孔可用小圆点或凹槽表示)，直弹匣(微弯)在机匣下方，缓冲管+伸缩托在后，握把(AR经典A2握把斜后下)，准星/表尺或红点镜(小圆盒)在导轨上。HK416全长约0.8m(托收起)，MK18约0.7m。每把枪至少10个部件。' },
  { name: 'smg_red', guns: 'pub fn pp19()->GunMesh // PP-19-01 勇士：冲锋枪，螺旋弹筒在机匣下方(粗圆筒)，短管，折叠托\npub fn pp9()->GunMesh // PP-9 胡蜂：微声冲锋，一体消音器(粗长管)，细长机匣，折叠托\npub fn vss()->GunMesh // VSS Vintorez：特种微声，一体消音器，木制枪托/握把，10发弹匣\npub fn asval()->GunMesh // AS Val：特种突击，一体消音器，聚合物折叠托，20发弹匣', spec: '冲锋/特种系特征：机匣窄长(约0.05宽0.06高0.22长)，PP-19弹筒为大圆柱(半径0.03)竖在机匣下前部，消音器类枪管为粗长圆柱(半径0.02-0.025)。VSS/AS Val护木与枪管一体粗壮，木色/橄榄绿。全长约0.6-0.9m。每把枪至少8个部件。' },
  { name: 'smg_blue', guns: 'pub fn mpx()->GunMesh // MPX 燕鸥：冲锋枪，AR风格短护木，直弹匣，伸缩托\npub fn mp5sd()->GunMesh // MP5SD 雨燕：微声冲锋，粗一体消音器，手枪握把，固定聚合物托\npub fn p90()->GunMesh // P90：无托PDW，顶部弹匣(扁平长条)，细长枪身，握把一体\npub fn mp7()->GunMesh // MP7：紧凑PDW，短粗枪身，折叠托，握把式', spec: 'MPX：AR式机匣+方形短护木+直弹匣。MP5SD：细长机匣+粗消音器(整管)。P90：无托流线型，顶部弹匣为扁平盒，枪身一体流线。MP7：紧凑，握把护圈一体，粗短。全长0.4-0.65m。每把枪至少8个部件。' },
  { name: 'dmr', guns: 'pub fn svd12()->GunMesh // SVD-12M 支点：精确射手，木质枪托带托腮，10发弹匣，镜桥+瞄准镜，粗长管\npub fn m110a1()->GunMesh // M110A1 信使：AR系精确，长管，全长导轨，可调托\npub fn mk14p()->GunMesh // MK14P 仲裁者：M14系，木托，20发直弹匣，长管', spec: '精确射手系：长管(0.02半径0.4长)，SVD木托斜切+托腮，M110AR系长护木导轨，MK14木托+机匣前弹匣。全长1.0-1.2m。每把枪至少10个部件。' },
  { name: 'sniper', guns: 'pub fn sv98()->GunMesh // SV-98M 针叶：栓动狙击，木托，无托腮短托，粗管，瞄准镜\npub fn m2010()->GunMesh // M2010 ESR 界标：栓动狙击，聚合物折叠托，粗管+制退器\npub fn mrad()->GunMesh // MRAD 巨石：栓动狙击，模块化托，可调托腮，粗管', spec: '栓动狙击：粗长枪管(0.016-0.02半径0.5-0.6长)，枪托含托腮(矩形+凸起)，机匣上方大瞄准镜(圆柱+两端镜头)，枪口制退器(短粗圆柱)。全长1.1-1.3m。每把枪至少9个部件。' },
  { name: 'antimaterial', guns: 'pub fn osv96()->GunMesh // OSV-96 削岩：反器材，12.7x108，长粗管+大型制退器，无托布局(弹匣在握把后)\npub fn m82a1()->GunMesh // M82A1：反器材，.50，大型制退器，可调托，粗壮枪身，10发弹匣', spec: '反器材：粗长管(0.025-0.03半径0.7长)+大制退器(带孔粗筒)，枪身粗壮，OSV无托弹匣在握把后方，M82托带缓冲管。全长1.4-1.5m。每把枪至少10个部件。' },
  { name: 'lmg_red', guns: 'pub fn rpk16()->GunMesh // RPK-16 桦木：轻机枪，AK系长机匣，45发长弹匣/弹鼓，长管，折叠托\npub fn pkm()->GunMesh // PKM 钢线：通用机枪，弹链供弹(机匣侧扁盒)，粗管，两脚架\npub fn pkp()->GunMesh // PKP 佩切涅格：通用机枪，重管带散热套(管上套筒)，弹链', spec: '机枪系：AK式机匣(略大)，RPK长管+弹鼓(扁圆柱)，PKM/PKP机匣右侧弹链盒(扁盒)，枪管带散热套筒(粗圆柱套细圆柱)，前端两脚架(两条斜杆)。全长1.1-1.2m。每把枪至少10个部件。' },
  { name: 'lmg_blue', guns: 'pub fn m249()->GunMesh // M249 SAAR 蜂群：轻机枪，5.56弹链/弹匣双供，机匣带提把，方形护木，两脚架\npub fn m240l()->GunMesh // M240L 铁砧：通用机枪，7.62弹链，细长管，枪托带缓冲', spec: 'M249：AR式机匣+方形护木+弹箱(机匣下扁盒)+提把。M240L：细长机匣+弹链盒+固定托。全长1.0-1.2m。每把枪至少10个部件。' },
  { name: 'hmg', guns: 'pub fn rope12()->GunMesh // 绳结 12.7mm 重机枪：粗管+大制退器，重枪身\npub fn m2a1()->GunMesh // M2A1 硬汉：.50重机枪，粗管带散热槽，重托，大弹箱', spec: '重机枪：粗长管(0.03半径0.8长)，枪身粗壮，M2枪管带散热孔(环纹用多个细环)，大型弹箱(矩形盒)。全长1.4-1.6m。每把枪至少10个部件。' },
  { name: 'shotgun', guns: 'pub fn saiga12()->GunMesh // 圆木 Saiga-12：半自动霰弹，AK系，管式弹仓(管下)，木托\npub fn m1014()->GunMesh // M1014 破门：半自动霰弹，伸缩托，管式弹仓，粗管\npub fn aa12()->GunMesh // AA12 风暴：全自动霰弹，重型枪身，箱式弹匣，粗管', spec: '霰弹枪：粗管(0.018-0.022半径0.5长)，管式弹仓在枪管下方(细长圆柱)，Saiga AK式机匣+木托，M1014伸缩托，AA12粗壮带盒式弹匣。全长0.9-1.0m。每把枪至少9个部件。' },
  { name: 'pistols', guns: 'pub fn mp443()->GunMesh // MP-443 乌鸦：手枪，9x19，双动，塑料握把\npub fn rsh12()->GunMesh // RSh-12 撞锤：左轮，12.7x55，转轮(短粗圆柱)\npub fn m18()->GunMesh // M18 信标：手枪，9x19，紧凑\npub fn mk23()->GunMesh // Mk23 海豹：重型手枪，.45，带螺纹枪管', spec: '手枪：短枪管(0.012半径0.12长)，套筒(圆角盒0.03宽0.035高0.2长)，握把(斜向后下的梯形盒)，扳机护圈(小环)。RSh-12转轮为大圆柱在机匣后。全长0.2-0.4m。每把枪至少6个部件。' },
];
const groups = args && args.half === 2 ? allGroups.slice(6) : allGroups.slice(0, 6);
const tasks = groups.map(g => ({ label: g.name, prompt: group(g.guns, g.spec) }));
const results = await parallel(tasks.map(t => () => agent(t.prompt, { label: t.label, phase: '建模' })));
const files = {};
tasks.forEach((t, i) => { if (results[i]) files[t.label + '.rs'] = results[i]; });
return { files };
