# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
# 字段（dn_pipeline/dn_layout/dn_dset……简化：降噪用独立 pipeline+dset（每帧动态？一次建！）
s = s.replace("    hud_render_pass: vk::RenderPass,", "    hud_render_pass: vk::RenderPass,\n    dn_pipeline: vk::Pipeline,\n    dn_layout: vk::PipelineLayout,\n    dn_dset: vk::DescriptorSet,\n    dn_dset_pool: vk::DescriptorPool,\n    dn_setl: vk::DescriptorSetLayout,\n    dn_img: vk::Image,\n    dn_img_mem: vk::DeviceMemory,\n    dn_view: vk::ImageView,\n    dn_module: vk::ShaderModule,")
s = s.replace("            hud_render_pass: vk::RenderPass::null(),", "            hud_render_pass: vk::RenderPass::null(),\n            dn_pipeline: vk::Pipeline::null(),\n            dn_layout: vk::PipelineLayout::null(),\n            dn_dset: vk::DescriptorSet::null(),\n            dn_dset_pool: vk::DescriptorPool::null(),\n            dn_setl: vk::DescriptorSetLayout::null(),\n            dn_img: vk::Image::null(),\n            dn_img_mem: vk::DeviceMemory::null(),\n            dn_view: vk::ImageView::null(),\n            dn_module: vk::ShaderModule::null(),")
io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('dn fields added')
