// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    /// 创建立方体顶点缓冲（24 顶点）
    pub(crate) fn create_vertex_buffer(&mut self) -> Result<(), String> {
        let buffer_size = std::mem::size_of_val(&VERTICES) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, buffer_size)?;
        self.vertex_buffer = buffer;
        self.vertex_buffer_memory = memory;

        let data_ptr = unsafe {
            self.device
                .map_memory(
                    self.vertex_buffer_memory,
                    0,
                    buffer_size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("映射顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                VERTICES.as_ptr() as *const u8,
                data_ptr as *mut u8,
                buffer_size as usize,
            );
            self.device.unmap_memory(self.vertex_buffer_memory);
        }
        Ok(())
    }
    /// 创建立方体索引缓冲（36 索引，UINT32）
    pub(crate) fn create_index_buffer(&mut self) -> Result<(), String> {
        let buffer_size = std::mem::size_of_val(&INDICES) as u64;
        let (buffer, memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, buffer_size)?;
        self.index_buffer = buffer;
        self.index_buffer_memory = memory;

        let data_ptr = unsafe {
            self.device
                .map_memory(
                    self.index_buffer_memory,
                    0,
                    buffer_size,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| format!("映射索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                INDICES.as_ptr() as *const u8,
                data_ptr as *mut u8,
                buffer_size as usize,
            );
            self.device.unmap_memory(self.index_buffer_memory);
        }
        Ok(())
    }
    /// 创建远档 LOD 十字双 quad 的顶点/索引缓冲（8 顶点 / 12 索引）
    pub(crate) fn create_far_geometry(&mut self) -> Result<(), String> {
        let vert_size = std::mem::size_of_val(&FAR_VERTS) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.far_vertex_buffer = v_buffer;
        self.far_vertex_buffer_memory = v_memory;

        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射远档顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                FAR_VERTS.as_ptr() as *const u8,
                v_ptr as *mut u8,
                vert_size as usize,
            );
            self.device.unmap_memory(v_memory);
        }

        let idx_size = std::mem::size_of_val(&FAR_INDICES) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.far_index_buffer = i_buffer;
        self.far_index_buffer_memory = i_memory;

        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射远档索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                FAR_INDICES.as_ptr() as *const u8,
                i_ptr as *mut u8,
                idx_size as usize,
            );
            self.device.unmap_memory(i_memory);
        }
        log::info!("远档 LOD 几何创建完成: {} 顶点 / {} 索引（十字双 quad）", FAR_VERTS.len(), FAR_INDICES.len());
        Ok(())
    }
    /// 创建地面平铺 quad 的顶点/索引缓冲（4 顶点 / 6 索引），近档+远档地面 draw 共用。
    pub(crate) fn create_ground_geometry(&mut self) -> Result<(), String> {
        let vert_size = std::mem::size_of_val(&GROUND_VERTS) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.ground_vertex_buffer = v_buffer;
        self.ground_vertex_buffer_memory = v_memory;

        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射地面顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                GROUND_VERTS.as_ptr() as *const u8,
                v_ptr as *mut u8,
                vert_size as usize,
            );
            self.device.unmap_memory(v_memory);
        }

        let idx_size = std::mem::size_of_val(&GROUND_INDICES) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.ground_index_buffer = i_buffer;
        self.ground_index_buffer_memory = i_memory;

        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射地面索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                GROUND_INDICES.as_ptr() as *const u8,
                i_ptr as *mut u8,
                idx_size as usize,
            );
            self.device.unmap_memory(i_memory);
        }
        log::info!(
            "地面平铺 quad 几何创建完成: {} 顶点 / {} 索引（无侧壁）",
            GROUND_VERTS.len(),
            GROUND_INDICES.len()
        );
        Ok(())
    }
    /// 创建 UV 球体几何（爆炸球形扩散用）：24 经 × 12 纬段，CPU 生成顶点/索引。
    /// 球面坐标 (u,v) → 单位球 (sinφ·cosθ, cosφ, sinφ·sinθ)，白化颜色走 tint。
    pub(crate) fn create_sphere_geometry(&mut self) -> Result<(), String> {
        const SEGS: u32 = 24; // 经线
        const RINGS: u32 = 12; // 纬线
        let mut verts: Vec<Vertex> = Vec::with_capacity(((SEGS + 1) * (RINGS + 1)) as usize);
        for j in 0..=RINGS {
            let phi = std::f32::consts::PI * j as f32 / RINGS as f32; // 0..π
            let (sp, cp) = phi.sin_cos();
            for i in 0..=SEGS {
                let theta = std::f32::consts::TAU * i as f32 / SEGS as f32;
                let (st, ct) = theta.sin_cos();
                verts.push(Vertex {
                    pos: [sp * ct, cp, sp * st],
                    color: [1.0, 1.0, 1.0],
                    uv: [i as f32 / SEGS as f32, 1.0 - j as f32 / RINGS as f32],
                });
            }
        }
        let mut indices: Vec<u32> = Vec::with_capacity((SEGS * RINGS * 6) as usize);
        for j in 0..RINGS {
            for i in 0..SEGS {
                let a = j * (SEGS + 1) + i;
                let b = a + 1;
                let c = a + SEGS + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let vert_size = (verts.len() * std::mem::size_of::<Vertex>()) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.sphere_vertex_buffer = v_buffer;
        self.sphere_vertex_buffer_memory = v_memory;
        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射球体顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, v_ptr as *mut u8, vert_size as usize);
            self.device.unmap_memory(v_memory);
        }
        let idx_size = (indices.len() * std::mem::size_of::<u32>()) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.sphere_index_buffer = i_buffer;
        self.sphere_index_buffer_memory = i_memory;
        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射球体索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr() as *const u8, i_ptr as *mut u8, idx_size as usize);
            self.device.unmap_memory(i_memory);
        }
        self.sphere_index_count = indices.len() as u32;
        log::info!("球体几何创建完成: {} 顶点 / {} 索引（爆炸球形扩散）", verts.len(), indices.len());
        Ok(())
    }
    /// 圆柱几何数据（上传与绕序回归测试共用）：单位圆柱 r=1 h=1 沿 Y，24 段，含上下盖。
    /// 水平盖的绕序必须与地面 quad 同约定（从上方可见 ⇔ (x,z) 有向面积 > 0），
    /// 立方体顶/底面曾因反绕被上方剔除，判据见 `horizontal_winding_tests`。
    pub(crate) fn cylinder_mesh_data() -> (Vec<Vertex>, Vec<u32>) {
        const SEGS: u32 = 24;
        let mut verts: Vec<Vertex> = Vec::with_capacity((SEGS * 2 + 2) as usize);
        // 侧壁：上下两圈（y = ±0.5）
        for j in 0..2 {
            let y = if j == 0 { -0.5 } else { 0.5 };
            for i in 0..SEGS {
                let theta = std::f32::consts::TAU * i as f32 / SEGS as f32;
                let (st, ct) = theta.sin_cos();
                verts.push(Vertex {
                    pos: [ct, y, st],
                    color: [1.0, 1.0, 1.0],
                    uv: [i as f32 / SEGS as f32, j as f32],
                });
            }
        }
        // 上下盖中心顶点
        let top_center = verts.len() as u32;
        verts.push(Vertex { pos: [0.0, 0.5, 0.0], color: [1.0, 1.0, 1.0], uv: [0.5, 1.0] });
        let bottom_center = verts.len() as u32;
        verts.push(Vertex { pos: [0.0, -0.5, 0.0], color: [1.0, 1.0, 1.0], uv: [0.5, 0.0] });
        let mut indices: Vec<u32> = Vec::with_capacity((SEGS * 6 + SEGS * 6) as usize);
        for i in 0..SEGS {
            let a = i;
            let b = (i + 1) % SEGS;
            // 侧壁三角形（a=下圈, b=下圈+1, c=上圈... 下圈顶点 0..SEGS，上圈 SEGS..2*SEGS）
            let t0 = a;
            let t1 = b;
            let t2 = SEGS + a;
            let t3 = SEGS + b;
            indices.extend_from_slice(&[t0, t2, t1, t1, t2, t3]);
            // 上盖 fan（绕序同地面 quad：从上方看是正面）
            indices.extend_from_slice(&[top_center, t2, t3]);
            // 下盖 fan（反向：从下方看才是正面）
            indices.extend_from_slice(&[bottom_center, t1, t0]);
        }
        (verts, indices)
    }
    /// 创建 NPC 人体圆柱几何（四肢用）：单位圆柱 r=1 h=1 沿 Y，24 段，含上下盖。
    pub(crate) fn create_cylinder_geometry(&mut self) -> Result<(), String> {
        let (verts, indices) = Self::cylinder_mesh_data();
        let vert_size = (verts.len() * std::mem::size_of::<Vertex>()) as u64;
        let (v_buffer, v_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::VERTEX_BUFFER, vert_size)?;
        self.cylinder_vertex_buffer = v_buffer;
        self.cylinder_vertex_buffer_memory = v_memory;
        let v_ptr = unsafe {
            self.device
                .map_memory(v_memory, 0, vert_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射圆柱顶点缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(verts.as_ptr() as *const u8, v_ptr as *mut u8, vert_size as usize);
            self.device.unmap_memory(v_memory);
        }
        let idx_size = (indices.len() * std::mem::size_of::<u32>()) as u64;
        let (i_buffer, i_memory) =
            self.create_host_buffer(vk::BufferUsageFlags::INDEX_BUFFER, idx_size)?;
        self.cylinder_index_buffer = i_buffer;
        self.cylinder_index_buffer_memory = i_memory;
        let i_ptr = unsafe {
            self.device
                .map_memory(i_memory, 0, idx_size, vk::MemoryMapFlags::empty())
                .map_err(|e| format!("映射圆柱索引缓冲内存失败: {}", e))?
        };
        unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr() as *const u8, i_ptr as *mut u8, idx_size as usize);
            self.device.unmap_memory(i_memory);
        }
        self.cylinder_index_count = indices.len() as u32;
        log::info!("圆柱几何创建完成: {} 顶点 / {} 索引（NPC 四肢）", verts.len(), indices.len());
        Ok(())
    }
    /// 创建 3 级地形 LOD 网格（高 257² / 中 129² / 低 65² 顶点）。
    /// 高度用与实例 Y 完全相同的 terrain_height() 生成；顶点缓冲 HOST_VISIBLE，
    /// 过渡带内每帧 morph 高度后整块重传；索引缓冲一次性上传。
    pub(crate) fn create_terrain_lods(&mut self) -> Result<(), String> {
        // 1. 各级网格原始高度（粗网格顶点恰为细网格顶点子集）
        let grid_heights: Vec<Vec<f32>> = TERRAIN_LOD_CELLS
            .iter()
            .map(|&cells| {
                let w = cells + 1;
                let cell = 512.0 / cells as f32;
                let mut hs = Vec::with_capacity(w * w);
                for iz in 0..w {
                    let z = -TERRAIN_HALF + iz as f32 * cell;
                    for ix in 0..w {
                        let x = -TERRAIN_HALF + ix as f32 * cell;
                        hs.push(terrain_height(x, z));
                    }
                }
                hs
            })
            .collect();

        for idx in 0..TERRAIN_LOD_CELLS.len() {
            let level = TerrainLod::from_idx(idx);
            let cells = level.cells();
            let w = level.verts();
            let cell = level.cell_size();
            let heights = &grid_heights[idx];

            // 顶点（UV 用世界坐标，保证各级贴图对齐；颜色白）
            let mut verts: Vec<Vertex> = Vec::with_capacity(w * w);
            let mut base_heights: Vec<f32> = Vec::with_capacity(w * w);
            for iz in 0..w {
                for ix in 0..w {
                    let x = -TERRAIN_HALF + ix as f32 * cell;
                    let z = -TERRAIN_HALF + iz as f32 * cell;
                    let y = heights[iz * w + ix] - TERRAIN_RENDER_SINK;
                    base_heights.push(y);
                    verts.push(Vertex {
                        pos: [x, y, z],
                        color: [1.0, 1.0, 1.0],
                        uv: [
                            (x + TERRAIN_HALF) / TERRAIN_UV_SCALE,
                            (z + TERRAIN_HALF) / TERRAIN_UV_SCALE,
                        ],
                    });
                }
            }

            // morph 目标高度：下一级（更粗）曲面三角形插值；Low 级无下一级
            let coarse_heights: Vec<f32> = if idx + 1 < TERRAIN_LOD_CELLS.len() {
                let coarse = &grid_heights[idx + 1];
                let coarse_cells = TERRAIN_LOD_CELLS[idx + 1];
                (0..w)
                    .flat_map(|iz| {
                        (0..w).map(move |ix| {
                            let x = -TERRAIN_HALF + ix as f32 * cell;
                            let z = -TERRAIN_HALF + iz as f32 * cell;
                            terrain_coarse_height(x, z, coarse, coarse_cells) - TERRAIN_RENDER_SINK
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // 索引（与原有地形相同的三角形剖分：cell 对角 v0→v2）
            let mut idx_buf: Vec<u32> = Vec::with_capacity(cells * cells * 6);
            for iz in 0..cells {
                for ix in 0..cells {
                    let v0 = (iz * w + ix) as u32;
                    let v1 = v0 + 1;
                    let v2 = v0 + w as u32 + 1;
                    let v3 = v0 + w as u32;
                    idx_buf.push(v0);
                    idx_buf.push(v2);
                    idx_buf.push(v1);
                    idx_buf.push(v0);
                    idx_buf.push(v3);
                    idx_buf.push(v2);
                }
            }

            // 顶点缓冲：HOST_VISIBLE 并持久映射（每帧 morph 后整块重传）
            let vert_bytes = unsafe {
                std::slice::from_raw_parts(
                    verts.as_ptr() as *const u8,
                    verts.len() * std::mem::size_of::<Vertex>(),
                )
            };
            let (v_buffer, v_memory) = self.create_host_buffer(
                vk::BufferUsageFlags::VERTEX_BUFFER,
                vert_bytes.len() as u64,
            )?;
            let v_ptr = unsafe {
                self.device
                    .map_memory(
                        v_memory,
                        0,
                        vert_bytes.len() as u64,
                        vk::MemoryMapFlags::empty(),
                    )
                    .map_err(|e| format!("映射地形 LOD[{}] 顶点内存失败: {}", idx, e))?
            };
            unsafe {
                std::ptr::copy_nonoverlapping(vert_bytes.as_ptr(), v_ptr as *mut u8, vert_bytes.len());
            }

            // 索引缓冲：静态数据，DEVICE_LOCAL 一次性上传（staging 拷贝）
            let idx_bytes = unsafe {
                std::slice::from_raw_parts(
                    idx_buf.as_ptr() as *const u8,
                    idx_buf.len() * std::mem::size_of::<u32>(),
                )
            };
            let (i_buffer, i_memory) = self.create_device_local_buffer(
                vk::BufferUsageFlags::INDEX_BUFFER,
                idx_bytes,
                "地形索引",
            )?;

            self.terrain_lods.push(TerrainLodMesh {
                vertex_buffer: v_buffer,
                vertex_memory: v_memory,
                vertex_mapped: v_ptr,
                index_buffer: i_buffer,
                index_memory: i_memory,
                index_count: level.index_count(),
                verts,
                base_heights,
                coarse_heights,
            });

            log::info!(
                "地形 LOD[{}] 创建完成: {} 顶点 / {} 索引（{}×{} 网格，间距 {}）",
                idx,
                w * w,
                cells * cells * 6,
                w,
                w,
                cell
            );
        }
        log::info!("地形 3 级 LOD 全部创建完成（高/中/低）");
        Ok(())
    }
    /// 标量地形 LOD morph 高度：y = base + (coarse − base) × blend（回退路径/基准语义）
    pub(crate) fn morph_heights_scalar(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        for i in 0..out.len() {
            out[i] = base[i] + (coarse[i] - base[i]) * blend;
        }
    }
    /// AVX-512 地形 morph：16 顶点/批。运算顺序与标量一致（先 sub 再 mul 再 add，无 FMA），
    /// IEEE 逐位一致。★ AVX-512 加速说明：Zen4/Zen5（7000/9000 系）双 256 单元合并执行
    /// 512 位请求；选路走 cpu::avx512_enabled()（Intel 11 代能效差 / 12 代起大小核自动禁用）。
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx512f")]
    pub(crate) unsafe fn morph_heights_avx512(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm512_set1_ps(blend);
        let mut i = 0usize;
        while i + 16 <= out.len() {
            let bv = _mm512_loadu_ps(base.as_ptr().add(i));
            let cv = _mm512_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm512_sub_ps(cv, bv);
            let y = _mm512_add_ps(bv, _mm512_mul_ps(diff, b));
            _mm512_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 16;
        }
        // 尾部不足 16 个走标量（与 cull 尾部队列策略一致）
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }
    /// AVX2 地形 morph：8 顶点/批（与标量逐位一致，非 FMA）
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn morph_heights_avx2(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm256_set1_ps(blend);
        let mut i = 0usize;
        while i + 8 <= out.len() {
            let bv = _mm256_loadu_ps(base.as_ptr().add(i));
            let cv = _mm256_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm256_sub_ps(cv, bv);
            let y = _mm256_add_ps(bv, _mm256_mul_ps(diff, b));
            _mm256_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 8;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }
    /// AVX（非 AVX2，3/4 代酷睿与初代锐龙）地形 morph：8 顶点/批
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx")]
    pub(crate) unsafe fn morph_heights_avx(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm256_set1_ps(blend);
        let mut i = 0usize;
        while i + 8 <= out.len() {
            let bv = _mm256_loadu_ps(base.as_ptr().add(i));
            let cv = _mm256_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm256_sub_ps(cv, bv);
            let y = _mm256_add_ps(bv, _mm256_mul_ps(diff, b));
            _mm256_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 8;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }
    /// SSE4.2 地形 morph：4 顶点/批（2008 年后所有 Intel/AMD 消费级）
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "sse4.2")]
    pub(crate) unsafe fn morph_heights_sse(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::x86_64::*;
        let b = _mm_set1_ps(blend);
        let mut i = 0usize;
        while i + 4 <= out.len() {
            let bv = _mm_loadu_ps(base.as_ptr().add(i));
            let cv = _mm_loadu_ps(coarse.as_ptr().add(i));
            let diff = _mm_sub_ps(cv, bv);
            let y = _mm_add_ps(bv, _mm_mul_ps(diff, b));
            _mm_storeu_ps(out.as_mut_ptr().add(i), y);
            i += 4;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }
    /// NEON（AArch64，Apple Silicon/Android/高通 X Elite）地形 morph：4 顶点/批
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    pub(crate) unsafe fn morph_heights_neon(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        use std::arch::aarch64::*;
        let b = vdupq_n_f32(blend);
        let mut i = 0usize;
        while i + 4 <= out.len() {
            let bv = vld1q_f32(base.as_ptr().add(i));
            let cv = vld1q_f32(coarse.as_ptr().add(i));
            let diff = vsubq_f32(cv, bv);
            let y = vaddq_f32(bv, vmulq_f32(diff, b));
            vst1q_f32(out.as_mut_ptr().add(i), y);
            i += 4;
        }
        for j in i..out.len() {
            out[j] = base[j] + (coarse[j] - base[j]) * blend;
        }
    }
    /// 地形 morph 高度选路（与剔除同策略，见 cull_spheres_dispatch）：
    /// x86_64：AVX-512(16) > AVX2(8) > AVX(8) > SSE4.2(4) > 标量；aarch64：NEON(4) > 标量。
    pub(crate) fn morph_heights_dispatch(base: &[f32], coarse: &[f32], blend: f32, out: &mut [f32]) {
        #[cfg(target_arch = "x86_64")]
        {
            // 基准用强制选路（RV3D_FORCE_SIMD，见 cpu::forced_simd_path）；仍要求硬件支持
            if let Some(forced) = crate::engine::cpu::forced_simd_path() {
                let supported = match forced {
                    "avx512" => std::is_x86_feature_detected!("avx512f"),
                    "avx2" => std::is_x86_feature_detected!("avx2"),
                    "avx" => std::is_x86_feature_detected!("avx"),
                    "sse4.2" => std::is_x86_feature_detected!("sse4.2"),
                    "scalar" => true,
                    _ => false,
                };
                if supported {
                    match forced {
                        "avx512" => {
                            // safety: 上面已确认 avx512f 硬件支持
                            unsafe {
                                Self::morph_heights_avx512(base, coarse, blend, out);
                            }
                        }
                        "avx2" => {
                            // safety: 上面已确认 avx2 硬件支持
                            unsafe {
                                Self::morph_heights_avx2(base, coarse, blend, out);
                            }
                        }
                        "avx" => {
                            // safety: 上面已确认 avx 硬件支持
                            unsafe {
                                Self::morph_heights_avx(base, coarse, blend, out);
                            }
                        }
                        "sse4.2" => {
                            // safety: 上面已确认 sse4.2 硬件支持
                            unsafe {
                                Self::morph_heights_sse(base, coarse, blend, out);
                            }
                        }
                        _ => Self::morph_heights_scalar(base, coarse, blend, out),
                    }
                    return;
                }
                // 每帧调用（morph 每级 / 剔除每段）⇒ 走一次性告警，见 `simd::warn_forced_simd_unsupported`
                crate::engine::simd::warn_forced_simd_unsupported(forced);
            }
            if crate::engine::cpu::avx512_enabled() {
                // safety: 上面已运行时检测 AVX-512，CPU 支持才进入该分支
                unsafe {
                    Self::morph_heights_avx512(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("avx2") {
                // safety: 上面已运行时检测 AVX2
                unsafe {
                    Self::morph_heights_avx2(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("avx") {
                // safety: 上面已运行时检测 AVX
                unsafe {
                    Self::morph_heights_avx(base, coarse, blend, out);
                }
            } else if std::is_x86_feature_detected!("sse4.2") {
                // safety: 上面已运行时检测 SSE4.2
                unsafe {
                    Self::morph_heights_sse(base, coarse, blend, out);
                }
            } else {
                Self::morph_heights_scalar(base, coarse, blend, out);
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            if std::arch::is_aarch64_feature_detected!("neon") {
                // safety: NEON 在 AArch64 是基线特性（此处仍运行时确认）
                unsafe {
                    Self::morph_heights_neon(base, coarse, blend, out);
                }
            } else {
                Self::morph_heights_scalar(base, coarse, blend, out);
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::morph_heights_scalar(base, coarse, blend, out);
        }
    }
    /// 每帧按 morph 进度 t 更新当前 LOD 网格顶点高度：
    /// h = 细高度 + t × (下一级曲面插值高度 − 细高度)。t=1 时几何与下一级完全重合，
    /// 因此切换级别无 popping。仅在过渡带内（0<t<1）执行。
    /// 计算走 SIMD 选路（AVX-512 > AVX2 > AVX > SSE4.2 > NEON > 标量，逐位一致），
    /// 写回按段并行（scene_pool：AMD CCD0 / Intel P-core，与渲染主线程同簇）。
    /// 仅写 y 分量 4B/顶点：其余顶点分量上一帧已就位，映射内存常驻无需整块重传。
    pub(crate) fn update_terrain_lod_morph(&mut self, level: TerrainLod, blend: f32) {
        if blend <= 0.0 || blend >= 1.0 {
            return;
        }
        let idx = level as usize;
        let mesh = match self.terrain_lods.get_mut(idx) {
            Some(m) => m,
            None => return,
        };
        if mesh.coarse_heights.is_empty() {
            return;
        }
        let base = &mesh.base_heights;
        let coarse = &mesh.coarse_heights;
        let n = mesh.verts.len();
        // 1) SIMD 计算 y 数组（n ≤ 65536 → 最多 256KB，过渡带内才执行）
        let mut ys = vec![0.0f32; n];
        Self::morph_heights_dispatch(base, coarse, blend, &mut ys);
        // 2) 并行写回 verts.pos[1] + 映射内存 y 分量（段间不相交，join 后才返回）
        let stride = std::mem::size_of::<Vertex>();
        let mapped = crate::engine::cpu::SendPtr(mesh.vertex_mapped as *mut u8);
        let pool = crate::engine::cpu::scene_pool();
        pool.par_for_each_mut(&mut mesh.verts, move |_seg, start, slice| {
            for (k, v) in slice.iter_mut().enumerate() {
                let y = ys[start + k];
                v.pos[1] = y;
                // SAFETY: mapped 指向 HOST_VISIBLE 顶点缓冲（常驻映射，本帧未写入该区段）；
                // 各段只写 [ (start+k)*stride+4, +4 ) 的 y 分量，互不相交。
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        &y as *const f32 as *const u8,
                        mapped.get().add((start + k) * stride + 4),
                        4,
                    );
                }
            }
        });
    }
    /// 生成 256×256 网格实例；按 frame-in-flight 数量双缓冲
    /// （每帧一份 HOST_VISIBLE|HOST_COHERENT buffer，剔除后压缩上传到当前帧 slot）
    pub(crate) fn create_instance_buffer(&mut self) -> Result<(), String> {
        debug_assert!(
            std::mem::size_of::<InstanceData>() == 80,
            "InstanceData 必须对齐 std430 步长 80 字节"
        );

        // 256×256 网格：间距 2.0、以原点为中心、y=0 平面（场地 512×512）。
        // 地面实例用专用平铺 quad 几何（GROUND_VERTS，几何无侧壁），矩阵纯平移，
        // 高度 = terrain_height + 0.05（略高于地形网格 y=0，避免深度冲突）。
        // 旧版 2×2×2 立方体（顶面 +0.95）与压扁薄片（侧壁 0.2m）都有可见侧壁，
        // 视觉上像一格一格"掀盖纸箱"铺地；平铺 quad 无任何竖立面，地面真正连续。
        self.instances = Vec::with_capacity(INSTANCE_COUNT as usize);
        for iz in 0..GRID_SIZE {
            for ix in 0..GRID_SIZE {
                let x = (ix as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let z = (iz as f32 - (GRID_SIZE as f32 - 1.0) * 0.5) * 2.0;
                let y = terrain_height(x, z) + 0.05;
                let model = glam::Mat4::from_translation(glam::Vec3::new(x, y, z));
                // 半径 = 2×2m quad 半对角线 √(1²+1²)=√2≈1.414（2026-08-15 修正：
                // 旧 0.5×√2=0.707 低估一半 → 屏幕四角边缘实例被激进剔除穿帮）
                let r = 2.0f32.sqrt();
                self.instance_radii.push(r);
                self.instance_center_x.push(x);
                self.instance_center_y.push(y);
                self.instance_center_z.push(z);
                self.instances.push(InstanceData {
                    model: model.to_cols_array(),
                    tint: [0.7, 0.7, 0.7, 1.0],
                });
            }
        }
        // 并行剔除暂存：一次分配整场容量（每段可见索引上限 = 段实例数）
        self.culled_scratch = vec![0u32; INSTANCE_COUNT as usize];

        // 末尾保留 1 个 slot 存 identity 实例（地形 draw 用，仅创建时写入一次），
        // 元素数由 INSTANCE_BUFFER_ELEMS 单一定义（= 最高槽位 + 1），不再在此抄写副本：
        // 历史上这里是三份互不同步的硬编码，漏改任一份都会让 shader 越界读到全零矩阵、
        // 几何静默消失（无日志、无 VUID）。详见该常量的注释。
        let buffer_elems = INSTANCE_BUFFER_ELEMS;
        let buffer_size = buffer_elems * std::mem::size_of::<InstanceData>() as u64;
        let identity = InstanceData {
            model: glam::Mat4::IDENTITY.to_cols_array(),
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        // 每帧一份 HOST_VISIBLE | HOST_COHERENT buffer，STORAGE_BUFFER（每帧 CPU 直接写）
        let buffer_info = vk::BufferCreateInfo::default()
            .size(buffer_size)
            .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        for _ in 0..self.max_frames_in_flight {
            let buffer = unsafe {
                self.device
                    .create_buffer(&buffer_info, None)
                    .map_err(|e| format!("创建实例 buffer 失败: {}", e))?
            };
            let mem_reqs = unsafe { self.device.get_buffer_memory_requirements(buffer) };
            let memory_type = self.pick_memory_type(mem_reqs, false)?;
            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_reqs.size)
                .memory_type_index(memory_type);
            let memory = unsafe {
                self.device
                    .allocate_memory(&alloc_info, None)
                    .map_err(|e| format!("分配实例 buffer 内存失败: {}", e))?
            };
            unsafe {
                self.device
                    .bind_buffer_memory(buffer, memory, 0)
                    .map_err(|e| format!("绑定实例 buffer 内存失败: {}", e))?;
            }
            let mapped = unsafe {
                self.device
                    .map_memory(memory, 0, buffer_size, vk::MemoryMapFlags::empty())
                    .map_err(|e| format!("映射实例 buffer 失败: {}", e))?
            };
            self.instance_buffers.push(buffer);
            self.instance_buffers_memory.push(memory);
            self.instance_mapped.push(mapped);
            // 写入 identity 实例到槽位 INSTANCE_COUNT（地形 draw 读取，永不覆盖）。
            // 必须写对槽位偏移：旧实现写到了槽位 0，被 cull_and_upload 每帧覆盖，
            // 槽位 65536 恒为未初始化内存 → 地形矩阵塌缩到原点（主 pass 被地面
            // quad 遮住未暴露，阴影 pass 里地形整片消失，阴影图 99.7% 空白）。
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &identity as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        INSTANCE_COUNT as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
                // 枪模 identity 槽（GUN_INSTANCE_INDEX）：主管线 flat=1 纯色路径用
                std::ptr::copy_nonoverlapping(
                    &identity as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        GUN_INSTANCE_INDEX as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
                // 道具 identity 槽（PROP_INSTANCE_INDEX）：identity 矩阵 + Authored 标签。
                // tint.rgb 必须全 1，否则片元的 `input.color = vertexColor × tint.rgb`
                // 会把烘焙好的顶点色整体染色。
                let authored = InstanceData {
                    model: glam::Mat4::IDENTITY.to_cols_array(),
                    tint: [1.0, 1.0, 1.0, crate::engine::geom::Shape::Authored.tag()],
                };
                std::ptr::copy_nonoverlapping(
                    &authored as *const InstanceData as *const u8,
                    (mapped as *mut u8).add(
                        PROP_INSTANCE_INDEX as usize * std::mem::size_of::<InstanceData>(),
                    ),
                    std::mem::size_of::<InstanceData>(),
                );
            }
        }

        if self.mesh_enabled {
            // mesh 路径：地面实例场完全静态（创建后永不修改），初始化时一次性写入全部
            // 槽位（0..INSTANCE_COUNT）到每帧 buffer；此后每帧只上传 marker/NPC/自发光
            // 增量，完全跳过 CPU SIMD 剔除与压缩上传（5.24MB 一次性带宽换每帧 CPU 减负）。
            let bytes = self.instances.len() * std::mem::size_of::<InstanceData>();
            for &mapped in &self.instance_mapped {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.instances.as_ptr() as *const u8,
                        mapped as *mut u8,
                        bytes,
                    );
                }
            }
        }

        log::info!(
            "实例缓冲创建完成: {} 个实例，stride {} 字节，{} 帧双缓冲 HOST_VISIBLE|HOST_COHERENT（每帧压缩上传）",
            INSTANCE_COUNT,
            std::mem::size_of::<InstanceData>(),
            self.max_frames_in_flight
        );
        log::info!("instances={} draw_calls=1", INSTANCE_COUNT);
        Ok(())
    }
}
