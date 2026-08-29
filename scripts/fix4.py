# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 1) 删除 renderer 内的 PtAssets 结构定义块
s = s.replace('''    /// 路径追踪资源集（TLAS/BLAS/几何缓冲句柄）
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

''', '')
# 2) 类型引用
s = s.replace("    ) -> Result<self::PtAssets, String> {", "    ) -> Result<crate::engine::ray_tracer::PtAssets, String> {")
# 3) 初始化补 inst_buf/inst_mem
s = s.replace("""            idx_buf: ibuf,
            idx_mem: imem,
        })""", """            idx_buf: ibuf,
            idx_mem: imem,
            inst_buf: inst_buf,
            inst_mem: inst_mem,
        })""")
# 4) 内联 range（删 khr_build_ranges 调用）
s = s.replace("""        let range_b = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 36 * (0 + 1), primitive_offset: 0, first_vertex: 0, transform_offset: 0 };
        // 注意 primitive_count = 实际三角数（盒数×12）——由调用方传入，此处保守""", """        let range_b = vk::AccelerationStructureBuildRangeInfoKHR { primitive_count: 36, primitive_offset: 0, first_vertex: 0, transform_offset: 0 };""")
s = s.replace("""        unsafe {
            ext.cmd_build_acceleration_structures(cmd, &[b_geom], &[&khr_build_ranges(&range_b)]);
            ext.cmd_build_acceleration_structures(cmd, &[t_geom], &[&khr_build_ranges(&range_t)]);
        }""", """        unsafe {
            let rb: [vk::AccelerationStructureBuildRangeInfoKHR; 4] = [range_b; 4];
            let rbs: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&rb[..1]];
            ext.cmd_build_acceleration_structures(cmd, &[b_geom], &rbs);
            let rt: [vk::AccelerationStructureBuildRangeInfoKHR; 1] = [range_t];
            let rts: [&[vk::AccelerationStructureBuildRangeInfoKHR]; 1] = [&rt];
            ext.cmd_build_acceleration_structures(cmd, &[t_geom], &rts);
        }""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('four fixes')
