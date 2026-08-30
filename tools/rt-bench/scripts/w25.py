# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old_q = """        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(0)
            .queue_count(1)
            .p_queue_priorities(&[1.0])];"""
new_q = """        let prio = [1.0f32];
        let mut dq = vk::DeviceQueueCreateInfo::default();
        dq.queue_family_index = 0;
        dq.queue_count = 1;
        dq.p_queue_priorities = &prio;
        let queues = [dq];"""
if old_q in s:
    s = s.replace(old_q, new_q, 1)
    print('queues fixed')
# 133: fp_test 的 update_descriptor_sets?（1 arg）——查 131-135
io.open(p, 'w', encoding='utf-8', newline='').write(s)
