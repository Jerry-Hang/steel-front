//! 现代武器数据表（数据源：《大战场枪械设计 v1.0》全文参数）
//!
//! 35 把枪械：联合体（红）17 把 + 同盟（蓝）18 把。
//! 每把枪携带：口径、弹匣/备弹、射速(RPM)、有效射程、伤害档位（距离衰减）、
//! 换弹时间、后坐力、投射物速度、霰弹弹丸数、音色类别。
//! 伤害计算：有效伤害 = 档位伤害 × 部位倍率（头1.5/胸1.0/臂0.8/腿0.6）。
//!
//! 阵营手感（设计文档）：联合体单发伤害高 15~20%、后坐力大、射速中等；
//! 同盟单发低、后坐力小、射速快、操控好 —— 后坐力系数红 ×1.2 / 蓝 ×0.85。

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

/// 单把枪械完整参数
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
    /// 投射物速度（米/秒）
    pub speed: f32,
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
    /// 距离衰减档位：(距离上限米, 该档胸部伤害)
    pub tiers: &'static [(f32, f32)],
}

impl WeaponSpec {
    /// 基础伤害 = 最近档位伤害（出膛即达）
    pub fn base_damage(&self) -> f32 {
        self.tiers.first().map(|(_, d)| *d).unwrap_or(20.0)
    }
}

/// 全部 35 把枪械（编号 1..=35：联合体 1-17，同盟 18-35；命令窗口按编号切换）
pub const ALL_WEAPONS: [WeaponSpec; 35] = [
    // ============ 联合体（红）1-17 ============
    WeaponSpec { key: "ak12m", name_zh: "AK-12M 风暴", faction: Faction::Union, caliber: "7.62×39mm", magazine: 30, reserve: 90, rpm: 650.0, range: 400.0, speed: 75.0, reload: 2.3, kick_pitch: 0.0216, kick_yaw: 0.0076, pellets: 1, sound: SoundKind::Rifle, tiers: &[(100.0, 28.0), (200.0, 25.0), (400.0, 20.0), (600.0, 15.0)] },
    WeaponSpec { key: "ak104", name_zh: "AK-104 短剑", faction: Faction::Union, caliber: "7.62×39mm", magazine: 30, reserve: 90, rpm: 680.0, range: 300.0, speed: 75.0, reload: 2.3, kick_pitch: 0.0210, kick_yaw: 0.0074, pellets: 1, sound: SoundKind::Rifle, tiers: &[(100.0, 25.0), (200.0, 22.0), (300.0, 17.0)] },
    WeaponSpec { key: "pp19", name_zh: "PP-19-01 勇士", faction: Faction::Union, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 750.0, range: 150.0, speed: 55.0, reload: 1.9, kick_pitch: 0.0192, kick_yaw: 0.0067, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 20.0), (200.0, 14.0)] },
    WeaponSpec { key: "pp9", name_zh: "PP-9 胡蜂", faction: Faction::Union, caliber: "9×18mm", magazine: 20, reserve: 80, rpm: 700.0, range: 100.0, speed: 55.0, reload: 1.9, kick_pitch: 0.0192, kick_yaw: 0.0067, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 18.0)] },
    WeaponSpec { key: "vss", name_zh: "VSS Vintorez", faction: Faction::Union, caliber: "9×39mm", magazine: 20, reserve: 60, rpm: 700.0, range: 300.0, speed: 70.0, reload: 2.4, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 45.0), (200.0, 40.0), (300.0, 32.0)] },
    WeaponSpec { key: "asval", name_zh: "AS Val", faction: Faction::Union, caliber: "9×39mm", magazine: 20, reserve: 80, rpm: 800.0, range: 250.0, speed: 70.0, reload: 2.2, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Rifle, tiers: &[(100.0, 40.0), (200.0, 36.0), (250.0, 28.0)] },
    WeaponSpec { key: "svd12", name_zh: "SVD-12M 支点", faction: Faction::Union, caliber: "7.62×54R", magazine: 10, reserve: 40, rpm: 600.0, range: 800.0, speed: 85.0, reload: 2.6, kick_pitch: 0.0264, kick_yaw: 0.0092, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 70.0), (200.0, 65.0), (400.0, 55.0), (600.0, 45.0), (800.0, 35.0)] },
    WeaponSpec { key: "sv98", name_zh: "SV-98M 针叶", faction: Faction::Union, caliber: "7.62×54R", magazine: 10, reserve: 30, rpm: 45.0, range: 1000.0, speed: 90.0, reload: 3.2, kick_pitch: 0.0540, kick_yaw: 0.0158, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 85.0), (200.0, 80.0), (400.0, 70.0), (600.0, 60.0), (800.0, 50.0), (1000.0, 40.0)] },
    WeaponSpec { key: "osv96", name_zh: "OSV-96 削岩", faction: Faction::Union, caliber: "12.7×108mm", magazine: 5, reserve: 20, rpm: 40.0, range: 1500.0, speed: 100.0, reload: 3.8, kick_pitch: 0.0720, kick_yaw: 0.0202, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 120.0), (200.0, 110.0), (400.0, 95.0), (600.0, 80.0), (800.0, 65.0), (1500.0, 50.0)] },
    WeaponSpec { key: "rpk16", name_zh: "RPK-16 桦木", faction: Faction::Union, caliber: "7.62×39mm", magazine: 45, reserve: 135, rpm: 600.0, range: 500.0, speed: 65.0, reload: 4.2, kick_pitch: 0.0240, kick_yaw: 0.0084, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 27.0), (200.0, 24.0), (400.0, 19.0), (500.0, 14.0)] },
    WeaponSpec { key: "pkm", name_zh: "PKM 钢线", faction: Faction::Union, caliber: "7.62×54R", magazine: 100, reserve: 200, rpm: 650.0, range: 600.0, speed: 65.0, reload: 4.2, kick_pitch: 0.0264, kick_yaw: 0.0092, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 33.0), (200.0, 30.0), (400.0, 27.0), (600.0, 23.0)] },
    WeaponSpec { key: "pkp", name_zh: "PKP 佩切涅格", faction: Faction::Union, caliber: "7.62×54R", magazine: 100, reserve: 200, rpm: 650.0, range: 800.0, speed: 65.0, reload: 4.5, kick_pitch: 0.0288, kick_yaw: 0.0101, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 50.0), (200.0, 46.0), (400.0, 40.0), (600.0, 33.0), (800.0, 26.0)] },
    WeaponSpec { key: "rope12", name_zh: "绳结 12.7mm", faction: Faction::Union, caliber: "12.7×108mm", magazine: 50, reserve: 150, rpm: 550.0, range: 1200.0, speed: 75.0, reload: 4.5, kick_pitch: 0.0336, kick_yaw: 0.0118, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 80.0), (200.0, 74.0), (400.0, 65.0), (600.0, 55.0), (800.0, 45.0), (1200.0, 35.0)] },
    WeaponSpec { key: "saiga12", name_zh: "圆木 Saiga-12", faction: Faction::Union, caliber: "12号口径", magazine: 8, reserve: 32, rpm: 240.0, range: 50.0, speed: 40.0, reload: 2.8, kick_pitch: 0.0360, kick_yaw: 0.0126, pellets: 8, sound: SoundKind::Shotgun, tiers: &[(50.0, 25.0)] },
    WeaponSpec { key: "mp443", name_zh: "MP-443 乌鸦", faction: Faction::Union, caliber: "9×19mm", magazine: 18, reserve: 72, rpm: 400.0, range: 50.0, speed: 45.0, reload: 1.5, kick_pitch: 0.0144, kick_yaw: 0.0050, pellets: 1, sound: SoundKind::Pistol, tiers: &[(50.0, 25.0), (100.0, 15.0)] },
    WeaponSpec { key: "rsh12", name_zh: "RSh-12 撞锤", faction: Faction::Union, caliber: "12.7×55mm", magazine: 5, reserve: 20, rpm: 180.0, range: 75.0, speed: 45.0, reload: 2.0, kick_pitch: 0.0300, kick_yaw: 0.0105, pellets: 1, sound: SoundKind::Pistol, tiers: &[(50.0, 50.0), (75.0, 35.0)] },
    WeaponSpec { key: "ash12", name_zh: "ASh-12 破城锤", faction: Faction::Union, caliber: "12.7×55mm", magazine: 10, reserve: 30, rpm: 350.0, range: 200.0, speed: 70.0, reload: 2.6, kick_pitch: 0.0300, kick_yaw: 0.0105, pellets: 1, sound: SoundKind::Rifle, tiers: &[(50.0, 120.0), (100.0, 100.0), (150.0, 80.0), (200.0, 60.0)] },
    // ============ 同盟（蓝）18-35 ============
    WeaponSpec { key: "hk416", name_zh: "HK416 A8 游隼", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 30, reserve: 90, rpm: 850.0, range: 400.0, speed: 75.0, reload: 2.2, kick_pitch: 0.0153, kick_yaw: 0.0054, pellets: 1, sound: SoundKind::Rifle, tiers: &[(100.0, 22.0), (200.0, 19.0), (400.0, 15.0), (600.0, 11.0)] },
    WeaponSpec { key: "mk18", name_zh: "MK18 隼爪", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 30, reserve: 90, rpm: 880.0, range: 300.0, speed: 75.0, reload: 2.2, kick_pitch: 0.0149, kick_yaw: 0.0052, pellets: 1, sound: SoundKind::Rifle, tiers: &[(100.0, 20.0), (200.0, 17.0), (300.0, 13.0)] },
    WeaponSpec { key: "mpx", name_zh: "MPX 燕鸥", faction: Faction::Alliance, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 800.0, range: 150.0, speed: 55.0, reload: 1.8, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 18.0), (200.0, 12.0)] },
    WeaponSpec { key: "mp5sd", name_zh: "MP5SD 雨燕", faction: Faction::Alliance, caliber: "9×19mm", magazine: 30, reserve: 120, rpm: 750.0, range: 100.0, speed: 55.0, reload: 1.8, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 16.0)] },
    WeaponSpec { key: "p90", name_zh: "P90", faction: Faction::Alliance, caliber: "5.7×28mm", magazine: 50, reserve: 150, rpm: 900.0, range: 200.0, speed: 60.0, reload: 1.9, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 18.0), (200.0, 15.0)] },
    WeaponSpec { key: "mp7", name_zh: "MP7", faction: Faction::Alliance, caliber: "4.6×30mm", magazine: 40, reserve: 120, rpm: 950.0, range: 150.0, speed: 60.0, reload: 1.9, kick_pitch: 0.0136, kick_yaw: 0.0048, pellets: 1, sound: SoundKind::Smg, tiers: &[(100.0, 20.0), (150.0, 16.0)] },
    WeaponSpec { key: "m110a1", name_zh: "M110A1 信使", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 20, reserve: 80, rpm: 600.0, range: 800.0, speed: 85.0, reload: 2.5, kick_pitch: 0.0187, kick_yaw: 0.0065, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 65.0), (200.0, 60.0), (400.0, 50.0), (600.0, 40.0), (800.0, 30.0)] },
    WeaponSpec { key: "mk14p", name_zh: "MK14P 仲裁者", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 20, reserve: 80, rpm: 720.0, range: 600.0, speed: 80.0, reload: 2.4, kick_pitch: 0.0204, kick_yaw: 0.0071, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 34.0), (200.0, 30.0), (400.0, 25.0), (600.0, 20.0)] },
    WeaponSpec { key: "m2010", name_zh: "M2010 ESR 界标", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 10, reserve: 30, rpm: 40.0, range: 1100.0, speed: 90.0, reload: 3.2, kick_pitch: 0.0480, kick_yaw: 0.0144, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 80.0), (200.0, 75.0), (400.0, 65.0), (600.0, 55.0), (800.0, 45.0), (1100.0, 35.0)] },
    WeaponSpec { key: "mrad", name_zh: "MRAD 巨石", faction: Faction::Alliance, caliber: ".338 Lapua", magazine: 5, reserve: 20, rpm: 35.0, range: 1600.0, speed: 100.0, reload: 3.5, kick_pitch: 0.0660, kick_yaw: 0.0185, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 110.0), (200.0, 105.0), (400.0, 95.0), (600.0, 85.0), (800.0, 75.0), (1600.0, 60.0)] },
    WeaponSpec { key: "m249", name_zh: "M249 SAAR 蜂群", faction: Faction::Alliance, caliber: "5.56×45mm", magazine: 100, reserve: 200, rpm: 750.0, range: 600.0, speed: 65.0, reload: 4.2, kick_pitch: 0.0170, kick_yaw: 0.0060, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 20.0), (200.0, 17.0), (400.0, 13.0), (600.0, 9.0)] },
    WeaponSpec { key: "m240l", name_zh: "M240L 铁砧", faction: Faction::Alliance, caliber: "7.62×51mm", magazine: 100, reserve: 200, rpm: 650.0, range: 900.0, speed: 65.0, reload: 4.2, kick_pitch: 0.0221, kick_yaw: 0.0077, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 45.0), (200.0, 41.0), (400.0, 35.0), (600.0, 28.0), (900.0, 20.0)] },
    WeaponSpec { key: "m2a1", name_zh: "M2A1 硬汉", faction: Faction::Alliance, caliber: "12.7×99mm", magazine: 50, reserve: 150, rpm: 550.0, range: 1300.0, speed: 75.0, reload: 4.5, kick_pitch: 0.0289, kick_yaw: 0.0101, pellets: 1, sound: SoundKind::Lmg, tiers: &[(100.0, 75.0), (200.0, 70.0), (400.0, 60.0), (600.0, 50.0), (800.0, 40.0), (1300.0, 30.0)] },
    WeaponSpec { key: "m1014", name_zh: "M1014 破门", faction: Faction::Alliance, caliber: "12号口径", magazine: 8, reserve: 32, rpm: 180.0, range: 50.0, speed: 40.0, reload: 2.8, kick_pitch: 0.0306, kick_yaw: 0.0107, pellets: 8, sound: SoundKind::Shotgun, tiers: &[(50.0, 22.0)] },
    WeaponSpec { key: "aa12", name_zh: "AA12 风暴", faction: Faction::Alliance, caliber: "12号口径", magazine: 20, reserve: 40, rpm: 300.0, range: 80.0, speed: 40.0, reload: 2.8, kick_pitch: 0.0298, kick_yaw: 0.0104, pellets: 8, sound: SoundKind::Shotgun, tiers: &[(100.0, 18.0), (200.0, 12.0)] },
    WeaponSpec { key: "m18", name_zh: "M18 信标", faction: Faction::Alliance, caliber: "9×19mm", magazine: 17, reserve: 68, rpm: 400.0, range: 50.0, speed: 45.0, reload: 1.5, kick_pitch: 0.0122, kick_yaw: 0.0043, pellets: 1, sound: SoundKind::Pistol, tiers: &[(50.0, 22.0)] },
    WeaponSpec { key: "mk23", name_zh: "Mk23 Mod 0 海豹", faction: Faction::Alliance, caliber: ".45 ACP", magazine: 12, reserve: 48, rpm: 300.0, range: 75.0, speed: 45.0, reload: 1.8, kick_pitch: 0.0184, kick_yaw: 0.0064, pellets: 1, sound: SoundKind::Pistol, tiers: &[(50.0, 35.0), (75.0, 25.0)] },
    WeaponSpec { key: "m82a1", name_zh: "M82A1", faction: Faction::Alliance, caliber: "12.7×99mm", magazine: 10, reserve: 20, rpm: 40.0, range: 1800.0, speed: 100.0, reload: 3.8, kick_pitch: 0.0700, kick_yaw: 0.0196, pellets: 1, sound: SoundKind::Sniper, tiers: &[(100.0, 115.0), (200.0, 108.0), (400.0, 95.0), (600.0, 80.0), (800.0, 65.0), (1800.0, 50.0)] },
];

/// 按编号取武器规格：命令窗口输入 1..=35（联合体 1-17 / 同盟 18-35）
#[allow(dead_code)] // 供测试/命令窗口使用（当前切枪直接以槽位索引等价编号）
pub fn spec_by_number(n: usize) -> Option<&'static WeaponSpec> {
    ALL_WEAPONS.get(n.wrapping_sub(1))
}

/// 由规格构建弹匣武器（基础伤害 = 最近档位伤害；投射物寿命略大于射程/速度）
pub fn build_firearm(spec: &WeaponSpec) -> Firearm {
    let lifetime = spec.range / spec.speed + 0.15;
    let weapon = ProjectileWeapon::new_tiered(
        spec.name_zh,
        spec.base_damage(),
        spec.rpm / 60.0,
        spec.range,
        spec.speed,
        lifetime,
        spec.tiers,
    );
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
            assert!(spec.pellets >= 1, "{} 弹丸数非法", spec.key);
        }
    }

    /// 阵营分布：联合体 17 把（1-17），同盟 18 把（18-35）
    #[test]
    fn faction_split_17_18() {
        let union = ALL_WEAPONS
            .iter()
            .filter(|s| s.faction == Faction::Union)
            .count();
        assert_eq!(union, 17);
        assert_eq!(ALL_WEAPONS.len() - union, 18);
        // 编号映射：1-17 联合体，18-35 同盟
        assert_eq!(spec_by_number(1).unwrap().faction, Faction::Union);
        assert_eq!(spec_by_number(17).unwrap().faction, Faction::Union);
        assert_eq!(spec_by_number(18).unwrap().faction, Faction::Alliance);
        assert_eq!(spec_by_number(35).unwrap().faction, Faction::Alliance);
        assert!(spec_by_number(0).is_none());
        assert!(spec_by_number(36).is_none());
    }

    /// 伤害档位单调不增（近距 ≥ 远距），档位上限递增
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

    /// 距离衰减查表：档内恒等、跨档衰减、超程 60% 兜底、空表恒伤害
    #[test]
    fn tiered_damage_lookup() {
        let tiers = &[(100.0, 28.0), (200.0, 25.0), (400.0, 20.0), (600.0, 15.0)];
        assert_eq!(tiered_damage(28.0, tiers, 50.0), 28.0);
        assert_eq!(tiered_damage(28.0, tiers, 100.0), 28.0);
        assert_eq!(tiered_damage(28.0, tiers, 150.0), 25.0);
        assert_eq!(tiered_damage(28.0, tiers, 600.0), 15.0);
        assert!((tiered_damage(28.0, tiers, 9999.0) - 9.0).abs() < 1e-4, "超程应 15×0.6=9");
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
            assert!(
                (gun.fire_interval() - expect_interval).abs() < 1e-3,
                "{} 开火间隔不符",
                spec.key
            );
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

    /// 关键武器参数抽查（设计文档数值）
    #[test]
    fn spot_check_doc_values() {
        let ak12m = spec_by_number(1).unwrap();
        assert_eq!(ak12m.magazine, 30);
        assert_eq!(ak12m.rpm, 650.0);
        assert_eq!(ak12m.tiers[0], (100.0, 28.0));
        let sv98 = spec_by_number(8).unwrap();
        assert_eq!(sv98.magazine, 10);
        assert_eq!(sv98.rpm, 45.0);
        assert_eq!(sv98.tiers[0], (100.0, 85.0));
        let hk416 = spec_by_number(18).unwrap();
        assert_eq!(hk416.rpm, 850.0);
        assert_eq!(hk416.tiers[0], (100.0, 22.0));
        let m82 = spec_by_number(35).unwrap();
        assert_eq!(m82.range, 1800.0);
        assert_eq!(m82.tiers[0], (100.0, 115.0));
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

    /// 枪模姿态校验：枪身应沿 +Z 水平（枪口朝前），不得竖直建模（Y 向过长）。
    /// 第一人称渲染依赖该姿态（局部 y-up 枪口 +Z，翻转后水平指向屏幕外）。
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
            println!(
                "GUN_ORIENT {}: sz={:.2} sy={:.2} sx={:.2} {}",
                spec.key,
                sz,
                sy,
                sx,
                if sz > sy { "OK" } else { "VERTICAL!" }
            );
        }
        // 断言：任何枪不得竖直
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
            // 姿态：长轴沿 Z（允许短武器 ±15%）；尺寸上限防"单位球未缩放"类 bug
            assert!(
                sz > sy * 0.85,
                "{} 姿态异常：枪身沿 Y 竖直（sz={:.2} sy={:.2}）",
                spec.key,
                sz,
                sy
            );
            assert!(
                sx < 1.5 && sy < 1.5 && sz < 2.5,
                "{} 尺寸异常（疑似未缩放图元）：sx={:.2} sy={:.2} sz={:.2}",
                spec.key,
                sx,
                sy,
                sz
            );
        }
    }

    /// 第一人称视空间范围校验（地基验证）：枪模在腰射/开镜姿态下
    /// 视空间 z 必须 < 0（不穿相机）且屏幕张角 < FOV（不怼脸）。
    /// 姿态参数与 main.rs first_person_gun_mesh 保持一致。
    #[test]
    fn gun_fp_viewspace_no_clip() {
        // 姿态（与 main.rs 同步）：(锚点, 缩放)
        let hip = (glam::Vec3::new(0.25, -0.20, -0.60), 0.50);
        let ads = (glam::Vec3::new(0.0, -0.08, -0.42), 0.47);
        for spec in ALL_WEAPONS.iter() {
            let gm = crate::engine::guns::gun_mesh_by_key(spec.key)
                .unwrap_or_else(|| panic!("{} 无枪模", spec.key));
            // 局部包围盒
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for v in &gm.verts {
                for (i, p) in v.pos.iter().enumerate() {
                    min[i] = min[i].min(*p);
                    max[i] = max[i].max(*p);
                }
            }
            for (name, (anchor, scale)) in [("hip", hip), ("ads", ads)] {
                // 视空间 z = anchor.z - 局部 z * scale（Ry(π) 翻转使 +Z 朝后）
                let z_front = anchor.z - min[2] * scale; // 枪口端（局部 +Z → 视空间更负）
                let z_back = anchor.z - max[2] * scale; // 枪尾端（局部 -Z → 视空间更正）
                let z_near = z_front.min(z_back); // 最远（负方向）
                let z_far = z_front.max(z_back); // 最近（穿模风险端）
                assert!(
                    z_far < -0.05,
                    "{} {} 穿模：枪尾 z={:.2} 越过相机平面",
                    spec.key,
                    name,
                    z_far
                );
                // 张角：枪长在最近距离下的半角
                let dist = (-z_near).max(0.1);
                let half_len = (max[2] - min[2]).abs() * scale * 0.5;
                let ang = half_len.atan2(dist) * 2.0;
                assert!(
                    ang < 70.0_f32.to_radians(),
                    "{} {} 怼脸：枪张角 {:.0}° > FOV 70°",
                    spec.key,
                    name,
                    ang.to_degrees()
                );
            }
        }
    }

    /// 打印并校验最大网格规模（枪模缓冲预分配容量依据）
    #[test]
    fn gun_mesh_max_sizes_report() {
        let (mv, mi) = gun_mesh_max_sizes();
        println!("GUN_MAX: verts={} idx={}", mv, mi);
        // 逐枪规模（找容量边界问题）
        for spec in ALL_WEAPONS.iter() {
            if let Some(gm) = crate::engine::guns::gun_mesh_by_key(spec.key) {
                println!(
                    "GUN_SIZE {}: verts={} idx={}",
                    spec.key,
                    gm.verts.len(),
                    gm.indices.len()
                );
            }
        }
        // 找出最大的一把
        let mut biggest = "";
        let mut bv = 0u32;
        for spec in ALL_WEAPONS.iter() {
            if let Some(gm) = crate::engine::guns::gun_mesh_by_key(spec.key) {
                if (gm.verts.len() as u32) > bv {
                    bv = gm.verts.len() as u32;
                    biggest = spec.key;
                }
            }
        }
        println!("GUN_BIGGEST: {} verts={}", biggest, bv);
        assert!(mv > 0 && mi > 0);
    }

    /// 全部枪模可构建且非空（程序化建模完整性）
    #[test]
    fn all_gun_meshes_buildable() {
        for spec in ALL_WEAPONS.iter() {
            let gm = crate::engine::guns::gun_mesh_by_key(spec.key)
                .unwrap_or_else(|| panic!("{} 无枪模", spec.key));
            assert!(!gm.verts.is_empty(), "{} 网格为空", spec.key);
            assert!(!gm.indices.is_empty(), "{} 索引为空", spec.key);
            assert!(!gm.display_name.is_empty(), "{} 缺显示名", spec.key);
            assert!(gm.length > 0.15, "{} 长度异常 {}", spec.key, gm.length);
            // 索引合法性：任何索引不得越界（越界会在 GPU 端导致 device lost）
            let vmax = gm.verts.len() as u32;
            for (i, &idx) in gm.indices.iter().enumerate() {
                assert!(
                    idx < vmax,
                    "{} 索引 #{} = {} 越界（顶点数 {}）",
                    spec.key,
                    i,
                    idx,
                    vmax
                );
            }
        }
    }
}

