# -*- coding: utf-8 -*-
import io
p = 'src/rt_impl.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("let ds = dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl]), None)?[0];", "let ds = dev.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&[dsl])).map_err(|e| format!(\"{e:?}\"))?[0];")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('dset alloc fixed')
