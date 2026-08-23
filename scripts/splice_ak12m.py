# -*- coding: utf-8 -*-
p = 'src/engine/guns/assault_red.rs'
d = open(p, 'rb').read()
try:
    d.decode('utf-8')
    enc = 'utf-8'
    print('file is utf-8')
except UnicodeDecodeError:
    enc = 'gbk'
    print('file is GBK, converting to utf-8')
s = d.decode(enc)
nl = '\r\n' if '\r\n' in s else '\n'
start = s.index('// ===== AK-12M')
k = s.index('AK-104')
ls = s.rindex('\n', 0, k) + 1
print('span', start, ls)
new_fn = nl.join([
'// ===== AK-12M (5.45x39): black polymer, long handguard + rail, curved 30rd mag,',
'//       skeleton folding stock. Stations: stock rear -0.542, receiver -0.30..+0.06,',
'//       handguard +0.06..+0.21, muzzle brake +0.30..+0.362 (total ~0.90m).',
'//       2026-08-19 rebuild: fixed gaps (mag/grip/stock were floating), magazine curve',
'//       now sweeps forward like the real AK, stock connects to receiver rear.',
'pub fn ak12m() -> crate::engine::guns::GunMesh {',
'    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));',
'    let rx = |a: f32| Mat4::from_rotation_x(a);',
'    let rh = |x: f32, y: f32, z: f32| t(x, y, z) * Mat4::from_rotation_z(-FRAC_PI_2);',
'    let steel = [0.34, 0.36, 0.40];',
'    let black = [0.17, 0.18, 0.20];',
'    let dark = [0.11, 0.12, 0.13];',
'    let bright = [0.55, 0.58, 0.62];',
'    let magc = [0.17, 0.13, 0.10];',
'    let parts = vec![',
'        // receiver (black polymer): z -0.30..+0.06, y 0.04..0.11',
'        (t(0.0, 0.075, -0.12), beveled_box(0.064, 0.070, 0.36, 0.014, 3), black),',
'        // top rail: runs receiver rear over handguard front',
'        (t(0.0, 0.117, -0.035), beveled_box(0.030, 0.014, 0.49, 0.005, 3), dark),',
'        // rear trunnion / stock hinge seat',
'        (t(0.0, 0.075, -0.30), beveled_box(0.050, 0.050, 0.05, 0.010, 3), steel),',
'        // barrel (z 0.06..0.30)',
'        (t(0.0, 0.050, 0.18) * rz(), cylinder(0.014, 0.24, 14), steel),',
'        // gas tube (fills gap between handguard top and rail)',
'        (t(0.0, 0.100, 0.13) * rz(), cylinder(0.011, 0.14, 12), steel),',
'        // handguard',
'        (t(0.0, 0.050, 0.135), beveled_box(0.060, 0.080, 0.15, 0.012, 3), black),',
'        // handguard front collar',
'        (t(0.0, 0.050, 0.206), beveled_box(0.062, 0.084, 0.018, 0.007, 3), black),',
'        // gas block',
'        (t(0.0, 0.060, 0.230), beveled_box(0.036, 0.050, 0.04, 0.008, 3), steel),',
'        // front sight post + guard wings',
'        (t(0.0, 0.104, 0.228), beveled_box(0.010, 0.030, 0.012, 0.003, 3), dark),',
'        (t(-0.016, 0.102, 0.228), beveled_box(0.005, 0.038, 0.010, 0.002, 3), black),',
'        (t(0.016, 0.102, 0.228), beveled_box(0.005, 0.038, 0.010, 0.002, 3), black),',
'        // rear sight on rail',
'        (t(0.0, 0.128, -0.05), beveled_box(0.024, 0.018, 0.030, 0.005, 3), steel),',
'        // charging handle (right side)',
'        (rh(0.042, 0.085, -0.14), cylinder(0.0055, 0.032, 10), steel),',
'        (rh(0.061, 0.085, -0.14), cylinder(0.009, 0.014, 10), black),',
'        // ejection port cover (right side)',
'        (t(0.034, 0.082, -0.09), beveled_box(0.008, 0.026, 0.050, 0.003, 3), dark),',
'        // curved magazine: 3 segments + floor plate (bottom sweeps forward)',
'        (t(0.0, 0.018, -0.130) * rx(-0.15), beveled_box(0.042, 0.075, 0.066, 0.012, 3), magc),',
'        (t(0.0, -0.037, -0.118) * rx(-0.35), beveled_box(0.040, 0.075, 0.062, 0.012, 3), magc),',
'        (t(0.0, -0.092, -0.090) * rx(-0.55), beveled_box(0.038, 0.075, 0.058, 0.012, 3), magc),',
'        (t(0.0, -0.124, -0.070) * rx(-0.60), beveled_box(0.042, 0.020, 0.068, 0.006, 3), dark),',
'        // trigger guard: front post, rear post, bottom beam',
'        (t(0.0, 0.022, -0.160), beveled_box(0.012, 0.050, 0.012, 0.004, 3), black),',
'        (t(0.0, 0.022, -0.225), beveled_box(0.012, 0.050, 0.012, 0.004, 3), black),',
'        (t(0.0, 0.000, -0.1925), beveled_box(0.050, 0.012, 0.078, 0.004, 3), black),',
'        // trigger',
'        (t(0.0, 0.024, -0.185) * rx(0.12), beveled_box(0.012, 0.034, 0.012, 0.004, 3), dark),',
'        // pistol grip (raked back, attached to receiver bottom)',
'        (t(0.0, -0.012, -0.248) * rx(0.22), beveled_box(0.040, 0.125, 0.052, 0.012, 3), black),',
'        // stock: hinge, top strut, rear plate, butt pad, lower strut',
'        (t(0.0, 0.075, -0.315), beveled_box(0.048, 0.055, 0.050, 0.010, 3), black),',
'        (t(0.0, 0.088, -0.415), beveled_box(0.030, 0.024, 0.170, 0.008, 3), black),',
'        (t(0.0, 0.075, -0.505), beveled_box(0.032, 0.095, 0.040, 0.008, 3), black),',
'        (t(0.0, 0.072, -0.533), beveled_box(0.036, 0.105, 0.018, 0.006, 3), dark),',
'        (t(0.0, 0.036, -0.404) * rx(0.066), beveled_box(0.026, 0.020, 0.160, 0.006, 3), black),',
'        // muzzle brake (bright steel)',
'        (t(0.0, 0.050, 0.331) * rz(), frustum(0.016, 0.019, 0.062, 16, true), bright),',
'    ];',
'    let (verts, indices) = assemble(&parts);',
'    GunMesh { verts, indices, display_name: "AK-12M 风暴", length: 0.90 }',
'}',
''])
s2 = s[:start] + new_fn + s[ls:]
open(p, 'w', encoding=enc, newline='').write(s2)
print('done, new size', len(s2))
