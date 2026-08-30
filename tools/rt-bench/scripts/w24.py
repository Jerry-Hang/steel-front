# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("as_f.p_next = &mut bd_f as *mut _ as *mut vk::BaseOutStructure;", "as_f.p_next = &mut bd_f as *mut _ as *mut std::ffi::c_void;")
s = s.replace("rq_f.p_next = &mut as_f as *mut _ as *mut vk::BaseOutStructure;", "rq_f.p_next = &mut as_f as *mut _ as *mut std::ffi::c_void;")
s = s.replace("info.p_next = &mut rq_f as *mut _ as *mut vk::BaseOutStructure;", "info.p_next = &mut rq_f as *mut _ as *mut std::ffi::c_void;")
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('main pnext fixed')
# rt_impl map_memory ? 174 前面无 map_err（map_memory(...)? 在结果 String 函数内的 Err 转换——它 map 了但类型？）
p2 = 'src/rt_impl.rs'
s2 = io.open(p2, encoding='utf-8').read()
s2 = s2.replace("let p = dev.map_memory(mem, 0, inst_bytes.len() as u64, vk::MemoryMapFlags::empty())?;", "let p = dev.map_memory(mem, 0, inst_bytes.len() as u64, vk::MemoryMapFlags::empty()).map_err(|e| format!(\"{e:?}\"))?;")
# 174 的实际问题可能是 mem_alloc 闭包返回类型推断——加显式闭包类型标注（改为 fn 简化：整体用 map_err?）。先试 p_next 修复
io.open(p2, 'w', encoding='utf-8', newline='').write(s2)
print('rt map_memory fixed')
