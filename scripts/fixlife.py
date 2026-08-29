# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        let set_create = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&[set_layout, hits_layout]);""", """        let set_bindings = [set_layout, hits_layout];
        let set_create = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&set_bindings);""")
s = s.replace("""        let pipe_create = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&[set_layout_handle])
            .push_constant_ranges(&[]);""", """        let pipe_layouts = [set_layout_handle];
        let pc_ranges: [vk::PushConstantRange; 0] = [];
        let pipe_create = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&pipe_layouts)
            .push_constant_ranges(&pc_ranges);""")
s = s.replace("""        let compute_info = vk::ComputePipelineCreateInfo::default()
            .stage(vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(vs_module)
                .name(c"main"))
            .layout(pipe_layout);""", """        let stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(vs_module)
            .name(c"main");
        let compute_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage_info)
            .layout(pipe_layout);""")
s = s.replace("""        let dset_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(dpool)
            .set_layouts(&[set_layout_handle]);""", """        let dset_layouts = [set_layout_handle];
        let dset_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(dpool)
            .set_layouts(&dset_layouts);""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('lifetime fixed')
