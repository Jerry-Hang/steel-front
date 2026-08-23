# -*- coding: utf-8 -*-
import io
p = 'src/engine/cpu.rs'
s = io.open(p, encoding='utf-8').read()

start = s.index("    pub fn detect() -> CpuTopology {")
end_marker = "    /// 把当前线程（主线程）绑定到目标 vCPU 集合。"
end = s.rindex("\n", 0, s.index(end_marker))

new_detect = r'''    pub fn detect() -> CpuTopology {
        let vendor = cpu_vendor();
        let threads = cpu_thread_count();
        // 平台精确拓扑：Windows 用 GetLogicalProcessorInformationEx（物理核/CCX/能效等级），
        // 其它平台 None → 回退旧逻辑（sysfs + CPUID + 半半）。
        #[cfg(target_os = "windows")]
        let plat = win_topology::detect();
        #[cfg(not(target_os = "windows"))]
        let plat: Option<PlatformTopo> = None;

        let (primary_set, secondary_set, le_set, ccx_groups, efficiency, e_cores) =
            if let Some(pt) = &plat {
                if pt.efficiency.iter().any(|&e| e != 0) {
                    // 混合架构（Intel P/E/LE；AMD Zen4C/5C 被系统标记时同样生效）：
                    // 按能效等级分组：0=P 主簇（渲染/逻辑），1=E（AI/后台），2=LE（音频/低延迟）
                    let n = pt.efficiency.len();
                    let p: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 0).collect();
                    let e: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 1).collect();
                    let le: Vec<usize> = (0..n).filter(|&c| pt.efficiency[c] == 2).collect();
                    // E 核物理数：每物理核（cores 条目）只要有一成员为 E 即计 1
                    let mut ec = 0usize;
                    for core in &pt.cores {
                        if core.iter().any(|&c| c < pt.efficiency.len() && pt.efficiency[c] == 1) {
                            ec += 1;
                        }
                    }
                    (p, e, le, pt.ccx_groups.clone(), pt.efficiency.clone(), ec)
                } else if pt.ccx_groups.len() >= 2 {
                    // AMD 双簇（CCX/L3 分组）：主簇 = 组 0（逻辑/渲染同簇，共享 L3），
                    // 次簇 = 组 1（AI/地图生成）。Zen1/Zen2 为 4 核 CCX 时精确成立。
                    (
                        pt.ccx_groups[0].clone(),
                        pt.ccx_groups[1].clone(),
                        Vec::new(),
                        pt.ccx_groups.clone(),
                        pt.efficiency.clone(),
                        0,
                    )
                } else {
                    // 无小核且无 L3 分组：回退半半
                    let half = threads / 2;
                    (
                        (0..half).collect(),
                        (half..threads).collect(),
                        Vec::new(),
                        pt.ccx_groups.clone(),
                        pt.efficiency.clone(),
                        0,
                    )
                }
            } else {
                // 旧逻辑（Linux sysfs / 其它平台）——语义不变
                let (primary_set, secondary_set, e_cores) = match vendor {
                    CpuVendor::Amd => {
                        let half = threads / 2;
                        ((0..half).collect::<Vec<usize>>(), (half..threads).collect::<Vec<usize>>(), 0)
                    }
                    CpuVendor::Intel => {
                        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                        {
                            let hybrid = probe_per_cpu(0x1A);
                            let mut seen = std::collections::HashSet::new();
                            for (i, &eax) in hybrid.iter().enumerate() {
                                if ((eax >> 24) & 0xff) == 0x20 {
                                    seen.insert(i);
                                }
                            }
                            let e_cpus = seen;
                            let p: Vec<usize> = (0..threads).filter(|c| !e_cpus.contains(c)).collect();
                            let e: Vec<usize> = (0..threads).filter(|c| e_cpus.contains(c)).collect();
                            let e_count = e.len();
                            (p, e, e_count)
                        }
                        #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
                        {
                            ((0..threads).collect(), Vec::new(), 0)
                        }
                    }
                    CpuVendor::Other => ((0..threads).collect(), Vec::new(), 0),
                };
                (primary_set, secondary_set, Vec::new(), Vec::new(), Vec::new(), e_cores)
            };
        // 物理核/超线程识别：Windows 从核心关系直接获得（SMT 配对 = 每核逻辑集合）；
        // Linux sysfs；不可读时回退不区分
        let (primary_physical, secondary_physical, smt_set) = if let Some(pt) = &plat {
            let mut physical_all = Vec::new();
            let mut smt_all = Vec::new();
            for core in &pt.cores {
                let min = *core.iter().min().unwrap_or(&0);
                physical_all.push(min);
                for &c in core {
                    if c != min {
                        smt_all.push(c);
                    }
                }
            }
            let pp: Vec<usize> = primary_set
                .iter()
                .filter(|c| physical_all.contains(c))
                .copied()
                .collect();
            let sp: Vec<usize> = secondary_set
                .iter()
                .filter(|c| physical_all.contains(c))
                .copied()
                .collect();
            let pp = if pp.is_empty() { primary_set.clone() } else { pp };
            let sp = if sp.is_empty() { secondary_set.clone() } else { sp };
            (pp, sp, smt_all)
        } else {
            match read_smt_siblings(threads) {
                Some(groups) => {
                    let (physical_all, smt_all) = split_smt_pairs(&groups);
                    let pp: Vec<usize> = primary_set
                        .iter()
                        .filter(|c| physical_all.contains(c))
                        .copied()
                        .collect();
                    let sp: Vec<usize> = secondary_set
                        .iter()
                        .filter(|c| physical_all.contains(c))
                        .copied()
                        .collect();
                    let pp = if pp.is_empty() { primary_set.clone() } else { pp };
                    let sp = if sp.is_empty() { secondary_set.clone() } else { sp };
                    (pp, sp, smt_all)
                }
                None => (primary_set.clone(), secondary_set.clone(), Vec::new()),
            }
        };
        // 音频集合（简单/低延迟任务）：LE 核 > E 核 > 第三 CCX > 次簇 > 主簇
        let audio_set = if !le_set.is_empty() {
            le_set.clone()
        } else if !secondary_set.is_empty() && vendor == CpuVendor::Intel {
            secondary_set.clone()
        } else if ccx_groups.len() >= 3 {
            ccx_groups[2].clone()
        } else if !secondary_set.is_empty() {
            secondary_set.clone()
        } else {
            primary_set.clone()
        };
        CpuTopology {
            vendor,
            threads,
            primary_set,
            secondary_set,
            primary_physical,
            secondary_physical,
            smt_set,
            e_cores,
            avx2: {
                #[cfg(target_arch = "x86_64")]
                {
                    std::is_x86_feature_detected!("avx2")
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    false
                }
            },
            avx512: avx512_enabled(),
            ccx_groups,
            efficiency,
            le_set,
            audio_set,
        }
    }

'''
s2 = s[:start] + new_detect + s[end:]
io.open(p, 'w', encoding='utf-8', newline='').write(s2)
print('part2 ok', len(new_detect))
