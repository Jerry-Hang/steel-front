# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
anchor = "    pub fn pt_set_scene_markers(&mut self, markers: &[WorldMarker]) -> Result<(), String> {"
add_head = """    /// PT 降噪后处理初始化（2026-09-02：SM 满载 + 去噪！）
    pub fn init_pt_denoise(&mut self, w: u32, h: u32) -> Result<(), String> {
        let dn_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("rt").join("denoise.spv");
        let spv = std::fs::read(dn_path).map_err(|e| format!("dn read: {e}"))?;
        let wv: Vec<u32> = spv.chunks_exact(4).map(|c| u32::from_le_bytes([c[0],c[1],c[2],c[3]])).collect();
        let dn_module = self.create_shader_module(&wv).map_err(|e| format!("dn m: {e}"))?;
        let in_layout = vk::DescriptorSetLayoutBinding::default().binding(0).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let out_layout = vk::DescriptorSetLayoutBinding::default().binding(1).descriptor_type(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(1).stage_flags(vk::ShaderStageFlags::COMPUTE);
        let sl = unsafe { self.device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&[in_layout, out_layout]), None) }.map_err(|e| format!("dn sl: {e}"))?;
        let pl = unsafe { self.device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&[sl]), None) }.map_err(|e| format!("dn pl: {e}"))?;
        let stage = vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::COMPUTE).module(dn_module).name(c"main");
        let dn_pipeline = unsafe { self.device.create_compute_pipelines(vk::PipelineCache::null(), &[vk::ComputePipelineCreateInfo::default().stage(stage).layout(pl)], None) }.map_err(|e| format!("dn pipe {:?}", e.1))?[0];
        let dn_img_info = vk::ImageCreateInfo::default().image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_UNORM).extent(vk::Extent3D { width: w, height: h, depth: 1 })
            .mip_levels(1).array_layers(1).samples(vk::SampleCountFlags::TYPE_1)
            .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE).initial_layout(vk::ImageLayout::UNDEFINED);
        let dn_img = unsafe { self.device.create_image(&dn_img_info, None) }.map_err(|e| format!("dn img: {e}"))?;
        let dn_req = unsafe { self.device.get_image_memory_requirements(dn_img) };
        let dn_type = self.pick_memory_type(dn_req, true).map_err(|e| format!("dn mt: {e}"))?;
        let dn_mem = unsafe { self.device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(dn_req.size).memory_type_index(dn_type), None) }.map_err(|e| format!("dn mm: {e}"))?;
        unsafe { self.device.bind_image_memory(dn_img, dn_mem, 0) }.map_err(|e| format!("dn bi: {e}"))?;
        let dn_view = unsafe { self.device.create_image_view(&vk::ImageViewCreateInfo::default().image(dn_img).view_type(vk::ImageViewType::TYPE_2D).format(vk::Format::B8G8R8A8_UNORM)
            .subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 }), None) }.map_err(|e| format!("dn iv: {e}"))?;
        let pool = unsafe { self.device.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&[
            vk::DescriptorPoolSize::default().ty(vk::DescriptorType::STORAGE_IMAGE).descriptor_count(2)]), None) }.map_err(|e| format!("dn dp: {e}"))?;
        let dset = unsafe { self.device.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[sl])) }.map_err(|e| format!("dn ds: {e}"))?[0];
        let in_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: self.pt_view, image_layout: vk::ImageLayout::GENERAL };
        let out_desc = vk::DescriptorImageInfo { sampler: vk::Sampler::null(), image_view: dn_view, image_layout: vk::ImageLayout::GENERAL };
        let mut w1 = vk::WriteDescriptorSet::default(); w1.dst_set = dset; w1.dst_binding = 0; w1.descriptor_type = vk::DescriptorType::STORAGE_IMAGE; w1.descriptor_count = 1; w1.p_image_info = &in_desc;
        let mut w2 = vk::WriteDescriptorSet::default(); w2.dst_set = dset; w2.dst_binding = 1; w2.descriptor_type = vk::DescriptorType::STORAGE_IMAGE; w2.descriptor_count = 1; w2.p_image_info = &out_desc;
        unsafe { self.device.update_descriptor_sets(&[w1, w2], &[]); }
        self.dn_pipeline = dn_pipeline;
        self.dn_layout = pl;
        self.dn_dset = dset;
        self.dn_dset_pool = pool;
        self.dn_setl = sl;
        self.dn_img = dn_img;
        self.dn_img_mem = dn_mem;
        self.dn_view = dn_view;
        self.dn_module = dn_module;
        Ok(())
    }

""" + anchor
if anchor in s and 'pub fn init_pt_denoise' not in s:
    s = s.replace(anchor, add_head, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('init_pt_denoise inserted')
else:
    print('miss2')
