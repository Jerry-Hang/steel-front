# -*- coding: utf-8 -*-
import io
# ray_tracer.rs 加 PtAssets（用 ash 类型——renderer 用它；放这里复用）
s = io.open('src/engine/ray_tracer.rs', encoding='utf-8').read()
if 'pub struct PtAssets' not in s:
    s += "\n/// 路径追踪 GPU 资源集（构建/记录/销毁）\npub struct PtAssets {\n    pub tlas: ash::vk::AccelerationStructureKHR,\n    pub blas: ash::vk::AccelerationStructureKHR,\n    pub tlas_buf: ash::vk::Buffer,\n    pub tlas_mem: ash::vk::DeviceMemory,\n    pub blas_buf: ash::vk::Buffer,\n    pub blas_mem: ash::vk::DeviceMemory,\n    pub verts_buf: ash::vk::Buffer,\n    pub verts_mem: ash::vk::DeviceMemory,\n    pub idx_buf: ash::vk::Buffer,\n    pub idx_mem: ash::vk::DeviceMemory,\n    pub inst_buf: ash::vk::Buffer,\n    pub inst_mem: ash::vk::DeviceMemory,\n}\n"
    io.open('src/engine/ray_tracer.rs', 'w', encoding='utf-8', newline='').write(s)
    print('PtAssets in ray_tracer')
else:
    print('has')
