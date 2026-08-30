# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
helper = '''
fn mem_alloc_ex(
    dev: &ash::Device,
    instance: &ash::Instance,
    phys: vk::PhysicalDevice,
    buf: vk::Buffer,
    host_visible: bool,
    with_address: bool,
) -> Result<(vk::DeviceMemory, u64), String> {
    unsafe {
        let req = dev.get_buffer_memory_requirements(buf);
        let mprops = instance.get_physical_device_memory_properties(phys);
        let mut idx = 0u32;
        for (i, t) in mprops.memory_types.iter().enumerate() {
            if req.memory_type_bits & (1 << i) != 0 {
                let prop = if host_visible {
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
                } else {
                    vk::MemoryPropertyFlags::DEVICE_LOCAL
                };
                if t.property_flags.contains(prop) { idx = i as u32; break; }
            }
        }
        let mut ai = vk::MemoryAllocateInfo::default();
        ai.allocation_size = req.size;
        ai.memory_type_index = idx;
        if with_address {
            let mut fl = vk::MemoryAllocateFlagsInfo::default();
            fl.flags = vk::MemoryAllocateFlags::DEVICE_ADDRESS;
            ai.p_next = &fl as *const _ as *const std::ffi::c_void;
        }
        let mem = dev.allocate_memory(&ai, None).map_err(|e| format!("alloc: {e:?}"))?;
        dev.bind_buffer_memory(buf, mem, 0).map_err(|e| format!("bind: {e:?}"))?;
        Ok((mem, req.size))
    }
}

// buffer 创建 helper
fn create_buf(dev: &ash::Device, size: u64, usage: vk::BufferUsageFlags) -> Result<vk::Buffer, String> {
    unsafe {
        dev.create_buffer(
            &vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        ).map_err(|e| format!("create_buffer: {e:?}"))
    }
}

'''
s = s.replace("fn box_indices() -> [u32; 36] {", helper + "fn box_indices() -> [u32; 36] {")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('helpers added')
