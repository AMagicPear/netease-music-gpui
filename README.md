
# netease-music-gpui

使用 Rust 和 [GPUI](https://gpui.rs/) 复刻网易云音乐桌面界面的练手项目，逐步接入真实账号、歌单和歌曲播放，学习原生 UI、共享状态与异步音频处理。

项目仍在开发中，主要在 macOS 上开发和验证，其他平台尚未验证。

![GPUI 原生实现的网易云音乐界面](docs/images/screenshot.png)

> [!TIP]
> 是的，你没有看错！这不是原版而是 GPUI 原生实现！！


## 当前功能

- 通过网易云登录 COOKIE 获取账号资料、头像和会员信息。
- 加载个人歌单与「我喜欢的音乐」，展示真实歌曲、封面、创建者和喜欢状态。
- 虚拟化歌曲列表，支持排序和拖动标题／专辑列分界线。
- 选歌、播放全部、暂停／继续、上一首／下一首，以及播完自动播放下一首。
- 流式下载和后台解码，支持拖动进度；切换页面不会停止播放。
- 播放栏展示歌曲信息、红心计数和评论计数。

## 运行

需要较新的 Rust stable 工具链、Git、网络连接及可用的网易云登录 COOKIE。macOS 编译环境还需要 Xcode Command Line Tools。

在项目根目录运行：

```bash
export COOKIE='你的网易云登录 Cookie'
cargo run --locked
```

`COOKIE` 应填写登录后请求中的 Cookie 字符串，不包含 `Cookie:` 请求头名称。应用启动时直接读取环境变量，缺失或为空会退出；播放权限取决于账号及接口返回的资源。

应用不会自动加载 `.env`。如果已经安装支持 `dotenv run` 的工具，可以在根目录的 `.env` 中配置：

```dotenv
COOKIE=你的网易云登录 Cookie
```

然后运行：

```bash
dotenv run cargo run --locked
```

`.env` 已加入 `.gitignore`。Cookie 属于登录凭据，请勿提交到仓库。

## 技术栈

| 依赖 | 用途 |
| --- | --- |
| GPUI / gpui-kit | 原生窗口、布局、组件与 Entity 状态管理 |
| ncm-api-rs | 网易云账号、歌单、歌曲详情和播放地址接口 |
| Tokio / reqwest | 异步网络请求与音频下载 |
| rodio | 音频解码与设备输出 |
| serde / serde_json | API 数据解析 |

音频下载直接使用标准 `reqwest`；GPUI 图片请求使用 `reqwest-client` 适配器。适配器内部依赖 `gpui-pre-reqwest` 分支，因此两者各自复用客户端和连接池。

## 目录结构

```text
src/
├── main.rs                  初始化与应用组装
├── api.rs                   网易云接口、网络客户端与 Tokio 运行时
├── models/                  歌曲、歌单、用户及播放资源的数据模型
├── state/                   账号、音乐库和歌单详情的共享状态
├── playback/
│   ├── controller.rs        统一播放接口、队列和 GPUI 状态通知
│   ├── engine.rs            音频设备、播放源、暂停和音量
│   └── stream.rs            流式下载、缓存、后台解码和 PCM 队列
└── ui/
    ├── shell.rs             主窗口布局和导航协调
    ├── sidebar.rs           侧栏
    ├── pages/               页面
    ├── components/          播放栏、进度条、标签栏和虚拟表格
    ├── assets.rs            资源加载与缩略图地址
    └── theme.rs             字体与主题

assets/                      图标、图片和字体
docs/architecture.md         模块职责与播放链路说明
```

共享业务状态由 GPUI Entity 管理，View 观察通知并渲染。界面通过 `PlaybackController` 提交播放命令，通过只读 `snapshot()` 获取状态；底层音频引擎不依赖 GPUI。

详细设计见 [架构文档](docs/architecture.md)。

## 开发验证

```bash
cargo fmt --check
cargo check --locked --all-targets
cargo test --locked
```

普通测试不启动界面，也不需要网络或声卡。真实接口测试默认忽略，配置 COOKIE 后可以单独运行：

```bash
cargo test --locked cookie_can_load_real_library -- --ignored
```

## 当前限制

- 使用 standard 音质；接口可能返回试听片段，或因账号权限和下架状态无法播放。
- 单首音频使用最多 100 MiB 的内存缓存，尚未实现持久磁盘缓存和 HTTP Range 下载。拖到未下载的位置时静默等待，部分容器格式可能需要等待尾部索引。
- 队列按选歌时的列表顺序播放，末尾停止；尚未实现随机播放、循环模式和队列管理界面。
- 音量控制接口已实现，音量调整 UI 尚未接入。
- 搜索、下载、分享等功能以及部分页面仍是占位；评论和收藏者标签尚未实现内容加载。
- 播放错误记录在终端，点击播放可重试。
