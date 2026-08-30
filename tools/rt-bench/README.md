# RT-Bench 显卡光线追踪算力测试台

## 用途
独立小工具：持续递归压测 NVIDIA RT core 射线运算能力 + FP32/FP16/FP8/FP4 吞吐测试。

## 运行
```
cd tools/rt-bench
cargo build --release
target\release\rt-bench.exe            # 默认全勾选测试
target\release\rt-bench.exe x 1 1 1 1  # 参数：FP32 FP16 FP8 FP4 (1测试 0跳过)
```

## 输出
- 控制台实时显示：GPU 名、各测试算力、总分
- 日志：`logs/测试日期_时间.txt`（与工具同目录统一文件夹，文件名 = 本地日期时间）

## 测试项
| 项目 | 说明 |
|---|---|
| RT | 4 盒 BLAS/TLAS + ray-query 4M 射线 x 200 迭代 x 3 轮，取峰值 Mrays/s |
| FP32 | compute 单精度 FMA 循环（GFLOPS） |
| FP16 | GL_EXT_shader_explicit_arithmetic_types_float16 真半精度 |
| FP8/FP4 | 8/4-bit 打包模拟（真实硬件扩展需 VK_KHR_shader_float8/4，Blackwell 支持后接入） |

## 结果（RTX 5060 Laptop, 2026-08-30）
- RT: ~106,000 Mrays/s
- FP32: ~361 TFLOPS
- FP16: ~214 TFLOPS
- FP8: ~291 TOPS, FP4: ~295 TOPS
