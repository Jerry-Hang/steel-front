// 由 src/engine/renderer.rs 拆出（纯移动：块内容逐字节一致，判据 tools/refactor_move_check.py）。
// 本模块是 `renderer` 的后代 ⇒ 照旧看得见 Renderer 的私有字段、私有方法与文件作用域私有项；
// `use super::*;` 把上一层的名字（含它的 use 绑定）转发进来。
use super::*;

impl Renderer {
    pub(crate) fn record_command_buffer(
        &self,
        command_buffer: vk::CommandBuffer,
        image_index: usize,
        near_count: u32,
        far_count: u32,
        terrain_lod: usize,
    ) -> Result<(), String> {
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty());

        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(|e| format!("开始命令缓冲失败: {}", e))?;
        }

        // ---- 阴影 pass：depth-only 渲光空间深度，供主 pass 3x3 PCF 采样 ----
        // （mesh 路径已冻结，shadow 只服务传统 VERTEX 几何；mesh 模式 near=INSTANCE_COUNT
        //   地面实例静态上传，marker/NPC/自发光照常上传，同一槽位布局可复用）
        //
        // 默认（拆分）：**两张图** —— 动态图每帧只画 NPC/士兵（便宜、影子实时），
        // 静态图每隔 `shadow_static_every` 帧画一次地形/地面场/marker/道具。
        // 旧行为（`RV3D_NO_SHADOW_SPLIT=1`）：单张静态图，两类一起画，按 `shadow_frame` 隔帧。
        if self.shadow_split {
            if self.shadow_frame {
                self.record_shadow_pass(
                    command_buffer,
                    near_count,
                    far_count,
                    terrain_lod,
                    self.shadow_dyn_image,
                    self.shadow_dyn_framebuffer,
                    false,
                    true,
                )?;
            }
            if self.shadow_static_frame {
                self.record_shadow_pass(
                    command_buffer,
                    near_count,
                    far_count,
                    terrain_lod,
                    self.shadow_image,
                    self.shadow_framebuffer,
                    true,
                    false,
                )?;
            }
        } else if self.shadow_frame {
            self.record_shadow_pass(
                command_buffer,
                near_count,
                far_count,
                terrain_lod,
                self.shadow_image,
                self.shadow_framebuffer,
                true,
                true,
            )?;
        }

        // clear values 按 attachment 索引寻址：0=MSAA 颜色(CLEAR)、1=resolve(DONT_CARE，
        // 值被忽略但占位保证索引正确)、2=深度(CLEAR)。旧实现只有 2 个元素 → 深度清除值
        // 越界读取 → 深度缓冲未清除（垃圾）→ 深度测试随机失败：地面/障碍大面积消失。
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: if self.void_mode {
                        [1.0, 1.0, 1.0, 1.0] // 检视模式：白色背景，便于对比透视
                    } else {
                        // 白天天空（线性 RGB → sRGB 约浅蓝）；城市地图配套（2026-08-21）
                        [0.24, 0.36, 0.60, 1.0]
                    },
                },
            },
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            },
        ];

        let render_pass_begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[image_index])
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.swapchain_extent,
            })
            .clear_values(&clear_values);

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin_info,
                vk::SubpassContents::INLINE,
            );
        }

        // 动态 viewport/scissor：每帧按当前 swapchain_extent 重设（resize 后自动适配，
        // 2026-08-15 修复全屏/窗口变化后画面卡左上角）
        let vp = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.swapchain_extent.width as f32)
            .height(self.swapchain_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let sc = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(self.swapchain_extent);
        unsafe {
            self.device.cmd_set_viewport(command_buffer, 0, &[vp]);
            self.device.cmd_set_scissor(command_buffer, 0, &[sc]);
        }

        unsafe {
            self.device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
        }

        // ---- 绑定 Descriptor Set ----
        // 本帧写入的 UBO 与 instance buffer 都是 current_frame 对应的 slot，
        // 因此必须绑定 descriptor_sets[current_frame]（image_index 与帧 slot 无关）。
        let descriptor_sets = [self.descriptor_sets[self.current_frame]];
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &descriptor_sets,
                &[],
            );
        }

        // 地形 draw call（非实例，instance_index = 65536 读保留 identity 实例；
        // 每帧按 LOD 选择绘制 3 级网格之一，mesh.index_count 随密度变化）
        // 虚空检视模式：不绘制地形（仅枪模）
        //
        // 🔴 2026-09-26 新增 `RV3D_NO_TERRAIN=1`（与 `RV3D_NO_PROPS` / `RV3D_NO_SHADOW` 同一套
        // 对照开关惯例）：**帧预算地图里"关掉一切可关的"仍有 5.4ms，而它只可能是地形网格 +
        // 枪模 + HUD**。没有这条开关就只能猜 —— 而"猜瓶颈"在本仓是明令禁止的（第 37 轮起）。
        if !self.void_mode && std::env::var("RV3D_NO_TERRAIN").is_err() {
        if let Some(mesh) = self.terrain_lods.get(terrain_lod) {
            let terrain_vertex_buffers = [mesh.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &terrain_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    mesh.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    mesh.index_count,
                    1,
                    0,
                    0,
                    INSTANCE_COUNT,
                );
            }
        }
        }

        if self.mesh_enabled {
            // ---- 网格着色器路径（VK_EXT_mesh_shader）：逐实例 GPU 视锥剔除 + 顶点变换 ----
            // 地面实例场静态一次性上传（槽位 0..INSTANCE_COUNT）；marker/NPC/自发光每帧
            // 顺序上传到各自 BASE 槽位（shader 按距离自选立方体 / 远档十字 quad 几何）。
            let mesh = self
                .mesh_shader
                .as_ref()
                .expect("mesh_enabled=true 但 vkCmdDrawMeshTasksEXT 加载器缺失");
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.mesh_pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
            }
            if !self.void_mode {
                self.draw_mesh_range(command_buffer, mesh, 0, INSTANCE_COUNT);
            }
            self.draw_mesh_range(
                command_buffer,
                mesh,
                MARKER_SLOT_BASE,
                self.last_marker_near + self.last_marker_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_SLOT_BASE,
                self.last_npc_box_near + self.last_npc_box_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_CYL_SLOT_BASE,
                self.last_npc_cyl_near + self.last_npc_cyl_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                NPC_SPH_SLOT_BASE,
                self.last_npc_sph_near + self.last_npc_sph_far,
            );
            self.draw_mesh_range(
                command_buffer,
                mesh,
                EMISSIVE_SLOT_BASE,
                self.last_emissive_near + self.last_emissive_far,
            );
        } else {
        // 近档地面 draw call：平铺 quad 几何（无侧壁），实例区从 0 开始
        if near_count > 0 {
            let vertex_buffers = [self.ground_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.ground_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    GROUND_INDICES.len() as u32,
                    near_count,
                    0,
                    0,
                    0,
                );
            }
        }

        // 远档地面 draw call：同样平铺 quad 几何，实例区偏移 = near_count（[近档][远档] 连续排布）
        if far_count > 0 {
            let far_vertex_buffers = [self.ground_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &far_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.ground_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    GROUND_INDICES.len() as u32,
                    far_count,
                    0,
                    0,
                    near_count,
                );
            }
        }

        // ---- 世界障碍 marker draw（复用同一 pipeline 与几何，实例槽从 MARKER_SLOT_BASE 起）----
        if self.last_marker_near > 0 {
            let marker_vertex_buffers = [self.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &marker_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    INDICES.len() as u32,
                    self.last_marker_near,
                    0,
                    0,
                    MARKER_SLOT_BASE,
                );
            }
        }
        if self.last_marker_far > 0 {
            let far_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &far_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_marker_far,
                    0,
                    0,
                    MARKER_SLOT_BASE + self.last_marker_near,
                );
            }
        }

        // ---- NPC 士兵段 draw（人体三几何：盒体躯干/圆柱四肢/球体头，各自独立
        //      几何与实例槽区；每区按距离分近档（对应几何）+ 远档（十字 quad））----
        // 盒体区（躯干/脚/枪）
        if self.last_npc_box_near > 0 {
            let npc_vertex_buffers = [self.vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    INDICES.len() as u32,
                    self.last_npc_box_near,
                    0,
                    0,
                    NPC_SLOT_BASE,
                );
            }
        }
        if self.last_npc_box_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_box_far,
                    0,
                    0,
                    NPC_SLOT_BASE + self.last_npc_box_near,
                );
            }
        }
        // 圆柱区（四肢）
        if self.last_npc_cyl_near > 0 {
            let npc_vertex_buffers = [self.cylinder_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.cylinder_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.cylinder_index_count,
                    self.last_npc_cyl_near,
                    0,
                    0,
                    NPC_CYL_SLOT_BASE,
                );
            }
        }
        if self.last_npc_cyl_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_cyl_far,
                    0,
                    0,
                    NPC_CYL_SLOT_BASE + self.last_npc_cyl_near,
                );
            }
        }
        // 球体区（头）
        if self.last_npc_sph_near > 0 {
            let npc_vertex_buffers = [self.sphere_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.sphere_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.sphere_index_count,
                    self.last_npc_sph_near,
                    0,
                    0,
                    NPC_SPH_SLOT_BASE,
                );
            }
        }
        if self.last_npc_sph_far > 0 {
            let npc_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &npc_vertex_buffers, &offsets);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_npc_sph_far,
                    0,
                    0,
                    NPC_SPH_SLOT_BASE + self.last_npc_sph_near,
                );
            }
        }

        // ---- 自发光实体 draw（爆炸闪光等；复用同一 pipeline，实例槽从 EMISSIVE_SLOT_BASE 起，
        //      shader 对槽位 >= EMISSIVE_INSTANCE_BASE 的实例走自发光直出）----
        // 2026-08-15：改用 UV 球体几何（爆炸球形扩散，不再是一整块立方体）
        if self.last_emissive_near > 0 {
            let emissive_vertex_buffers = [self.sphere_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &emissive_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.sphere_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.sphere_index_count,
                    self.last_emissive_near,
                    0,
                    0,
                    EMISSIVE_SLOT_BASE,
                );
            }
        }
        if self.last_emissive_far > 0 {
            let emissive_vertex_buffers = [self.far_vertex_buffer];
            let offsets = [0u64];
            unsafe {
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &emissive_vertex_buffers,
                    &offsets,
                );
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.far_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    FAR_INDICES.len() as u32,
                    self.last_emissive_far,
                    0,
                    0,
                    EMISSIVE_SLOT_BASE + self.last_emissive_near,
                );
            }
        }
        }

        // ---- GLB 道具：按空间分桶逐桶视锥剔除后绘制。
        //      走主管线（**已开深度测试**），所以道具之间、道具与地形之间遮挡正确。
        //      identity 实例取 PROP_INSTANCE_INDEX：位姿已在 CPU 烘进顶点，GPU 侧不需要
        //      逐实例矩阵；该槽 tint.w=Shape::Authored.tag() 让片元跳过程序化立面加工。
        //      分桶动机与实测收益见 `engine::props::merge_binned`（道具曾占整帧约 40%）。
        if self.prop_index_count > 0 && self.prop_vertex_count > 0 && !self.prop_bins.is_empty() {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
                let pvb = [self.prop_vertex_buffer];
                let poff = [0u64];
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &pvb, &poff);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.prop_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                // 逐桶球-视锥测试，只发可见桶的那段索引。所有桶共用同一份 VBO/IBO 与
                // 同一个 identity 实例，所以这里只换 firstIndex/indexCount，
                // 不需要任何重新绑定或重传。
                // margin 2m：桶边界上的建筑不该在转视角时逐帧抖动进出。
                // RV3D_ONE_PROP_DRAW=1：A/B（第 43 轮）—— 不分桶，整份索引一次画完。
                // 用来二选一：**1 个 draw 也慢 ⇒ 顶点/片元着色本身贵**；
                // **1 个 draw 快很多 ⇒ 逐 draw 的 state 开销贵**（28 次切换）。
                // 前提：顶点数/三角形数/draw call 数/填充率四个维度都已排除对不上这 3.7ms。
                let one_draw = std::env::var("RV3D_ONE_PROP_DRAW").is_ok();
                // 距离直方图只在 RV3D_PROP_STATS=1 时统计（默认零成本）：
                // 回答"按距离剔除还有多少可剔"——`noprops` 实测 +37%（2026-09-26 新尺子），
                // 道具仍是最大单项，但**剔除半径**该定在哪要靠这个分布说话。
                let stats_on = std::env::var("RV3D_PROP_STATS").is_ok();
                let mut dist_bins = [0u32; 3]; // <200m / 200..400m / >=400m（XZ 平面距离）
                let mut drawn_bins = 0u32;
                let mut drawn_tris = 0u32;
                let mut drawn_vert_span = 0u64;
                if one_draw {
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        self.prop_index_count,
                        1,
                        0,
                        0,
                        PROP_INSTANCE_INDEX,
                    );
                    drawn_bins = 1;
                    drawn_tris = self.prop_index_count / 3;
                    drawn_vert_span = self.prop_vertex_count as u64;
                } else {
                    for bin in &self.prop_bins {
                        if !crate::engine::props::bin_visible(bin, &self.frame_frustum, 2.0) {
                            continue;
                        }
                        drawn_bins += 1;
                        drawn_tris += bin.index_count / 3;
                        drawn_vert_span += (bin.max_vertex - bin.min_vertex + 1) as u64;
                        if stats_on {
                            let dx = bin.center[0] - self.frame_cam_pos.x;
                            let dz = bin.center[2] - self.frame_cam_pos.z;
                            let d = (dx * dx + dz * dz).sqrt();
                            let slot = if d < 200.0 {
                                0
                            } else if d < 400.0 {
                                1
                            } else {
                                2
                            };
                            dist_bins[slot] += 1;
                        }
                        self.device.cmd_draw_indexed(
                            command_buffer,
                            bin.index_count,
                            1,
                            bin.first_index,
                            0,
                            PROP_INSTANCE_INDEX,
                        );
                    }
                }
                // 🪖 士兵 GLB（2026-09-13）：**一次 draw 画完所有实例**。
                //
                // 与道具/箱体的关键差别：这里是**真正的实例化** —— 网格上传一次，
                // 每个 NPC 只占一个实例矩阵。`cmd_draw_indexed` 的实例数（第 2 个参数）
                // 就是为这个准备的，枪模已经在用同一条路（只是它固定传 1）。
                //
                // 管线用 `self.pipeline`：它就是传统 VERTEX 管线（`vs_main`/`fs_main`），
                // `depth_test` 是开的 —— 世界里的士兵必须被墙挡住（枪那条是 OFF，
                // 因为它要恒在 HUD 之上）。这个管线在 mesh 可用时同样被无条件创建，
                // 道具/地面也一直在用它。
                //
                // ⚠️ `soldier_drawn` 由 `upload_soldiers` 写；为 0 时整段跳过，
                // 于是"没上传网格"时行为与改动前**逐字节一致**（NPC 仍只有 18 段箱体）。
                if self.soldier_drawn > 0
                    && self.soldier_index_count > 0
                    && self.soldier_vertex_buffer != vk::Buffer::null()
                {
                    self.device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipeline,
                    );
                    let svb = [self.soldier_vertex_buffer];
                    let soff = [0u64];
                    self.device
                        .cmd_bind_vertex_buffers(command_buffer, 0, &svb, &soff);
                    self.device.cmd_bind_index_buffer(
                        command_buffer,
                        self.soldier_index_buffer,
                        0,
                        vk::IndexType::UINT32,
                    );
                    self.device.cmd_draw_indexed(
                        command_buffer,
                        self.soldier_index_count,
                        self.soldier_drawn,
                        0,
                        0,
                        SOLDIER_INSTANCE_BASE,
                    );
                }
                // 第②条判据（RV3D_PROP_STATS=1）：每帧实际提交了多少桶/三角形。
                // 存在的理由：道具是已知最大单项（关掉道具 fps 68.8→184.7），
                // 但机制一直靠猜。先回答"到底提交了多少"，再看是**剔除粒度**问题
                // （提交量远超屏幕能分辨的量）还是**逐 draw call 开销**（量不大但时间高）。
                {
                    use std::sync::atomic::{AtomicU32, Ordering};
                    static TICK: AtomicU32 = AtomicU32::new(0);
                    if stats_on && TICK.fetch_add(1, Ordering::Relaxed) % 120 == 0 {
                        let max_bin = self
                            .prop_bins
                            .iter()
                            .map(|b| b.index_count / 3)
                            .max()
                            .unwrap_or(0);
                        log::info!(
                            "propdraw: 桶 {drawn_bins}/{} 可见；提交三角形 {drawn_tris}；单桶最大 {max_bin}；顶点区间合计 {drawn_vert_span}（顶点总数 {}）；距离 <200m {} / 200-400m {} / >=400m {}",
                            self.prop_bins.len(),
                            self.prop_vertex_count,
                            dist_bins[0],
                            dist_bins[1],
                            dist_bins[2]
                        );
                    }
                }
            }
        }

        // ---- 第一人称枪模（程序化高模，2026-08-16）：identity 实例（GUN_INSTANCE_INDEX
        //      → inst.model = 单位阵，顶点即世界空间，main.rs 已烘焙 view⁻¹×锚点）。
        //      走 `gun_pipeline`（depth_test=OFF 且不写深度）→ 枪模恒可见、也不会挡住 HUD。
        //      2026-09-04：主管线开了深度测试，枪模若继续共用会被它前面的墙裁掉，
        //      所以这里必须切到独立管线，而不是继续靠主管线的宽松 depth 状态。
        if self.gun_index_count > 0 && self.gun_vertex_count > 0 {
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.gun_pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &descriptor_sets,
                    &[],
                );
                let gun_vb = [self.gun_vertex_buffer];
                let gun_off = [0u64];
                self.device.cmd_bind_vertex_buffers(command_buffer, 0, &gun_vb, &gun_off);
                self.device.cmd_bind_index_buffer(
                    command_buffer,
                    self.gun_index_buffer,
                    0,
                    vk::IndexType::UINT32,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    self.gun_index_count,
                    1,
                    0,
                    0,
                    GUN_INSTANCE_INDEX,
                );
            }
        }

        // ---- HUD 覆盖层：自包含 pipeline 与顶点缓冲，追加在主 pass 末尾 ----
        // 🧊 有磨砂玻璃面板时**这一处不画**：玻璃要采"面板背后的画面"，而主 pass 里
        // 交换链是 MSAA 解析目标、根本不能采样自己 ⇒ 那种帧改走 overlay pass
        // （先 blit 出模糊图，再画 HUD，见 `record_command_buffer` 末尾）。
        if self.hud_vertex_count > 0
            && self.hud_pipeline != vk::Pipeline::null()
            && !self.hud_has_glass
        {
            let hud_vertex_buffers = [self.hud_vertex_buffer];
            let hud_offsets = [0u64];
            let hud_sets = [self.hud_glass_set];
            unsafe {
                self.device.cmd_bind_pipeline(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.hud_pipeline,
                );
                // 着色器静态引用 binding 0/1（见 `hud_pipeline_layout`）⇒ 这一处也得绑。
                self.device.cmd_bind_descriptor_sets(
                    command_buffer,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.hud_pipeline_layout,
                    0,
                    &hud_sets,
                    &[],
                );
                self.device.cmd_bind_vertex_buffers(
                    command_buffer,
                    0,
                    &hud_vertex_buffers,
                    &hud_offsets,
                );
                self.device.cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
            }
        }

        unsafe {
            self.device.cmd_end_render_pass(command_buffer);
        }

        // ===== 🧊 磨砂玻璃（未结案 #19）：主 pass 之后、PT 之前，把交换链降采样成模糊图 =====
        //
        // 为什么必须在这里、而且必须换 pass 画 HUD：主 pass 的 HUD 是画在 **MSAA 解析目标**
        // 上的，而"面板背后的画面"要么是它自己（同一 pass 内不能采样）、要么是那张 MSAA 图
        // （普通 sampler 不能采多采样图）。⇒ 唯一干净的做法 = 主 pass 结束（此时交换链已是
        // 单采样、`finalLayout = PRESENT_SRC_KHR`）→ blit 出模糊图 → 用 **overlay HUD pass**
        // 把 HUD 画在模糊图之上。overlay pass 与它的 barrier 顺序**照抄 PT 通路**
        // （那条路已经跑了很久、VUID=0），raster 路径只是第一次走它。
        if self.hud_has_glass && self.hud_vertex_count > 0 {
            let sw_img = self.swapchain_images[image_index as usize];
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };
            unsafe {
                // ① 交换链 PRESENT_SRC → TRANSFER_SRC
                let to_src = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_READ)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(range);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[to_src],
                );
                // ② 模糊图 SHADER_READ_ONLY → TRANSFER_DST（它上一帧被采过，所以不是 UNDEFINED）
                let blur_to_dst = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_READ)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.menu_blur_image)
                    .subresource_range(range);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[blur_to_dst],
                );
                // ③ blit（LINEAR = 8×8 盒式平均，这**就是**模糊本身）
                let blit = vk::ImageBlit::default()
                    .src_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: self.swapchain_extent.width as i32,
                            y: self.swapchain_extent.height as i32,
                            z: 1,
                        },
                    ])
                    .dst_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .dst_offsets([
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: MENU_BLUR_W as i32,
                            y: MENU_BLUR_H as i32,
                            z: 1,
                        },
                    ]);
                self.device.cmd_blit_image(
                    command_buffer,
                    sw_img,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    self.menu_blur_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    vk::Filter::LINEAR,
                );
                // ④ 模糊图 → SHADER_READ_ONLY（HUD 片元要采它）
                let blur_to_read = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.menu_blur_image)
                    .subresource_range(range);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[blur_to_read],
                );
                // ⑤ 交换链 TRANSFER_SRC → COLOR_ATTACHMENT（overlay HUD pass 的 initialLayout）
                let to_color = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(range);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[to_color],
                );
                // ⑥ overlay HUD pass（load=LOAD 保住场景 + 模糊图之上的 HUD）
                if self.hud_render_pass != vk::RenderPass::null() {
                    self.device.cmd_begin_render_pass(
                        command_buffer,
                        &vk::RenderPassBeginInfo::default()
                            .render_pass(self.hud_render_pass)
                            .framebuffer(self.hud_framebuffers[image_index as usize])
                            .render_area(vk::Rect2D {
                                offset: vk::Offset2D { x: 0, y: 0 },
                                extent: self.swapchain_extent,
                            })
                            .clear_values(&[]),
                        vk::SubpassContents::INLINE,
                    );
                    let hud_vb = [self.hud_vertex_buffer];
                    let hud_off = [0u64];
                    let hud_sets = [self.hud_glass_set];
                    self.device.cmd_bind_pipeline(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.hud_overlay_pipeline,
                    );
                    self.device.cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.hud_pipeline_layout,
                        0,
                        &hud_sets,
                        &[],
                    );
                    self.device.cmd_bind_vertex_buffers(command_buffer, 0, &hud_vb, &hud_off);
                    self.device
                        .cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
                    self.device.cmd_end_render_pass(command_buffer);
                }
            }
        }
        if self.pt_live_enabled && self.pt_resident.is_some() {
            let sw_img = self.swapchain_images[image_index as usize];
            // 与 init_pt_resident 创建的图像同尺寸（硬编码会与新分辨率错配）
            let (pw, ph) = self.pt_size;
            unsafe {
                // 取景/光照变化 => 清空重开累积（不同视角样本混在一起会拖影）
                let sig = self.pt_params.signature();
                // 2026-09-01：sig 已量化 0.5m 位移——只有大于该步幅才重置（指数平均吸收细微移动）
                if sig != self.pt_view_sig.get() {
                    self.pt_view_sig.set(sig);
                    self.pt_reset.set(true);
                    self.pt_frame.set(0);
                }
                let accumulating = self.pt_frame.get() < self.pt_spp_target;
                if accumulating {
                    // 主图像每帧整体重写 => 允许 UNDEFINED 丢弃；累积图像必须 GENERAL->GENERAL 保内容
                    let pt_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.pt_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    let acc_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.pt_acc)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::COMPUTE_SHADER, vk::DependencyFlags::empty(), &[], &[], &[pt_bar, acc_bar]);
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::COMPUTE, self.pt_pipeline);
                    self.device.cmd_bind_descriptor_sets(command_buffer, vk::PipelineBindPoint::COMPUTE, self.pt_layout, 0, &[self.pt_dset], &[]);
                    let pc = self.pt_params.pack(
                        pw, ph,
                        self.pt_frame.get(),
                        self.pt_reset.get(),
                        self.pt_spp_target,
                        // 运动量：相机位移与朝向变化速度 → 0..1（移动/跳跃 → 高 spp + 短时域）
                        {
                            let c0 = self.pt_move_base_cam.get();
                            let f0 = self.pt_move_base_fwd.get();
                            let c1 = self.pt_params.cam.to_array();
                            let f1 = self.pt_params.fwd.to_array();
                            let mut d = 0.0f32;
                            for k in 0..3 { let dc = c1[k] - c0[k]; d += dc * dc; let df = f1[k] - f0[k]; d += df * df * 36.0; }
                            d = d.sqrt();
                            self.pt_move_base_cam.set(c1);
                            self.pt_move_base_fwd.set(f1);
                            (d * 20.0).min(1.0)
                        },
                        // 🏢 盒体三角形边界（道具路径分流判据，见 pt_panorama.glsl 的 pc.g）
                        (self.pt_box_count * 12) as u32,
                    );
                    self.pt_reset.set(false);
                    self.device.cmd_push_constants(
                        command_buffer,
                        self.pt_layout,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        bytemuck_bytes(&pc),
                    );
                    self.device.cmd_dispatch(command_buffer, (pw + 7) / 8, (ph + 7) / 8, 1);
                    self.pt_frame.set(self.pt_frame.get() + 1);
                }
                // PT 写完成 -> Transfer 读
                let pt_bar2 = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE).dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL).new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.pt_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::COMPUTE_SHADER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[pt_bar2]);
                // swapchain -> TRANSFER_DST
                let sw_bar = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::NONE).dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED).new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[sw_bar]);
                // blit PT -> swapchain
                // 🔴 目标范围必须取**活的**交换链尺寸。这里曾写死 2560x1600，于是
                // "默认窗口尺寸能跑、别的尺寸越界" —— 而默认尺寸正是平时跑验证用的那一个，
                // 所以它躲过了此前每一轮验证。真机复现 = `scripts\run_resize_probe.ps1 -PT`
                // （窗口改到 1280x720 等尺寸后立刻 `VUID-vkCmdBlitImage-dstOffsets-00203`）。
                // 判据 = 源码检查 `blit_regions_never_hardcode_pixel_extents`。
                // ⚠️ PT 图像是 init 时定尺寸的（`pt_size`，不随交换链重建变化）⇒ 缩放在这里发生：
                // 换个宽高比的窗口只会把 PT 参照画面拉伸，不会错位。
                let blit = vk::ImageBlit::default()
                    .src_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .src_offsets([vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: pw as i32, y: ph as i32, z: 1 }])
                    .dst_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .dst_offsets([vk::Offset3D { x: 0, y: 0, z: 0 }, vk::Offset3D { x: self.swapchain_extent.width as i32, y: self.swapchain_extent.height as i32, z: 1 }]);
                self.device.cmd_blit_image(command_buffer, self.pt_img, vk::ImageLayout::GENERAL, sw_img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[blit], vk::Filter::NEAREST);
                // swapchain -> PRESENT_SRC
                let sw_back = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::MEMORY_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL).new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(sw_img)
                    .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[sw_back]);
                // 2026-09-01：HUD/UI 重绘在 PT 之上（load=LOAD 保留 PT 画面！）
                if self.hud_render_pass != vk::RenderPass::null() && self.hud_vertex_count > 0 {
                    let hud_bar = vk::ImageMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::MEMORY_READ).dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                        .old_layout(vk::ImageLayout::PRESENT_SRC_KHR).new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(sw_img)
                        .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 });
                    self.device.cmd_pipeline_barrier(command_buffer, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, vk::DependencyFlags::empty(), &[], &[], &[hud_bar]);
                    self.device.cmd_begin_render_pass(command_buffer, &vk::RenderPassBeginInfo::default()
                        .render_pass(self.hud_render_pass)
                        .framebuffer(self.hud_framebuffers[image_index as usize])
                        .render_area(vk::Rect2D { offset: vk::Offset2D { x: 0, y: 0 }, extent: self.swapchain_extent }),
                        vk::SubpassContents::INLINE);
                    // ⚠ 必须绑 **overlay 专用**管线：overlay pass 是 1 采样、无深度，
                    // 而 `hud_pipeline` 是给主 pass（MSAA + 深度）建的 —— 绑错就是
                    // `VUID-vkCmdDraw-renderPass-02684`（管线与 render pass 不兼容 = UB）
                    self.device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, self.hud_overlay_pipeline);
                    // 🧊 `hud_pipeline_layout` 现在还带一套 HUD 玻璃 set（着色器静态引用
                    // binding 0/1）⇒ **PT 这条 HUD 绘制也必须绑它**，否则就是
                    // "描述符集未绑定"类 VUID。PT 模式下不会走玻璃分支（见 `set_hud_quads`
                    // 里的 `glass_on`：PT 时把标志位写成 0），所以采到的内容不会被用到。
                    let hud_sets = [self.hud_glass_set];
                    self.device.cmd_bind_descriptor_sets(
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.hud_pipeline_layout,
                        0,
                        &hud_sets,
                        &[],
                    );
                    let vb = [self.hud_vertex_buffer];
                    let offs = [0u64];
                    self.device.cmd_bind_vertex_buffers(command_buffer, 0, &vb, &offs);
                    self.device.cmd_draw(command_buffer, self.hud_vertex_count, 1, 0, 0);
                    self.device.cmd_end_render_pass(command_buffer);
                    // ⚠ 这里**不再**补 `COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR` 的 barrier：
                    // render pass 的 `finalLayout` 已经做了那次转换，再补一条就是"从已经变成
                    // PRESENT_SRC 的图再转一次 COLOR_ATTACHMENT_OPTIMAL" ⇒
                    // `VUID-VkImageMemoryBarrier-oldLayout-01197`（2026-09-15 删掉的就是它）。
                }
            }
        }

        unsafe {
            self.device
                .end_command_buffer(command_buffer)
                .map_err(|e| format!("结束命令缓冲失败: {}", e))?;
        }
        Ok(())
    }
}
