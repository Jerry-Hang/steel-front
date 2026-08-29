# -*- coding: utf-8 -*-
import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()
old = """            Ok(renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                self.renderer = Some(renderer);
            }"""
new = """            Ok(mut renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                // ---- RT core 纯求交吞吐基准（RV3D_PT_BENCH=1）----
                if std::env::var("RV3D_PT_BENCH").as_deref() == Ok("1") {
                    let boxes = vec![
                        crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [50.0, 0.5, 50.0], material: 0 },
                        crate::engine::ray_tracer::PtBox { center: [1.0, 1.0, 0.0], half: [2.0, 2.0, 1.0], material: 1 },
                        crate::engine::ray_tracer::PtBox { center: [-4.0, 1.5, -2.0], half: [1.5, 1.5, 1.5], material: 2 },
                        crate::engine::ray_tracer::PtBox { center: [0.5, 1.0, 5.0], half: [0.8, 0.8, 0.8], material: 3 },
                    ];
                    match renderer.run_pt_bench(&boxes, 1 << 20, 200) {
                        Ok((mrays, hits)) => {
                            log::info!(
                                "RT-BENCH: {} rays x 200 iters = {} M射线, 命中 {}, {:.1} Mrays/s ({:.0} Kray/ms)",
                                ., ",
                            )
                        }
                        Err(e) => log::error!("RT-BENCH 失败: {e}"),
                    }
                }
                self.renderer = Some(renderer);
            }"""
# 手写正确格式（上面占位错误）——直接覆盖
new2 = """            Ok(mut renderer) => {
                log::info!("Vulkan 渲染器初始化成功");
                // ---- RT core 纯求交吞吐基准（RV3D_PT_BENCH=1）----
                if std::env::var("RV3D_PT_BENCH").as_deref() == Ok("1") {
                    let boxes = vec![
                        crate::engine::ray_tracer::PtBox { center: [0.0, -0.5, 0.0], half: [50.0, 0.5, 50.0], material: 0 },
                        crate::engine::ray_tracer::PtBox { center: [1.0, 1.0, 0.0], half: [2.0, 2.0, 1.0], material: 1 },
                        crate::engine::ray_tracer::PtBox { center: [-4.0, 1.5, -2.0], half: [1.5, 1.5, 1.5], material: 2 },
                        crate::engine::ray_tracer::PtBox { center: [0.5, 1.0, 5.0], half: [0.8, 0.8, 0.8], material: 3 },
                    ];
                    match renderer.run_pt_bench(&boxes, 1 << 20, 200) {
                        Ok((mrays, hits)) => log::info!(
                            "RT-BENCH: 1M射线 x 200 = 2亿射线, 命中 {hits}, {mrays:.1} Mrays/s"
                        ),
                        Err(e) => log::error!("RT-BENCH 失败: {e}"),
                    }
                }
                self.renderer = Some(renderer);
            }"""
s = s.replace(old, new2, 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('trigger added')
