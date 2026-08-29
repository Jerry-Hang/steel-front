# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("""        let (pipelines, _res) = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
        };
        let compute_pipeline = pipelines[0];""", """        let pipelines = unsafe {
            self.device.create_compute_pipelines(vk::PipelineCache::null(), &[compute_info], None)
                .map_err(|e| format!("RT compute pipeline: {:?}", e.1))?
        };
        let compute_pipeline = pipelines[0];""")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fix')
