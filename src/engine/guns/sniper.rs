use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;
use std::f32::consts::{FRAC_PI_2, PI};

const BRIGHT: [f32; 3] = [0.62, 0.65, 0.70];
const DARK: [f32; 3] = [0.30, 0.33, 0.37];
const POLY: [f32; 3] = [0.16, 0.17, 0.19];
const DEEP: [f32; 3] = [0.10, 0.10, 0.12];
const WOOD: [f32; 3] = [0.50, 0.34, 0.17];
const OLIVE: [f32; 3] = [0.28, 0.35, 0.22];
const SAND: [f32; 3] = [0.48, 0.39, 0.26];

pub fn sv98() -> GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let parts = vec![
        (t(0.0, 0.045, 0.345) * rz(), cylinder(0.020, 0.55, 12), DARK),
        (t(0.0, 0.045, 0.670) * rz(), frustum(0.034, 0.030, 0.10, 12, true), DARK),
        (t(0.0, 0.075, 0.020), beveled_box(0.055, 0.075, 0.30, 0.012, 4), DARK),
        (t(0.0, 0.111, 0.020), beveled_box(0.030, 0.014, 0.26, 0.004, 4), POLY),
        (t(0.0, 0.035, 0.360), beveled_box(0.055, 0.065, 0.28, 0.010, 4), WOOD),
        (t(0.0, 0.050, -0.220), beveled_box(0.060, 0.140, 0.25, 0.014, 4), WOOD),
        (t(0.0, 0.145, -0.200), beveled_box(0.050, 0.050, 0.16, 0.010, 4), WOOD),
        (t(0.0, 0.050, -0.365), beveled_box(0.058, 0.130, 0.03, 0.008, 4), DEEP),
        (t(0.0, 0.165, 0.020) * rz(), cylinder(0.024, 0.32, 12), BRIGHT),
        (t(0.0, 0.165, 0.185) * rz(), cylinder(0.034, 0.02, 12), DARK),
        (t(0.0, 0.165, 0.197) * rz(), cylinder(0.028, 0.006, 12), DEEP),
        (t(0.0, 0.165, -0.155) * rz(), cylinder(0.030, 0.03, 12), DARK),
        (t(0.0, 0.128, 0.090), beveled_box(0.022, 0.030, 0.03, 0.006, 4), POLY),
        (t(0.0, 0.128, -0.070), beveled_box(0.022, 0.030, 0.03, 0.006, 4), POLY),
        (t(0.032, 0.085, -0.090) * rz(), cylinder(0.009, 0.10, 16), BRIGHT),
        (t(0.082, 0.085, -0.090) * Mat4::from_scale(glam::vec3(0.016, 0.016, 0.016)), sphere(12, 8), BRIGHT),
        (t(0.0, -0.005, 0.010), torus_arc(0.028, 0.006, PI * 1.05, PI * 1.95, 10, 6), DARK),
        (t(0.0, -0.030, 0.090), beveled_box(0.035, 0.090, 0.045, 0.008, 4), POLY),
        (t(0.0, 0.015, 0.010), beveled_box(0.010, 0.030, 0.008, 0.004, 4), DEEP),
        (t(0.0, 0.078, 0.585), beveled_box(0.008, 0.020, 0.008, 0.003, 4), DEEP),
        (t(0.0, 0.075, -0.160), beveled_box(0.050, 0.040, 0.05, 0.008, 4), DARK),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "SV-98M 针叶", length: 1.10 }
}

pub fn m2010() -> GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let parts = vec![
        (t(0.0, 0.045, 0.355) * rz(), cylinder(0.020, 0.55, 12), BRIGHT),
        (t(0.0, 0.045, 0.685) * rz(), frustum(0.036, 0.028, 0.11, 12, true), DARK),
        (t(0.0, 0.080, 0.020), beveled_box(0.055, 0.080, 0.32, 0.012, 4), DARK),
        (t(0.0, 0.117, 0.020), beveled_box(0.030, 0.014, 0.28, 0.004, 4), POLY),
        (t(0.0, 0.035, 0.380), beveled_box(0.056, 0.065, 0.30, 0.010, 4), SAND),
        (t(0.0, 0.055, -0.230), beveled_box(0.060, 0.130, 0.24, 0.012, 4), SAND),
        (t(0.0, 0.065, -0.095), beveled_box(0.058, 0.090, 0.06, 0.010, 4), POLY),
        (t(0.0, 0.055, -0.375), beveled_box(0.058, 0.120, 0.03, 0.008, 4), DEEP),
        (t(0.0, 0.145, -0.240), beveled_box(0.045, 0.045, 0.14, 0.010, 4), POLY),
        (t(0.0, 0.105, -0.240), beveled_box(0.020, 0.040, 0.02, 0.006, 4), DEEP),
        (t(0.0, 0.170, 0.020) * rz(), cylinder(0.024, 0.34, 12), BRIGHT),
        (t(0.0, 0.170, 0.190) * rz(), cylinder(0.035, 0.022, 12), DARK),
        (t(0.0, 0.170, 0.202) * rz(), cylinder(0.028, 0.005, 12), DEEP),
        (t(0.0, 0.170, -0.165) * rz(), cylinder(0.031, 0.03, 12), DARK),
        (t(0.0, 0.145, 0.020), beveled_box(0.030, 0.030, 0.10, 0.006, 4), POLY),
        (t(0.032, 0.090, -0.100) * rz(), cylinder(0.0095, 0.10, 16), BRIGHT),
        (t(0.082, 0.090, -0.100) * Mat4::from_scale(glam::vec3(0.017, 0.017, 0.017)), sphere(12, 8), BRIGHT),
        (t(0.0, -0.005, 0.000), torus_arc(0.028, 0.006, PI * 1.05, PI * 1.95, 10, 6), DARK),
        (t(0.0, -0.030, 0.100), beveled_box(0.038, 0.110, 0.05, 0.008, 4), POLY),
        (t(0.0, -0.020, -0.050) * Mat4::from_rotation_x(0.35), beveled_box(0.045, 0.090, 0.055, 0.012, 4), SAND),
        (t(0.0, 0.020, 0.000), beveled_box(0.010, 0.028, 0.008, 0.004, 4), DEEP),
        (t(0.0, 0.078, 0.600), beveled_box(0.007, 0.018, 0.007, 0.003, 4), DEEP),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "M2010 ESR 界标", length: 1.13 }
}

pub fn mrad() -> GunMesh {
    let t = |x: f32, y: f32, z: f32| Mat4::from_translation(glam::vec3(x, y, z));
    let parts = vec![
        (t(0.0, 0.045, 0.360) * rz(), cylinder(0.020, 0.56, 12), BRIGHT),
        (t(0.0, 0.045, 0.690) * rz(), frustum(0.037, 0.029, 0.10, 12, true), DARK),
        (t(0.0, 0.082, 0.020), beveled_box(0.055, 0.082, 0.33, 0.012, 4), DARK),
        (t(0.0, 0.120, 0.020), beveled_box(0.030, 0.014, 0.29, 0.004, 4), POLY),
        (t(0.0, 0.035, 0.390), beveled_box(0.056, 0.068, 0.32, 0.010, 4), OLIVE),
        (t(0.0, 0.058, -0.230), beveled_box(0.060, 0.135, 0.25, 0.012, 4), OLIVE),
        (t(0.0, 0.070, -0.090), beveled_box(0.058, 0.095, 0.05, 0.010, 4), POLY),
        (t(0.0, 0.058, -0.380), beveled_box(0.058, 0.125, 0.03, 0.008, 4), DEEP),
        (t(0.0, 0.150, -0.240), beveled_box(0.048, 0.050, 0.15, 0.010, 4), POLY),
        (t(0.032, 0.140, -0.240) * Mat4::from_rotation_z(FRAC_PI_2), cylinder(0.009, 0.020, 16), DARK),
        (t(0.0, 0.175, 0.020) * rz(), cylinder(0.024, 0.35, 12), BRIGHT),
        (t(0.0, 0.175, 0.195) * rz(), cylinder(0.036, 0.022, 12), DARK),
        (t(0.0, 0.175, 0.207) * rz(), cylinder(0.030, 0.005, 12), DEEP),
        (t(0.0, 0.175, -0.170) * rz(), cylinder(0.032, 0.03, 12), DARK),
        (t(0.0, 0.148, 0.020), beveled_box(0.030, 0.032, 0.11, 0.006, 4), POLY),
        (t(0.032, 0.092, -0.105) * rz(), cylinder(0.010, 0.105, 16), BRIGHT),
        (t(0.085, 0.092, -0.105) * Mat4::from_scale(glam::vec3(0.018, 0.018, 0.018)), sphere(12, 8), BRIGHT),
        (t(0.0, -0.005, 0.000), torus_arc(0.028, 0.006, PI * 1.05, PI * 1.95, 10, 6), DARK),
        (t(0.0, -0.030, 0.095), beveled_box(0.038, 0.115, 0.05, 0.008, 4), POLY),
        (t(0.0, -0.020, -0.045) * Mat4::from_rotation_x(0.35), beveled_box(0.045, 0.095, 0.055, 0.012, 4), OLIVE),
        (t(0.0, 0.020, 0.000), beveled_box(0.010, 0.028, 0.008, 0.004, 4), DEEP),
        (t(0.0, -0.015, 0.300), cylinder(0.012, 0.030, 16), DARK),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MRAD 巨石", length: 1.14 }
}
