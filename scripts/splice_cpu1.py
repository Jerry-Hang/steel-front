
# -*- coding: utf-8 -*-
import io
p = 'src/engine/cpu.rs'
s = io.open(p, encoding='utf-8').read()

old_fields = """    pub avx2: bool,
    pub avx512: bool,
}"""
new_fields = """    pub avx2: bool,
    pub avx512: bool,
    /// CCX/L3 缓存组（每组的逻辑处理器集合；Windows 按 L3 缓存精确分组，
    /// Zen1/Zen2 为 4 核 CCX、Zen3+ 为 8 核 CCD——旧“半半”拆分在早期 Zen 上错误）
    pub ccx_groups: Vec<Vec<usize>>,
    /// 每逻辑处理器能效等级（0=P 大核, 1=E 能效核, 2=LE 低功耗核；未知恒 0）
    pub efficiency: Vec<u8>,
    /// 低功耗能效核（Intel LE/LP-E；AMD 无 = 空）
    pub le_set: Vec<usize>,
    /// 音频等简单低延迟任务集合（LE > E > 第三 CCX > 次簇 > 主簇）
    pub audio_set: Vec<usize>,
}"""
assert old_fields in s
s = s.replace(old_fields, new_fields, 1)

anchor = "static TOPOLOGY: OnceLock<CpuTopology> = OnceLock::new();"
plat_mod = '''/// 平台精确拓扑中间结构（Windows 由 GetLogicalProcessorInformationEx 解析）
struct PlatformTopo {
    /// 物理核列表：每核 = 该核的逻辑处理器集合（SMT 配对，E-core 单成员）
    cores: Vec<Vec<usize>>,
    /// L3 缓存组（CCX）：每组的逻辑处理器集合
    ccx_groups: Vec<Vec<usize>>,
    /// 每逻辑处理器能效等级（0=P 1=E 2=LE）
    efficiency: Vec<u8>,
}

/// Windows：GetLogicalProcessorInformationEx 解析物理核/CCX(L3)/能效等级。
/// 零第三方依赖（kernel32 直接 FFI）；失败返回 None 回退旧逻辑。
#[cfg(target_os = "windows")]
mod win_topology {
    use super::PlatformTopo;

    const REL_PROCESSOR_CORE: u32 = 0;
    const REL_CACHE: u32 = 2;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct GroupAffinity {
        mask: usize,
        group: u16,
        reserved: [u16; 3],
    }

    #[repr(C)]
    struct ProcessorRel {
        flags: u8,
        efficiency: u8,
        reserved: [u8; 20],
        group_count: u16,
        group_mask: [GroupAffinity; 1],
    }

    #[repr(C)]
    struct CacheRel {
        level: u8,
        associativity: u8,
        line_size: u16,
        cache_size: u32,
        cache_type: u32,
        reserved: [u8; 20],
        group_mask: GroupAffinity,
    }

    #[repr(C)]
    struct InfoEx {
        relationship: u32,
        // 48 字节联合体（Cache 48B / Processor 40B；偏移 8 起）
        data: [u64; 6],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalProcessorInformationEx(
            relationship: u32,
            buffer: *mut InfoEx,
            returned_length: *mut u32,
        ) -> i32;
    }

    fn query(rel: u32) -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        unsafe {
            GetLogicalProcessorInformationEx(rel, std::ptr::null_mut(), &mut len);
        }
        if len == 0 {
            return None;
        }
        let mut buf: Vec<u8> = vec![0u8; len as usize];
        let ok = unsafe {
            GetLogicalProcessorInformationEx(
                rel,
                buf.as_mut_ptr() as *mut InfoEx,
                &mut len,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(buf)
    }

    fn mask_members(mask: usize) -> Vec<usize> {
        let mut v = Vec::new();
        for b in 0..usize::BITS {
            if mask & (1usize << b) != 0 {
                v.push(b as usize);
            }
        }
        v
    }

    pub fn detect() -> Option<PlatformTopo> {
        let mut cores: Vec<Vec<usize>> = Vec::new();
        let mut efficiency: Vec<u8> = Vec::new();
        let mut ccx: Vec<Vec<usize>> = Vec::new();

        if let Some(buf) = query(REL_PROCESSOR_CORE) {
            let stride = std::mem::size_of::<InfoEx>();
            for i in 0..buf.len() / stride {
                let e = unsafe { &*(buf.as_ptr().add(i * stride) as *const InfoEx) };
                if e.relationship != REL_PROCESSOR_CORE {
                    continue;
                }
                let pr = unsafe { &*(e.data.as_ptr() as *const ProcessorRel) };
                if pr.group_count < 1 {
                    continue;
                }
                let members = mask_members(pr.group_mask[0].mask);
                if !members.is_empty() {
                    for _ in 0..members.len() {
                        efficiency.push(pr.efficiency);
                    }
                    cores.push(members);
                }
            }
        }
        if let Some(buf) = query(REL_CACHE) {
            let stride = std::mem::size_of::<InfoEx>();
            for i in 0..buf.len() / stride {
                let e = unsafe { &*(buf.as_ptr().add(i * stride) as *const InfoEx) };
                if e.relationship != REL_CACHE {
                    continue;
                }
                let cr = unsafe { &*(e.data.as_ptr() as *const CacheRel) };
                if cr.level == 3 {
                    let members = mask_members(cr.group_mask.mask);
                    if !members.is_empty() {
                        ccx.push(members);
                    }
                }
            }
        }
        if cores.is_empty() {
            return None;
        }
        Some(PlatformTopo {
            cores,
            ccx_groups: ccx,
            efficiency,
        })
    }
}

'''
assert anchor in s
s = s.replace(anchor, plat_mod + anchor, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('part1 ok')
