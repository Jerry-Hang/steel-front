# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
# ? → map_err 顶到函数级：把整个 rt_test 的 ? 换处理（暴力：替换 dev.create_buffer(...)? 等）
s = s.replace('dev.create_buffer(&vk::BufferCreateInfo::default().size(verts.len() as u64)', 'dev.create_buffer(&vk::BufferCreateInfo::default().size(verts.len() as u64)')
s = s.replace('None)?;', 'None).map_err(|e| format!("{e:?}"))?;')
# 特殊：accel structure create
s = s.replace('vk_asext.create_acceleration_structure(&as_info, None)?;', 'vk_asext.create_acceleration_structure(&as_info, None).map_err(|e| format!("{e:?}"))?;')
# MemoryAllocateInfo _marker：用 default + 字段赋值
old_ai = """            let ai = vk::MemoryAllocateInfo {
                s_type: vk::StructureType::MEMORY_ALLOCATE_INFO,
                p_next: &vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS) as *const _ as *const std::ffi::c_void,
                allocation_size: req.size, memory_type_index: idx,
            };"""
new_ai = """            let mut ai = vk::MemoryAllocateInfo::default();
            ai.allocation_size = req.size;
            ai.memory_type_index = idx;
            let mut fl = vk::MemoryAllocateFlagsInfo::default();
            fl.flags = vk::MemoryAllocateFlags::DEVICE_ADDRESS;
            ai.p_next = &fl as *const _ as *const std::ffi::c_void;"""
if old_ai in s:
    s = s.replace(old_ai, new_ai, 1)
    print('ai fixed')
else:
    print('ai miss')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
