# -*- coding: utf-8 -*-
import io
p = 'src/engine/renderer.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("self.queue.submit(self.queue (", "self.graphics_queue.submit(self.graphics_queue (")
s = s.replace("self.device.queue_submit(self.queue,", "self.device.queue_submit(self.graphics_queue,")
s = s.replace("self.device.queue_wait_idle(self.queue)", "self.device.queue_wait_idle(self.graphics_queue)")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('queue fixed')
