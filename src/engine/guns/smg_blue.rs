use crate::engine::meshgen::{beveled_box, cylinder, frustum, sphere, torus_arc};
use crate::engine::guns::{assemble, GunMesh, rz};
use glam::Mat4;

pub fn mpx() -> GunMesh {
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let polymer = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];

    let b = |w: f32, h: f32, d: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)), beveled_box(w, h, d, 0.006, 2), t)
    };
    let c = |r: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), cylinder(r, h, 16), t)
    };
    let f = |r0: f32, r1: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), frustum(r0, r1, h, 16, true), t)
    };
    let s = |rad: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * Mat4::from_scale(glam::vec3(rad, rad, rad)), sphere(12, 8), t)
    };
    let ring = |rr: f32, tr: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), torus_arc(rr, tr, 0.0, std::f32::consts::TAU, 24, 8), t)
    };

    let parts = vec![
        c(0.011, 0.14, 0.0, 0.0, 0.405, steel),
        f(0.015, 0.018, 0.045, 0.0, 0.0, 0.4975, black),
        ring(0.019, 0.0035, 0.0, 0.0, 0.51, black),
        b(0.034, 0.04, 0.035, 0.0, 0.02, 0.40, dark_steel),
        b(0.008, 0.02, 0.008, 0.0, 0.048, 0.40, black),
        b(0.064, 0.078, 0.12, 0.0, -0.004, 0.33, polymer),
        b(0.03, 0.015, 0.05, 0.0, -0.05, 0.33, polymer),
        b(0.056, 0.072, 0.15, 0.0, 0.006, 0.235, steel),
        b(0.032, 0.014, 0.17, 0.0, 0.054, 0.235, dark_steel),
        b(0.028, 0.028, 0.02, 0.0, 0.058, 0.17, black),
        b(0.008, 0.014, 0.05, 0.03, 0.02, 0.26, dark_steel),
        b(0.05, 0.052, 0.13, 0.0, -0.032, 0.225, polymer),
        b(0.048, 0.045, 0.06, 0.0, -0.065, 0.21, polymer),
        b(0.036, 0.11, 0.058, 0.0, -0.14, 0.21, polymer),
        b(0.04, 0.014, 0.064, 0.0, -0.202, 0.21, black),
        b(0.042, 0.085, 0.052, 0.0, -0.085, 0.175, polymer),
        b(0.012, 0.032, 0.055, 0.0, -0.062, 0.19, polymer),
        c(0.014, 0.12, 0.0, 0.004, 0.10, dark_steel),
        b(0.05, 0.075, 0.10, 0.0, -0.006, 0.01, polymer),
        b(0.032, 0.022, 0.06, 0.0, 0.032, 0.01, black),
        s(0.006, -0.03, -0.02, 0.28, black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MPX 燕鸥", length: 0.56 }
}

pub fn mp5sd() -> GunMesh {
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let polymer = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];
    let supp = [0.30, 0.31, 0.34];

    let b = |w: f32, h: f32, d: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)), beveled_box(w, h, d, 0.006, 2), t)
    };
    let c = |r: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), cylinder(r, h, 16), t)
    };
    let f = |r0: f32, r1: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), frustum(r0, r1, h, 16, true), t)
    };
    let s = |rad: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * Mat4::from_scale(glam::vec3(rad, rad, rad)), sphere(12, 8), t)
    };
    let ring = |rr: f32, tr: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), torus_arc(rr, tr, 0.0, std::f32::consts::TAU, 24, 8), t)
    };

    let parts = vec![
        c(0.032, 0.19, 0.0, 0.0, 0.50, supp),
        f(0.032, 0.027, 0.025, 0.0, 0.0, 0.6075, supp),
        c(0.036, 0.03, 0.0, 0.0, 0.39, dark_steel),
        b(0.076, 0.045, 0.17, 0.0, -0.028, 0.525, polymer),
        b(0.012, 0.022, 0.012, 0.0, 0.042, 0.56, dark_steel),
        ring(0.014, 0.0035, 0.0, 0.052, 0.56, black),
        b(0.048, 0.058, 0.20, 0.0, 0.005, 0.29, steel),
        c(0.016, 0.16, 0.0, 0.042, 0.29, dark_steel),
        s(0.008, -0.02, 0.042, 0.22, black),
        c(0.016, 0.02, 0.0, 0.045, 0.20, dark_steel),
        b(0.009, 0.04, 0.05, 0.026, 0.005, 0.25, black),
        b(0.044, 0.048, 0.13, 0.0, -0.035, 0.265, polymer),
        b(0.04, 0.085, 0.052, 0.0, -0.088, 0.245, polymer),
        b(0.012, 0.03, 0.05, 0.0, -0.06, 0.23, polymer),
        b(0.036, 0.05, 0.055, 0.0, -0.078, 0.27, steel),
        b(0.034, 0.05, 0.055, 0.0, -0.128, 0.255, steel),
        b(0.032, 0.05, 0.055, 0.0, -0.178, 0.24, steel),
        b(0.038, 0.012, 0.06, 0.0, -0.204, 0.235, black),
        b(0.05, 0.062, 0.14, 0.0, -0.004, 0.12, polymer),
        b(0.03, 0.028, 0.09, 0.0, 0.026, 0.12, polymer),
        b(0.052, 0.07, 0.015, 0.0, -0.004, 0.0425, black),
        ring(0.008, 0.002, 0.0, -0.045, 0.06, black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MP5SD 雨燕", length: 0.59 }
}

pub fn p90() -> GunMesh {
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let polymer = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];
    let olive = [0.25, 0.32, 0.20];

    let b = |w: f32, h: f32, d: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)), beveled_box(w, h, d, 0.006, 2), t)
    };
    let c = |r: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), cylinder(r, h, 16), t)
    };
    let f = |r0: f32, r1: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), frustum(r0, r1, h, 16, true), t)
    };
    let s = |rad: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * Mat4::from_scale(glam::vec3(rad, rad, rad)), sphere(12, 8), t)
    };
    let ring = |rr: f32, tr: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), torus_arc(rr, tr, 0.0, std::f32::consts::TAU, 24, 8), t)
    };

    let parts = vec![
        c(0.011, 0.11, 0.0, 0.0, 0.40, steel),
        f(0.016, 0.02, 0.045, 0.0, 0.0, 0.4775, black),
        ring(0.021, 0.003, 0.0, 0.0, 0.485, black),
        b(0.058, 0.072, 0.39, 0.0, -0.004, 0.27, olive),
        b(0.05, 0.035, 0.10, 0.0, -0.05, 0.415, olive),
        b(0.046, 0.03, 0.30, 0.0, 0.048, 0.28, polymer),
        b(0.052, 0.032, 0.05, 0.0, 0.0465, 0.445, black),
        b(0.05, 0.028, 0.03, 0.0, 0.048, 0.125, black),
        b(0.044, 0.085, 0.055, 0.0, -0.075, 0.29, olive),
        b(0.014, 0.035, 0.06, 0.0, -0.058, 0.24, olive),
        b(0.052, 0.06, 0.06, 0.0, -0.015, 0.05, olive),
        b(0.05, 0.062, 0.015, 0.0, -0.015, 0.0125, black),
        ring(0.007, 0.002, 0.0, -0.045, 0.035, black),
        b(0.032, 0.028, 0.025, 0.0, 0.065, 0.09, black),
        b(0.01, 0.018, 0.012, 0.0, 0.042, 0.465, black),
        ring(0.014, 0.003, 0.0, 0.052, 0.465, black),
        b(0.024, 0.018, 0.08, 0.0, -0.07, 0.39, black),
        s(0.007, 0.034, 0.02, 0.20, dark_steel),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "P90", length: 0.50 }
}

pub fn mp7() -> GunMesh {
    let steel = [0.45, 0.48, 0.52];
    let dark_steel = [0.22, 0.24, 0.27];
    let polymer = [0.13, 0.14, 0.16];
    let black = [0.08, 0.08, 0.10];

    let b = |w: f32, h: f32, d: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)), beveled_box(w, h, d, 0.006, 2), t)
    };
    let c = |r: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), cylinder(r, h, 16), t)
    };
    let f = |r0: f32, r1: f32, h: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), frustum(r0, r1, h, 16, true), t)
    };
    let s = |rad: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * Mat4::from_scale(glam::vec3(rad, rad, rad)), sphere(12, 8), t)
    };
    let ring = |rr: f32, tr: f32, x: f32, y: f32, z: f32, t: [f32; 3]| {
        (Mat4::from_translation(glam::vec3(x, y, z)) * rz(), torus_arc(rr, tr, 0.0, std::f32::consts::TAU, 24, 8), t)
    };

    let parts = vec![
        c(0.010, 0.10, 0.0, 0.0, 0.47, steel),
        f(0.015, 0.019, 0.04, 0.0, 0.0, 0.54, black),
        ring(0.02, 0.003, 0.0, 0.0, 0.53, black),
        b(0.06, 0.09, 0.20, 0.0, -0.008, 0.41, polymer),
        b(0.052, 0.048, 0.13, 0.0, -0.055, 0.42, polymer),
        b(0.034, 0.035, 0.05, 0.0, -0.085, 0.45, polymer),
        b(0.054, 0.06, 0.18, 0.0, 0.006, 0.24, steel),
        b(0.03, 0.014, 0.22, 0.0, 0.040, 0.31, polymer),
        b(0.028, 0.026, 0.02, 0.0, 0.052, 0.18, black),
        s(0.008, 0.035, 0.012, 0.26, dark_steel),
        b(0.044, 0.095, 0.05, 0.0, -0.078, 0.30, polymer),
        b(0.013, 0.035, 0.055, 0.0, -0.062, 0.265, polymer),
        b(0.042, 0.04, 0.06, 0.0, -0.045, 0.24, polymer),
        b(0.036, 0.115, 0.055, 0.0, -0.11, 0.24, polymer),
        b(0.04, 0.012, 0.06, 0.0, -0.177, 0.24, black),
        b(0.028, 0.038, 0.16, 0.0, -0.004, 0.07, dark_steel),
        b(0.045, 0.058, 0.05, 0.0, -0.004, -0.035, polymer),
        b(0.043, 0.056, 0.012, 0.0, -0.004, -0.066, black),
        ring(0.007, 0.002, 0.0, -0.05, -0.03, black),
    ];
    let (verts, indices) = assemble(&parts);
    GunMesh { verts, indices, display_name: "MP7", length: 0.63 }
}
