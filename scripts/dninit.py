# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 在 init_pt_resident 结尾（Ok(()) 前）插 dn init——找 init 的 "        Ok(())\n    }\n\n    pub fn pt_set_scene_markers" 锚
anchor = "        Ok(())\n    }\n\n    pub fn pt_set_scene_markers"
add = """        // 2026-09-02 降噪后处理（SM 满载 + 消噪！）
        {
            let dn_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("rt").join("denoise.spv");
            let spv = std::fs::read(dn_path).unwrap_or_default();
            let dn_module = self.create_shader_module(&{
                let mut w = Vec::new();
                for c in spv.chunks_exact(4) { w.push(u32::from_le_bytes([c[0],c[1],c[2],c[3]])); }
                w
            }).map_err(|e| format!("dn m: {e}"))?;
            let in_layout = vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
            let out_layout = vk::DescriptorSetLayoutBinding::default().binding(1).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
            let sl = self.device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&[in_layout, out_layout]), None).map_err(|e| format!("dn sl: {e}"))?;
            let pl = self.device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&[sl]), None).map_err(|e| format!("dn pl: {e}"))?;
            let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(dn_module).name(c"main");
            let dn_pipeline = self.device.create_compute_pipelines(vk::PipelineCache::null(), &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(pl)], None).map_err(|e| format!("dn pipe {:?}", e.1))?[0];
            // dn 输出图
            let dn_img_info = vk::ImageCreateInfo::default().image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::B8G8R8A8_UNORM).extent(vk::Extent3D { width: w, height: h, depth: 1 })
                .mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
                .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
            let dn_img = self.device.create_image(&dn_img_info, None).map_err(|e| format!("dn img: {e}"))?;
            let dn_req = self.device.get_image_memory_requirements(dn_img);
            let dn_type = self.pick_memory_type(dn_req, true).map_err(|e| format!("dn mt: {e}"))?;
            let dn_mem = self.device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(dn_req.size).memory_type_index(dn_type), None).map_err(|e| format!("dn mm: {e}"))?;
            self.device.bind_image_memory(dn_img, dn_mem, 0).map_err(|e| format!("dn bi: {e}"))?;
            let dn_view = self.device.create_image_view(&vk::ImageViewCreateInfo::default().image(dn_img).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::B8G8R8A8_UNORM)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 }), None).map_err(|e| format!("dn iv: {e}"))?;
            let pool = self.device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&[
                vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2)]), None).map_err(|e| format!("dn dp: {e}"))?;
            let dset = self.device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[sl])).map_err(|e| format!("dn ds: {e}"))?[0];
            let in_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: view, image_layout: vk::ImageLayout::GENERAL };
            let out_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: dn_view, image_layout: vk::ImageLayout::GENERAL };
            let mut w1 = vk::WriteDescriptorSet::default(); w1.dst_set = dset; w1.dst_binding = 0; w1.descriptor_type = vk::DescriptorType::STORAGE_IMAGE; w1.descriptor_count = 1; w1.p_image_info = &in_desc;
            let mut w2 = vk::WriteDescriptorSet::default(); w2.dst_set = dset; w2.dst_binding = 1; w2.descriptor_type = vk::DescriptorType::STORAGE_IMAGE; w2.descriptor_count = 1; w2.p_image_info = &out_desc;
            self.device.update_descriptor_sets(&[w1, w2], &[]);
            self.dn_pipeline = dn_pipeline;
            self.dn_layout = pl;
            self.dn_dset = dset;
            self.dn_dset_pool = pool;
            self.dn_setl = sl;
            self.dn_img = dn_img;
            self.dn_img_mem = dn_mem;
            self.dn_view = dn_view;
            self.dn_module = dn_module;
        }

""" + anchor
if anchor in s and 'dn_pipeline' in s:
    s = s.replace(anchor, add, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('dn init inserted')
else:
    print('miss')
