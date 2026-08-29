pub fn hello_spv() -> Vec<u32> {
    let mut w: Vec<u32> = vec![0x0723_0203u32, 0x0001_0000u32, 0, 20, 0];
    let mut id = 0u32;
    let mut nid = |i: &mut u32| { *i += 1; *i };
    let mut e = |w: &mut Vec<u32>, op: u32, ops: &[u32]| { w.push(((1 + ops.len()) as u32) << 16 | op); w.extend_from_slice(ops); };
    let mut i = 0u32;
    e(&mut w, 17, &[1]);           // Shader
    e(&mut w, 14, &[0, 1]);
    let t_void = nid(&mut i);
    let t_fn = nid(&mut i);
    let t_u32 = nid(&mut i);
    let t_p_u32 = nid(&mut i);
    let t_pf_v3u = nid(&mut i);
    let t_v3u = nid(&mut i);
    let c0 = nid(&mut i);
    let c1 = nid(&mut i);
    let g_in = nid(&mut i);
    let g_out = nid(&mut i);
    let f_main = nid(&mut i);
    let l1 = nid(&mut i);
    let v1 = nid(&mut i);
    let v2 = nid(&mut i);
    let x1 = nid(&mut i);
    let sum = nid(&mut i);
    // entry + mode
    e(&mut w, 15, &[5, f_main, 0x6d61_696e, g_in]);
    e(&mut w, 16, &[f_main, 17, 64, 1, 1]);
    e(&mut w, 71, &[g_in, 34, 0]);
    e(&mut w, 71, &[g_in, 33, 0]);
    e(&mut w, 71, &[g_out, 34, 0]);
    e(&mut w, 71, &[g_out, 33, 1]);
    e(&mut w, 71, &[g_in, 11, 5]);
    // types
    e(&mut w, 19, &[t_void]);
    e(&mut w, 33, &[t_fn, t_void]);
    e(&mut w, 21, &[t_u32, 32, 0]);
    e(&mut w, 32, &[t_p_u32, 12, t_u32]);
    e(&mut w, 23, &[t_v3u, t_u32, 3]);
    e(&mut w, 32, &[t_pf_v3u, 1, t_v3u]);
    e(&mut w, 43, &[c0, t_u32, 7]);
    e(&mut w, 43, &[c1, t_u32, 7]);
    e(&mut w, 59, &[g_in, t_pf_v3u, 1]);
    e(&mut w, 59, &[g_out, t_p_u32, 12]);
    e(&mut w, 54, &[f_main, t_void, 0, t_fn]);
    e(&mut w, 248, &[l1]);
    e(&mut w, 61, &[v1, t_v3u, g_in]);
    e(&mut w, 186, &[x1, t_u32, v1, 0]);
    e(&mut w, 131, &[sum, t_u32, x1, c0]);
    e(&mut w, 62, &[g_out, sum]);
    e(&mut w, 253, &[]);
    e(&mut w, 56, &[]);
    w[3] = i + 1;
    w
}
