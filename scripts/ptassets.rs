/// 路径追踪资源集（2026-08-29）
pub struct PtAssets {
    pub tlas: vk::AccelerationStructureKHR,
    pub blas: vk::AccelerationStructureKHR,
    pub tlas_buf: vk::Buffer,
    pub tlas_mem: vk::DeviceMemory,
    pub blas_buf: vk::Buffer,
    pub blas_mem: vk::DeviceMemory,
    pub verts_buf: vk::Buffer,
    pub verts_mem: vk::DeviceMemory,
    pub idx_buf: vk::Buffer,
    pub idx_mem: vk::DeviceMemory,
}

impl PtAssets {
    pub fn destroy(&self, d: &ash::Device) {
        unsafe {
            let ext = ash::khr::acceleration_structure::Device::new(&crate::instance_for_ext(), d);
            ext.destroy_acceleration_structure(self.tlas, None);
            ext.destroy_acceleration_structure(self.blas, None);
            d.destroy_buffer(self.verts_buf, None); d.free_memory(self.verts_mem, None);
            d.destroy_buffer(self.idx_buf, None); d.free_memory(self.idx_mem, None);
            d.destroy_buffer(self.tlas_buf, None); d.free_memory(self.tlas_mem, None);
            d.destroy_buffer(self.blas_buf, None); d.free_memory(self.blas_mem, None);
        }
    }
}