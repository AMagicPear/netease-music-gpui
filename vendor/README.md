# GPUI 图片 GPU 旋转补丁

这里只跟踪 `patches/gpui-image-transform-0.3.8.patch`，不提交完整依赖源码。
补丁从原 stash 的 GPUI 0.3.7 实验移植到当前使用的 **0.3.8**。

首次构建或补丁更新后，在项目根目录运行：

```bash
python3 scripts/prepare_gpui.py
cargo run --locked
```

准备脚本从 crates.io 下载固定版本的 `gpui-pre`、`gpui-pre-apple`、`gpui-pre-wgpu`，
校验 SHA-256，并使用 `git apply` 应用补丁。生成的三个源码目录已加入 `.gitignore`；
根目录 Cargo 的 `[patch.crates-io]` 使用这些本地包，使直接和间接依赖共用相同版本。
原始发布包中的 Apache-2.0 许可证随源码保留。

脚本可重复运行；版本与补丁未变时直接跳过。需要修改补丁时，应编辑 patch，
不要只修改生成的源码，因为下一次准备会替换这些目录。

GPU 旋转接口：

```rust
img(url).with_transformation(Transformation::rotate(radians(angle)))
```

图片仍复用 GPUI 原有解码和纹理缓存，每帧只改变矩阵。布局和 hitbox 不变，
圆角在图片局部坐标中计算，祖先裁剪仍使用窗口坐标。
图片的线性采样限制在自身图集区域的像素中心范围内，避免旋转时混入相邻图片的颜色。
补丁同步调整了 Scene 包围盒、Metal/WGSL 绘制路径和 WebGL 的记录步长。
Metal 构建脚本从同级 `gpui-pre` 生成共享结构，避免 Rust 与 shader 布局不一致。

验证：`cargo test --locked`。macOS 使用真实离屏 Metal 渲染验证旋转方向、圆形裁剪和纹理复用；
其他平台通过 Naga 校验 WGSL 和结构布局，实际 GPU 绘制仍需在对应平台验证。

升级 GPUI 时，更新准备脚本中的版本与发布包 SHA-256，并重新生成、验证 patch。
如果上游提供同等接口，可删除补丁、准备脚本和 Cargo 中的路径覆盖。
