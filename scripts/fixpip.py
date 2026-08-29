# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        let compute_pipeline = unsafe {
            self.device
                .create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("RT compute pipeline: {e}"))?
        }[0];""", """        let (pipelines, _res) = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
        };
        let compute_pipeline = pipelines[0];""")
# _marker 名可能是 PhantomData（默认）——错误在 4637 p_acceleration_structures（&[assets.tlas] → as_ptr + len）
s = s.replace("            p_acceleration_structures: &[assets.tlas],", "            p_acceleration_structures: std::slice::from_ref(&assets.tlas).as_ptr(),")
s = s.replace("            p_buffer_info: &[buf_info],", "            p_buffer_info: std::slice::from_ref(&buf_info).as_ptr(),")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed')
