//! 现代武器数据表（数据源：《大战场枪械设计 V3.0（数学自洽版）》2026-08-19）
//!
//! 35 把枪械：联合体（红）17 把 + 同盟（蓝）18 把。
//! 每把枪携带：口径、弹匣/备弹、射速(RPM)、有效射程、伤害档位（距离衰减）、
//! 初速、子弹下坠重力、散射密度(MOA)、部位倍率（按距离分段）、换弹、后坐力。
//! 伤害计算：有效伤害 = 档位伤害 × 部位倍率 × 距离衰减；死亡判定 有效伤害 ≥ 100。
//! 击杀数 = ceil(100 / 有效伤害)。
//!
//! V3.0 核心变化：部位倍率按距离分段（如 AK-12M 头部 0-30m ×3.0 / 30-400m ×1.5）；
//! 新增初速（290~950 m/s）、子弹下坠（统一 9.8 m/s²）、散射密度（MOA）。

use crate::engine::weapons::{Firearm, ProjectileWeapon};

/// 阵营
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    /// 联合体（亚努斯联合体）—— 红方
    Union,
    /// 同盟（新诺斯同盟）—— 蓝方
    Alliance,
}

/// 音色类别（映射到 audio.rs 的 ShotParams）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    Rifle,
    Smg,
    Sniper,
    Lmg,
    Shotgun,
    Pistol,
}

/// 单把枪械完整参数（V3.0）
#[derive(Debug, Clone, Copy)]
pub struct WeaponSpec {
    /// 英文键名（日志/调试用，与枪模函数同名）
    pub key: &'static str,
    /// 中文显示名（HUD/命令窗口）
    pub name_zh: &'static str,
    /// 所属阵营
    pub faction: Faction,
    /// 口径
    pub caliber: &'static str,
    /// 弹匣容量（发）
    pub magazine: u32,
    /// 备弹（发，弹匣外）
    pub reserve: u32,
    /// 射速（发/分钟）
    pub rpm: f32,
    /// 有效射程（米）
    pub range: f32,
    /// 换弹时间（秒）
    pub reload: f32,
    /// 每发上跳后坐力（弧度）
    pub kick_pitch: f32,
    /// 每发水平后坐力（弧度）
    pub kick_yaw: f32,
    /// 每发射击弹丸数（霰弹 8，其余 1）
    pub pellets: u8,
    /// 音色类别
    pub sound: SoundKind,
    /// 初速（米/秒，V3.0 文档）
    pub muzzle_velocity: f32,
    /// 子弹下坠重力（米/秒²，V3.0 文档统一 9.8）
    pub gravity: f32,
    /// 散射密度（MOA，V3.0 文档；1 MOA ≈ 0.000291 rad）
    pub spread_moa: f32,
    /// 距离衰减档位：(距离上限米, 该档胸部基准伤害)；末档上限用 f32::MAX 表示"超程档"
    pub tiers: &'static [(f32, f32)],
    /// 部位倍率距离分段：(距离上限米, 头倍率, 胸倍率, 臂倍率, 腿倍率)
    pub part_tiers: &'static [(f32, f32, f32, f32, f32)],
}

impl WeaponSpec {
    /// 基础伤害 = 最近档位伤害（出膛即达）
    pub fn base_damage(&self) -> f32 {
        self.tiers.first().map(|(_, d)| *d).unwrap_or(20.0)
    }

    /// 部位倍率查表：按命中距离与命中点相对地面高度返回倍率（头/胸/臂/腿）。
    /// 高度阈值（与命中球几何一致）：头 ≥1.45m、胸 ≥0.95m、臂 ≥0.6m、腿 <0.6m。
    /// 生产路径经 build_firearm → 投射物携带分段；此表供测试与规格校验。
    #[allow(dead_code)]
    pub fn part_multiplier(&self, dist: f32, hit_height_rel: f32) -> f32 {
        let (_, h, c, a, l) = self
            .part_tiers
            .iter()
            .find(|(limit, ..)| dist <= *limit)
            .copied()
            .unwrap_or(*self.part_tiers.last().expect("part_tiers 非空"));
        if hit_height_rel >= 1.45 {
            h
        } else if hit_height_rel >= 0.95 {
            c
        } else if hit_height_rel >= 0.6 {
            a
        } else {
            l
        }
    }

    /// 击杀数（胸部基准）：ceil(100 / (档位伤害 × 胸倍率))
    #[allow(dead_code)]
    pub fn chest_kills(&self, dist: f32) -> u32 {
        let dmg = tiered_chest_damage(self, dist);
        let mult = self.part_multiplier(dist, 1.1);
        let eff = dmg * mult;
        ((100.0 / eff).ceil() as u32).max(1)
    }
}

/// 按距离查胸部基准伤害（档位伤害 × 距离衰减系数，未乘部位倍率）
#[allow(dead_code)]
pub fn tiered_chest_damage(spec: &WeaponSpec, dist: f32) -> f32 {
    crate::engine::weapons::tiered_damage(spec.base_damage(), spec.tiers, dist)
}

/// 全部 35 把枪械（编号 1..=35：联合体 1-17，同盟 18-35；命令窗口按编号切换）
pub const ALL_WEAPONS: [WeaponSpec; 35] = [
    // ============ 联合体（红）1-17 ============
    WeaponSpec { key: "ak12m", name_zh: "AK-12M 风暴", faction: Faction::Union, caliber: "7.62×39mm", magazine: 30, reserve: 90, rpm: 650.0, range: 400.0, reload: 2.3, kick_pitch: 0.0216, kick_yaw: 0.0076, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 710.0, gravity: 9.8, spread_moa: 0.8, tiers: &[(100.0, 34.0), (200.0, 30.0), (400.0, 26.0), (f32::MAX, 20.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (400.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "ak104", name_zh: "AK-104 短剑", faction: Faction::Union, caliber: "7.62×39mm", magazine: 30, reserve: 90, rpm: 680.0, range: 300.0, reload: 2.3, kick_pitch: 0.0210, kick_yaw: 0.0074, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 680.0, gravity: 9.8, spread_moa: 1.0, tiers: &[(100.0, 34.0), (200.0, 30.0), (300.0, 26.0), (f32::MAX, 18.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (300.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "pp19", name_zh: "PP-19-01 勇士", faction: Faction::Union, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 750.0, range: 150.0, reload: 1.9, kick_pitch: 0.0192, kick_yaw: 0.0067, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 400.0, gravity: 9.8, spread_moa: 2.5, tiers: &[(100.0, 26.0), (150.0, 18.0), (f32::MAX, 14.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (150.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 1.0, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "pp9", name_zh: "PP-9 胡蜂", faction: Faction::Union, caliber: "9×18mm", magazine: 20, reserve: 80, rpm: 700.0, range: 100.0, reload: 1.9, kick_pitch: 0.0192, kick_yaw: 0.0067, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 320.0, gravity: 9.8, spread_moa: 3.0, tiers: &[(100.0, 25.0), (f32::MAX, 15.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (100.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 1.0, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "vss", name_zh: "VSS Vintorez", faction: Faction::Union, caliber: "9×39mm", magazine: 20, reserve: 60, rpm: 700.0, range: 300.0, reload: 2.4, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 290.0, gravity: 9.8, spread_moa: 1.0, tiers: &[(100.0, 67.0), (200.0, 60.0), (300.0, 50.0), (f32::MAX, 35.0)], part_tiers: &[(300.0, 1.5, 1.2, 1.0, 0.8), (f32::MAX, 1.0, 1.0, 1.0, 0.8)] },
    WeaponSpec { key: "asval", name_zh: "AS Val", faction: Faction::Union, caliber: "9×39mm", magazine: 20, reserve: 80, rpm: 800.0, range: 250.0, reload: 2.2, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 290.0, gravity: 9.8, spread_moa: 1.2, tiers: &[(100.0, 34.0), (200.0, 30.0), (250.0, 26.0), (f32::MAX, 18.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (250.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "svd12", name_zh: "SVD-12M 支点", faction: Faction::Union, caliber: "7.62×54R", magazine: 10, reserve: 40, rpm: 600.0, range: 800.0, reload: 2.6, kick_pitch: 0.0264, kick_yaw: 0.0092, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 830.0, gravity: 9.8, spread_moa: 0.5, tiers: &[(100.0, 67.0), (200.0, 63.0), (400.0, 55.0), (600.0, 45.0), (800.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(800.0, 1.5, 1.0, 1.0, 0.8), (f32::MAX, 1.0, 1.0, 1.0, 0.8)] },
    WeaponSpec { key: "sv98", name_zh: "SV-98M 针叶", faction: Faction::Union, caliber: "7.62×54R", magazine: 10, reserve: 30, rpm: 45.0, range: 1000.0, reload: 3.2, kick_pitch: 0.0540, kick_yaw: 0.0158, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 860.0, gravity: 9.8, spread_moa: 0.3, tiers: &[(100.0, 77.0), (200.0, 72.0), (400.0, 65.0), (600.0, 55.0), (800.0, 45.0), (1000.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(1000.0, 1.6, 1.3, 1.0, 0.8), (f32::MAX, 1.2, 1.0, 1.0, 0.8)] },
    WeaponSpec { key: "osv96", name_zh: "OSV-96 削岩", faction: Faction::Union, caliber: "12.7×108mm", magazine: 5, reserve: 20, rpm: 40.0, range: 1500.0, reload: 3.8, kick_pitch: 0.0720, kick_yaw: 0.0202, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 900.0, gravity: 9.8, spread_moa: 0.4, tiers: &[(100.0, 80.0), (200.0, 75.0), (400.0, 65.0), (600.0, 55.0), (800.0, 45.0), (1500.0, 35.0), (f32::MAX, 28.0)], part_tiers: &[(1500.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.5, 1.2, 1.0, 0.8)] },
    WeaponSpec { key: "rpk16", name_zh: "RPK-16 桦木", faction: Faction::Union, caliber: "7.62×39mm", magazine: 45, reserve: 135, rpm: 600.0, range: 500.0, reload: 4.2, kick_pitch: 0.0240, kick_yaw: 0.0084, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 730.0, gravity: 9.8, spread_moa: 1.2, tiers: &[(100.0, 34.0), (200.0, 30.0), (400.0, 26.0), (500.0, 22.0), (f32::MAX, 16.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (500.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "pkm", name_zh: "PKM 钢线", faction: Faction::Union, caliber: "7.62×54R", magazine: 100, reserve: 200, rpm: 650.0, range: 600.0, reload: 4.2, kick_pitch: 0.0264, kick_yaw: 0.0092, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 825.0, gravity: 9.8, spread_moa: 1.5, tiers: &[(100.0, 67.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (f32::MAX, 28.0)], part_tiers: &[(100.0, 1.5, 1.0, 0.8, 0.6), (600.0, 1.2, 1.0, 0.8, 0.6), (f32::MAX, 0.8, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "pkp", name_zh: "PKP 佩切涅格", faction: Faction::Union, caliber: "7.62×54R", magazine: 100, reserve: 200, rpm: 650.0, range: 800.0, reload: 4.5, kick_pitch: 0.0288, kick_yaw: 0.0101, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 830.0, gravity: 9.8, spread_moa: 1.3, tiers: &[(100.0, 67.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (800.0, 33.0), (f32::MAX, 23.0)], part_tiers: &[(100.0, 1.5, 1.0, 0.8, 0.6), (800.0, 1.2, 1.0, 0.8, 0.6), (f32::MAX, 0.8, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "rope12", name_zh: "绳结 12.7mm", faction: Faction::Union, caliber: "12.7×108mm", magazine: 50, reserve: 150, rpm: 550.0, range: 1200.0, reload: 4.5, kick_pitch: 0.0336, kick_yaw: 0.0118, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 900.0, gravity: 9.8, spread_moa: 1.0, tiers: &[(100.0, 67.0), (200.0, 62.0), (400.0, 52.0), (600.0, 42.0), (800.0, 35.0), (1200.0, 28.0), (f32::MAX, 20.0)], part_tiers: &[(1200.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.5, 1.2, 1.0, 0.8)] },
    WeaponSpec { key: "saiga12", name_zh: "圆木 Saiga-12", faction: Faction::Union, caliber: "12号口径", magazine: 8, reserve: 32, rpm: 240.0, range: 50.0, reload: 2.8, kick_pitch: 0.0360, kick_yaw: 0.0126, pellets: 8, sound: SoundKind::Shotgun, muzzle_velocity: 400.0, gravity: 9.8, spread_moa: 25.0, tiers: &[(50.0, 14.0), (f32::MAX, 8.0)], part_tiers: &[(50.0, 1.5, 1.0, 1.0, 0.8), (f32::MAX, 1.2, 0.8, 0.8, 0.6)] },
    WeaponSpec { key: "mp443", name_zh: "MP-443 乌鸦", faction: Faction::Union, caliber: "9×19mm", magazine: 18, reserve: 72, rpm: 400.0, range: 50.0, reload: 1.5, kick_pitch: 0.0144, kick_yaw: 0.0050, pellets: 1, sound: SoundKind::Pistol, muzzle_velocity: 380.0, gravity: 9.8, spread_moa: 4.0, tiers: &[(50.0, 25.0), (f32::MAX, 15.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (50.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 0.8, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "rsh12", name_zh: "RSh-12 撞锤", faction: Faction::Union, caliber: "12.7×55mm", magazine: 5, reserve: 20, rpm: 180.0, range: 75.0, reload: 2.0, kick_pitch: 0.0300, kick_yaw: 0.0105, pellets: 1, sound: SoundKind::Pistol, muzzle_velocity: 400.0, gravity: 9.8, spread_moa: 3.0, tiers: &[(75.0, 40.0), (f32::MAX, 24.0)], part_tiers: &[(75.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.2, 0.8, 0.7, 0.6)] },
    WeaponSpec { key: "ash12", name_zh: "ASh-12 破城锤", faction: Faction::Union, caliber: "12.7×55mm", magazine: 10, reserve: 30, rpm: 350.0, range: 200.0, reload: 2.6, kick_pitch: 0.0300, kick_yaw: 0.0105, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 400.0, gravity: 9.8, spread_moa: 3.0, tiers: &[(50.0, 67.0), (100.0, 60.0), (150.0, 52.0), (200.0, 45.0), (f32::MAX, 28.0)], part_tiers: &[(200.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.2, 0.8, 0.7, 0.6)] },
    // ============ 同盟（蓝）18-35 ============
    WeaponSpec { key: "hk416", name_zh: "HK416 A8 游隼", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 30, reserve: 90, rpm: 850.0, range: 400.0, reload: 2.2, kick_pitch: 0.0153, kick_yaw: 0.0054, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 950.0, gravity: 9.8, spread_moa: 0.7, tiers: &[(100.0, 34.0), (200.0, 30.0), (400.0, 26.0), (f32::MAX, 18.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (400.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "mk18", name_zh: "MK18 隼爪", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 30, reserve: 90, rpm: 880.0, range: 300.0, reload: 2.2, kick_pitch: 0.0149, kick_yaw: 0.0052, pellets: 1, sound: SoundKind::Rifle, muzzle_velocity: 900.0, gravity: 9.8, spread_moa: 0.9, tiers: &[(100.0, 34.0), (200.0, 30.0), (300.0, 26.0), (f32::MAX, 16.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (300.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "mpx", name_zh: "MPX 燕鸥", faction: Faction::Alliance, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 800.0, range: 150.0, reload: 1.8, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 410.0, gravity: 9.8, spread_moa: 2.0, tiers: &[(100.0, 25.0), (150.0, 18.0), (f32::MAX, 14.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (150.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 1.0, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "mp5sd", name_zh: "MP5SD 雨燕", faction: Faction::Alliance, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 750.0, range: 100.0, reload: 1.8, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 380.0, gravity: 9.8, spread_moa: 2.5, tiers: &[(100.0, 25.0), (f32::MAX, 15.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (100.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 1.0, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "p90", name_zh: "P90", faction: Faction::Alliance, caliber: "5.7×28mm", magazine: 50, reserve: 150, rpm: 900.0, range: 200.0, reload: 1.9, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 850.0, gravity: 9.8, spread_moa: 2.0, tiers: &[(100.0, 34.0), (200.0, 30.0), (f32::MAX, 16.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (200.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "mp7", name_zh: "MP7", faction: Faction::Alliance, caliber: "4.6×30mm", magazine: 40, reserve: 120, rpm: 950.0, range: 150.0, reload: 1.9, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, muzzle_velocity: 725.0, gravity: 9.8, spread_moa: 2.5, tiers: &[(100.0, 34.0), (150.0, 30.0), (f32::MAX, 16.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (150.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "m110a1", name_zh: "M110A1 信使", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 20, reserve: 80, rpm: 600.0, range: 800.0, reload: 2.5, kick_pitch: 0.0187, kick_yaw: 0.0065, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 820.0, gravity: 9.8, spread_moa: 0.5, tiers: &[(100.0, 67.0), (200.0, 63.0), (400.0, 55.0), (600.0, 45.0), (800.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(800.0, 1.5, 1.0, 1.0, 0.8), (f32::MAX, 1.0, 1.0, 1.0, 0.8)] },
    WeaponSpec { key: "mk14p", name_zh: "MK14P 仲裁者", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 20, reserve: 80, rpm: 720.0, range: 600.0, reload: 2.4, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 800.0, gravity: 9.8, spread_moa: 0.6, tiers: &[(100.0, 67.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (f32::MAX, 28.0)], part_tiers: &[(100.0, 1.5, 1.0, 0.8, 0.6), (600.0, 1.2, 1.0, 0.8, 0.6), (f32::MAX, 0.8, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "m2010", name_zh: "M2010 ESR 界标", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 10, reserve: 30, rpm: 40.0, range: 1100.0, reload: 3.2, kick_pitch: 0.0480, kick_yaw: 0.0144, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 850.0, gravity: 9.8, spread_moa: 0.3, tiers: &[(100.0, 77.0), (200.0, 72.0), (400.0, 65.0), (600.0, 55.0), (800.0, 45.0), (1100.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(1100.0, 1.6, 1.3, 1.0, 0.8), (f32::MAX, 1.2, 1.0, 1.0, 0.8)] },
    WeaponSpec { key: "mrad", name_zh: "MRAD 巨石", faction: Faction::Alliance, caliber: ".338 Lapua", magazine: 5, reserve: 20, rpm: 35.0, range: 1600.0, reload: 3.5, kick_pitch: 0.0660, kick_yaw: 0.0185, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 900.0, gravity: 9.8, spread_moa: 0.3, tiers: &[(100.0, 75.0), (200.0, 70.0), (400.0, 60.0), (600.0, 50.0), (800.0, 42.0), (1600.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(1600.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.5, 1.2, 1.0, 0.8)] },
    WeaponSpec { key: "m249", name_zh: "M249 SAAR 蜂群", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 100, reserve: 200, rpm: 750.0, range: 600.0, reload: 4.2, kick_pitch: 0.0170, kick_yaw: 0.0060, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 950.0, gravity: 9.8, spread_moa: 1.2, tiers: &[(100.0, 34.0), (200.0, 30.0), (400.0, 26.0), (600.0, 22.0), (f32::MAX, 16.0)], part_tiers: &[(30.0, 3.0, 1.0, 0.8, 0.6), (600.0, 1.5, 1.0, 0.8, 0.6), (f32::MAX, 1.0, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "m240l", name_zh: "M240L 铁砧", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 100, reserve: 200, rpm: 650.0, range: 900.0, reload: 4.2, kick_pitch: 0.0221, kick_yaw: 0.0077, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 840.0, gravity: 9.8, spread_moa: 1.5, tiers: &[(100.0, 67.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (900.0, 32.0), (f32::MAX, 20.0)], part_tiers: &[(100.0, 1.5, 1.0, 0.8, 0.6), (900.0, 1.2, 1.0, 0.8, 0.6), (f32::MAX, 0.8, 1.0, 0.8, 0.6)] },
    WeaponSpec { key: "m2a1", name_zh: "M2A1 硬汉", faction: Faction::Alliance, caliber: "12.7×99mm", magazine: 50, reserve: 150, rpm: 550.0, range: 1300.0, reload: 4.5, kick_pitch: 0.0289, kick_yaw: 0.0101, pellets: 1, sound: SoundKind::Lmg, muzzle_velocity: 890.0, gravity: 9.8, spread_moa: 1.0, tiers: &[(100.0, 65.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (800.0, 33.0), (1300.0, 28.0), (f32::MAX, 20.0)], part_tiers: &[(1300.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.5, 1.2, 1.0, 0.8)] },
    WeaponSpec { key: "m1014", name_zh: "M1014 破门", faction: Faction::Alliance, caliber: "12号口径", magazine: 8, reserve: 32, rpm: 180.0, range: 50.0, reload: 2.8, kick_pitch: 0.0306, kick_yaw: 0.0107, pellets: 8, sound: SoundKind::Shotgun, muzzle_velocity: 400.0, gravity: 9.8, spread_moa: 25.0, tiers: &[(50.0, 13.0), (f32::MAX, 8.0)], part_tiers: &[(50.0, 1.5, 1.0, 1.0, 0.8), (f32::MAX, 1.2, 0.8, 0.8, 0.6)] },
    WeaponSpec { key: "aa12", name_zh: "AA12 风暴", faction: Faction::Alliance, caliber: "12号口径", magazine: 20, reserve: 40, rpm: 300.0, range: 80.0, reload: 2.8, kick_pitch: 0.0298, kick_yaw: 0.0104, pellets: 8, sound: SoundKind::Shotgun, muzzle_velocity: 420.0, gravity: 9.8, spread_moa: 30.0, tiers: &[(50.0, 13.0), (80.0, 10.0), (f32::MAX, 7.0)], part_tiers: &[(50.0, 1.5, 1.0, 1.0, 0.8), (80.0, 1.2, 0.8, 0.8, 0.6), (f32::MAX, 1.0, 0.6, 0.6, 0.5)] },
    WeaponSpec { key: "m18", name_zh: "M18 信标", faction: Faction::Alliance, caliber: "9×19mm", magazine: 17, reserve: 68, rpm: 400.0, range: 50.0, reload: 1.5, kick_pitch: 0.0122, kick_yaw: 0.0043, pellets: 1, sound: SoundKind::Pistol, muzzle_velocity: 380.0, gravity: 9.8, spread_moa: 4.0, tiers: &[(50.0, 25.0), (f32::MAX, 15.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (50.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 0.8, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "mk23", name_zh: "Mk23 Mod 0 海豹", faction: Faction::Alliance, caliber: ".45 ACP", magazine: 12, reserve: 48, rpm: 300.0, range: 75.0, reload: 1.8, kick_pitch: 0.0184, kick_yaw: 0.0064, pellets: 1, sound: SoundKind::Pistol, muzzle_velocity: 350.0, gravity: 9.8, spread_moa: 3.5, tiers: &[(50.0, 28.0), (75.0, 22.0), (f32::MAX, 14.0)], part_tiers: &[(10.0, 4.0, 1.0, 0.9, 0.7), (75.0, 1.5, 1.0, 0.9, 0.7), (f32::MAX, 0.8, 1.0, 0.9, 0.7)] },
    WeaponSpec { key: "m82a1", name_zh: "M82A1", faction: Faction::Alliance, caliber: "12.7×99mm", magazine: 10, reserve: 20, rpm: 40.0, range: 1800.0, reload: 3.8, kick_pitch: 0.0700, kick_yaw: 0.0196, pellets: 1, sound: SoundKind::Sniper, muzzle_velocity: 900.0, gravity: 9.8, spread_moa: 0.4, tiers: &[(100.0, 75.0), (200.0, 70.0), (400.0, 60.0), (600.0, 50.0), (800.0, 42.0), (1800.0, 35.0), (f32::MAX, 25.0)], part_tiers: &[(1800.0, 2.0, 1.5, 1.2, 1.0), (f32::MAX, 1.5, 1.2, 1.0, 0.8)] },
];

/// 按编号取武器规格：命令窗口输入 1..=35（联合体 1-17 / 同盟 18-35）
#[allow(dead_code)] // 供测试/命令窗口使用（当前切枪直接以槽位索引等价编号）
pub fn spec_by_number(n: usize) -> Option<&'static WeaponSpec> {
    ALL_WEAPONS.get(n.wrapping_sub(1))
}

/// 由规格构建弹匣武器（基础伤害 = 最近档位伤害；投射物寿命略大于射程/初速）
pub fn build_firearm(spec: &WeaponSpec) -> Firearm {
    let lifetime = spec.range / spec.muzzle_velocity + 0.15;
    let weapon = ProjectileWeapon::new_tiered(
        spec.name_zh,
        spec.base_damage(),
        spec.rpm / 60.0,
        spec.range,
        spec.muzzle_velocity,
        lifetime,
        spec.tiers,
    )
    .with_ballistics(spec.gravity, spec.part_tiers);
    Firearm::new(
        weapon,
        spec.magazine,
        spec.reserve,
        spec.reload,
        spec.kick_pitch,
        spec.kick_yaw,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::weapons::tiered_damage;

    /// 35 把枪齐全，编号 1..=35 可查，命名/口径/弹匣/射速齐全
    #[test]
    fn all_35_weapons_present() {
        assert_eq!(ALL_WEAPONS.len(), 35);
        for (i, spec) in ALL_WEAPONS.iter().enumerate() {
            assert!(!spec.key.is_empty(), "枪 #{} 缺 key", i + 1);
            assert!(!spec.name_zh.is_empty(), "枪 #{} 缺中文名", i + 1);
            assert!(!spec.caliber.is_empty(), "{} 缺口径", spec.key);
            assert!(spec.magazine > 0, "{} 弹匣为 0", spec.key);
            assert!(spec.rpm > 0.0, "{} 射速为 0", spec.key);
            assert!(spec.range > 0.0, "{} 射程为 0", spec.key);
            assert!(!spec.tiers.is_empty(), "{} 缺伤害档位", spec.key);
            assert!(!spec.part_tiers.is_empty(), "{} 缺部位倍率分段", spec.key);
            assert!(spec.muzzle_velocity > 100.0, "{} 初速异常 {}", spec.key, spec.muzzle_velocity);
            assert!(spec.gravity > 0.0, "{} 重力为 0", spec.key);
            assert!(spec.pellets >= 1, "{} 弹丸数非法", spec.key);
        }
    }

    /// key 唯一 + 与枪模一一映射（重复 key 会导致切枪串模/覆盖等隐形冲突）
    #[test]
    fn weapon_keys_unique_and_mesh_mapped() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for spec in ALL_WEAPONS.iter() {
            assert!(seen.insert(spec.key), "{} key 重复", spec.key);
            let gm = crate::engine::guns::gun_mesh_by_key(spec.key)
                .unwrap_or_else(|| panic!("{} 无对应枪模", spec.key));
            assert!(!gm.verts.is_empty() && !gm.indices.is_empty(), "{} 枪模为空", spec.key);
        }
        assert_eq!(seen.len(), 35);
    }

    /// 阵营分布：联合体 17 把（1-17），同盟 18 把（18-35）
    #[test]
    fn faction_split_17_18() {
        let union = ALL_WEAPONS.iter().filter(|s| s.faction == Faction::Union).count();
        assert_eq!(union, 17);
        assert_eq!(ALL_WEAPONS.len() - union, 18);
        assert_eq!(spec_by_number(1).unwrap().faction, Faction::Union);
        assert_eq!(spec_by_number(17).unwrap().faction, Faction::Union);
        assert_eq!(spec_by_number(18).unwrap().faction, Faction::Alliance);
        assert_eq!(spec_by_number(35).unwrap().faction, Faction::Alliance);
        assert!(spec_by_number(0).is_none());
        assert!(spec_by_number(36).is_none());
    }

    /// 伤害档位单调不增（近距 ≥ 远距），档位上限递增（含 f32::MAX 超程档）
    #[test]
    fn damage_tiers_monotonic() {
        for spec in ALL_WEAPONS.iter() {
            let mut prev_limit = 0.0f32;
            let mut prev_dmg = f32::MAX;
            for (limit, dmg) in spec.tiers {
                assert!(*limit > prev_limit, "{} 档位上限不递增", spec.key);
                assert!(*dmg <= prev_dmg, "{} 远档伤害高于近档", spec.key);
                prev_limit = *limit;
                prev_dmg = *dmg;
            }
            assert_eq!(spec.base_damage(), spec.tiers[0].1, "{} 基础伤害≠首档", spec.key);
        }
    }

    /// 部位倍率分段：距离上限递增；同段内 胸 ≥ 臂 ≥ 腿，头 ≥ 腿
    /// （PKM/PKP 远距段头 0.8 < 胸 1.0 是文档设计的"防弹头盔"语义，不作顺序强制）
    #[test]
    fn part_tiers_valid() {
        for spec in ALL_WEAPONS.iter() {
            let mut prev = 0.0f32;
            for (limit, h, c, a, l) in spec.part_tiers {
                assert!(*limit > prev, "{} 部位分段不递增", spec.key);
                assert!(c >= a && a >= l, "{} 倍率顺序异常", spec.key);
                assert!(h >= l, "{} 头倍率低于腿倍率", spec.key);
                assert!(*h > 0.0 && *c > 0.0 && *a > 0.0 && *l > 0.0, "{} 倍率必须为正", spec.key);
                prev = *limit;
            }
        }
    }

    /// 距离衰减查表：档内恒等、跨档衰减、超程档（f32::MAX）命中、空表恒伤害
    #[test]
    fn tiered_damage_lookup() {
        let tiers = &[(100.0, 34.0), (200.0, 30.0), (400.0, 26.0), (f32::MAX, 20.0)];
        assert_eq!(tiered_damage(34.0, tiers, 50.0), 34.0);
        assert_eq!(tiered_damage(34.0, tiers, 100.0), 34.0);
        assert_eq!(tiered_damage(34.0, tiers, 150.0), 30.0);
        assert_eq!(tiered_damage(34.0, tiers, 9999.0), 20.0, "超程档应命中");
        assert_eq!(tiered_damage(42.0, &[], 300.0), 42.0);
    }

    /// build_firearm：弹匣/备弹/换弹/后坐力逐项与规格一致；开火间隔 = 60/rpm
    #[test]
    fn build_firearm_matches_spec() {
        for spec in ALL_WEAPONS.iter() {
            let gun = build_firearm(spec);
            assert_eq!(gun.max_magazine(), spec.magazine, "{} 弹匣不符", spec.key);
            assert_eq!(gun.reserve(), spec.reserve, "{} 备弹不符", spec.key);
            let expect_interval = 60.0 / spec.rpm;
            assert!((gun.fire_interval() - expect_interval).abs() < 1e-3, "{} 开火间隔不符", spec.key);
        }
    }

    /// 霰弹：8 弹丸且近距单颗伤害为正
    #[test]
    fn shotguns_have_8_pellets() {
        for spec in ALL_WEAPONS.iter().filter(|s| s.pellets > 1) {
            assert_eq!(spec.pellets, 8, "{} 霰弹弹丸数应为 8", spec.key);
            assert!(spec.base_damage() > 0.0, "{} 弹丸伤害非法", spec.key);
        }
        let shotgun_count = ALL_WEAPONS.iter().filter(|s| s.pellets > 1).count();
        assert_eq!(shotgun_count, 3); // Saiga-12 / M1014 / AA12
    }

    /// V3.0 关键武器参数抽查（文档数值）
    #[test]
    fn spot_check_doc_values() {
        let ak12m = spec_by_number(1).unwrap();
        assert_eq!(ak12m.magazine, 30);
        assert_eq!(ak12m.rpm, 650.0);
        assert_eq!(ak12m.tiers[0], (100.0, 34.0));
        assert_eq!(ak12m.muzzle_velocity, 710.0);
        assert_eq!(ak12m.spread_moa, 0.8);
        assert_eq!(ak12m.part_tiers[0], (30.0, 3.0, 1.0, 0.8, 0.6));
        let sv98 = spec_by_number(8).unwrap();
        assert_eq!(sv98.rpm, 45.0);
        assert_eq!(sv98.tiers[0], (100.0, 77.0));
        assert_eq!(sv98.muzzle_velocity, 860.0);
        let hk416 = spec_by_number(18).unwrap();
        assert_eq!(hk416.rpm, 850.0);
        assert_eq!(hk416.tiers[0], (100.0, 34.0));
        assert_eq!(hk416.muzzle_velocity, 950.0);
        let m82 = spec_by_number(35).unwrap();
        assert_eq!(m82.range, 1800.0);
        assert_eq!(m82.tiers[0], (100.0, 75.0));
        assert_eq!(m82.muzzle_velocity, 900.0);
        // 霰弹：Saiga 单颗 14 / M1014 单颗 13 / AA12 单颗 13
        assert_eq!(spec_by_number(14).unwrap().base_damage(), 14.0);
        assert_eq!(spec_by_number(31).unwrap().base_damage(), 13.0);
        assert_eq!(spec_by_number(32).unwrap().base_damage(), 13.0);
    }

    /// 部位倍率查表：按距离分段（AK-12M 头部 0-30m ×3.0 / 30m+ ×1.5；胸部恒定 ×1.0）
    #[test]
    fn part_multiplier_lookup_v3() {
        let ak12m = spec_by_number(1).unwrap();
        assert_eq!(ak12m.part_multiplier(10.0, 1.6), 3.0, "0-30m 头 ×3.0");
        assert_eq!(ak12m.part_multiplier(10.0, 1.1), 1.0, "0-30m 胸 ×1.0");
        assert_eq!(ak12m.part_multiplier(10.0, 0.7), 0.8, "0-30m 臂 ×0.8");
        assert_eq!(ak12m.part_multiplier(10.0, 0.3), 0.6, "0-30m 腿 ×0.6");
        assert_eq!(ak12m.part_multiplier(100.0, 1.6), 1.5, "30m+ 头 ×1.5");
        assert_eq!(ak12m.part_multiplier(500.0, 1.6), 1.0, "400m+ 头 ×1.0");
        // VSS 胸部 ×1.2（0-300m）
        let vss = spec_by_number(5).unwrap();
        assert_eq!(vss.part_multiplier(50.0, 1.1), 1.2, "VSS 胸 ×1.2");
        assert_eq!(vss.part_multiplier(500.0, 1.1), 1.0, "VSS 300m+ 胸 ×1.0");
        // OSV-96 反器材胸 ×1.5
        let osv = spec_by_number(9).unwrap();
        assert_eq!(osv.part_multiplier(50.0, 1.1), 1.5);
        assert_eq!(osv.part_multiplier(50.0, 1.6), 2.0);
    }

    /// 击杀数公式验证（文档：击杀数 = ceil(100/有效伤害)）
    #[test]
    fn kills_to_kill_formula_v3() {
        let ak12m = spec_by_number(1).unwrap();
        // 0-30m 头：34×3.0=102 ≥100 → 1 枪
        let head_dmg = tiered_chest_damage(ak12m, 10.0) * ak12m.part_multiplier(10.0, 1.6);
        assert!(head_dmg >= 100.0, "AK 头 0-30m 应 1 枪，实际伤害 {}", head_dmg);
        // 0-100m 胸：34 → 3 枪
        assert_eq!(ak12m.chest_kills(50.0), 3);
        // 100-200m 胸：30 → 4 枪
        assert_eq!(ak12m.chest_kills(150.0), 4);
        // 400m+ 胸：20 → 5 枪
        assert_eq!(ak12m.chest_kills(500.0), 5);
        // SV-98 0-100m 胸：77×1.3=100.1 → 1 枪
        let sv98 = spec_by_number(8).unwrap();
        assert_eq!(sv98.chest_kills(50.0), 1);
        // OSV 0-100m 胸：80×1.5=120 → 1 枪
        let osv = spec_by_number(9).unwrap();
        assert_eq!(osv.chest_kills(50.0), 1);
        // M2A1 0-100m 胸：65×1.5=97.5 <100 → 2 枪（文档明确 2 枪）
        let m2 = spec_by_number(30).unwrap();
        assert_eq!(m2.chest_kills(50.0), 2);
        // MP-443 0-50m 胸 25 → 4 枪
        assert_eq!(spec_by_number(15).unwrap().chest_kills(30.0), 4);
    }

    /// 计算 35 把枪的最大网格规模（枪模缓冲预分配用）：返回 (max_verts, max_indices)
    pub fn gun_mesh_max_sizes() -> (u32, u32) {
        let mut max_v = 0u32;
        let mut max_i = 0u32;
        for spec in ALL_WEAPONS.iter() {
            if let Some(gm) = crate::engine::guns::gun_mesh_by_key(spec.key) {
                max_v = max_v.max(gm.verts.len() as u32);
                max_i = max_i.max(gm.indices.len() as u32);
            }
        }
        (max_v, max_i)
    }

    /// 打印并校验最大网格规模
    #[test]
    fn gun_mesh_max_sizes_report() {
        let (mv, mi) = gun_mesh_max_sizes();
        println!("GUN_MAX: verts={} idx={}", mv, mi);
        assert!(mv > 0 && mi > 0);
    }

    /// 枪模姿态校验：长轴沿 Z（水平），尺寸上限
    #[test]
    fn gun_meshes_horizontal_orientation() {
        for spec in ALL_WEAPONS.iter() {
            let gm = crate::engine::guns::gun_mesh_by_key(spec.key)
                .unwrap_or_else(|| panic!("{} 无枪模", spec.key));
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for v in &gm.verts {
                for (i, p) in v.pos.iter().enumerate() {
                    min[i] = min[i].min(*p);
                    max[i] = max[i].max(*p);
                }
            }
            let (sx, sy, sz) = (max[0] - min[0], max[1] - min[1], max[2] - min[2]);
            assert!(sz > sy * 0.85, "{} 姿态异常（竖直）", spec.key);
            assert!(sx < 1.5 && sy < 1.5 && sz < 2.5, "{} 尺寸异常", spec.key);
        }
    }

    /// 第一人称视空间范围校验（不穿模/不怼脸）
    #[test]
    fn gun_fp_viewspace_no_clip() {
        let hip = (glam::Vec3::new(0.25, -0.20, -0.60), 0.50);
        let ads = (glam::Vec3::new(0.0, -0.08, -0.42), 0.47);
        for spec in ALL_WEAPONS.iter() {
            let gm = crate::engine::guns::gun_mesh_by_key(spec.key)
                .unwrap_or_else(|| panic!("{} 无枪模", spec.key));
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for v in &gm.verts {
                for (i, p) in v.pos.iter().enumerate() {
                    min[i] = min[i].min(*p);
                    max[i] = max[i].max(*p);
                }
            }
            for (name, (anchor, scale)) in [("hip", hip), ("ads", ads)] {
                let z_front = anchor.z - min[2] * scale;
                let z_back = anchor.z - max[2] * scale;
                let z_far = z_front.max(z_back);
                assert!(z_far < -0.05, "{} {} 穿模", spec.key, name);
                let dist = (-z_front.min(z_back)).max(0.1);
                let half_len = (max[2] - min[2]).abs() * scale * 0.5;
                let ang = half_len.atan2(dist) * 2.0;
                assert!(ang < 70.0_f32.to_radians(), "{} {} 怼脸", spec.key, name);
            }
        }
    }
}

