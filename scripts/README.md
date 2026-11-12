# 打包脚本

在目标平台运行 `python3 scripts/package.py`，脚本会构建 release 并复制运行所需资源到 Cargo target 目录下的 `bundle/`。直接运行 `cargo build --release` 不会复制资源。运行 `python3 scripts/package.py --help` 查看选项。

- **macOS**：生成 `NetEase Music.app`，默认使用 ad-hoc 签名；设置 `APPLE_SIGNING_IDENTITY` 可使用指定证书。安装对应 Rust target 后，可通过 `--target` 构建 Apple Silicon 或 Intel 版本。此流程不执行公证。
- **Windows**：需要 Visual Studio C++ 构建工具和 Windows SDK。脚本通过 `VCToolsRedistDir` 或 `vswhere` 查找对应架构的 VC release CRT，也可使用 `--runtime-dir` 指定官方 CRT 目录。生成的 ZIP 包含程序、资源和运行库；未压缩目录同时保留，便于检查。
- **Linux**：生成包含程序和资源的 `tar.gz`；它依赖构建机的系统库，需要兼容的运行库、图形会话和 Vulkan 驱动。

Windows 和 Linux 的本地打包还会生成 SHA-256 校验文件。项目资源从应用旁的资源目录读取，因此分发时需要保留完整目录。

打包脚本回归测试不构建 Rust、不联网、不启动窗口：

```bash
python3 -m unittest discover -s scripts -p test_package.py
```
