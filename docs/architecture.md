# 架构与维护约定

## 数据与页面

`models/` 保存数据类型与解析，不依赖 GPUI；`api/` 通过共享 `MusicApi` 调用接口。
`state/` 中的 GPUI Entity 持有业务数据和命令；View 订阅它们，只保存页签、排序、输入、滚动等交互状态。
账号登录和播放控制器是各自的状态机，不套用普通列表的刷新流程。

远端快照统一使用 `state/resource.rs` 的 `Resource<T>`：

- 未请求、首次加载、首次失败、有数据是四种不同状态，成功的空列表也是数据。
- 同一个 key 刷新时保留快照；成功替换，失败保留旧值。
- 换账号、关键词或资源 ID 时先 `reset()`，不能显示另一个 key 的数据。
- `fetch()` 合并在途请求；`begin()` 替换请求；取消、重置和直接赋值都会使旧回调失效。
- Resource 销毁时取消网络任务；已经排入主线程的旧结果仍须通过代次校验。登录 / 登出还会使旧会话的结果失效。
- 普通读取用 `fetch()`；分页和歌单的“元数据 → 歌曲”使用同一模块的 `request()` 桥接，并经 Resource 落地。

`MainContent::show_page` 是主内容区的进入边界：每次切到页面都调用其刷新入口，不记录“访问过没有”，也不检查缓存是否存在。页签和侧面板在自己的进入边界执行同样操作。构造期间的首次读取与进入时的读取由 Resource 合并；业务 store 负责 key 和账号隔离。没有轮询、自动重试或全局磁盘页面缓存。

| Store | 数据与特殊规则 |
| --- | --- |
| `RecommendStore` | 推荐区块独立请求；雷达首曲封面有独立 Resource，不能改变歌单的加载状态。 |
| `DailySongsStore` | 等待账号恢复，游客显示登录空态。 |
| `SearchResults` | 单曲与歌单各一份快照，切分类只请求当前分类；换词清空两份。 |
| `PlaylistDetail` | 先更新元数据，再加载歌曲；刷新保留原歌曲，过期元数据不能启动后续请求。 |
| `MusicLibrary` | 歌单索引与喜欢集合分别加载；本地喜欢修改作废在途读取，写请求不随页面切换取消。 |
| `CommentsStore` | 第一页替换、后续页去重追加；刷新作废旧分页；发布后确保再拉一次写入后的数据。 |
| `LyricsStore` / `SongCounts` | 以歌曲 ID 为 key；布局和高亮留在歌词 View。 |
| `MessagesStore` | 每类消息独立快照，进入就刷新；换账号清空，一键已读作废旧读取。 |

歌曲评论按歌曲 ID 保留本次会话的页面与草稿，后退时恢复正确歌曲；换账号清空这些页面及导航历史。

评论 View 根据 store 的 revision 区分整份替换与分页追加：替换时重建列表，追加时保留已有行的测量和滚动锚点。选区、滚轮归属和虚拟列表都留在 View。发布成功事件让编辑框清空草稿，不拼接本地伪造评论。

## 错误与日志

日志入口是 `logging.rs`，复用 `log` 与 `tracing-subscriber`。开发默认输出本项目 debug 和第三方 warn，发布默认 info / warn；输出到 stderr，无日志文件。

```bash
RUST_LOG=warn,netease_music_gpui=debug cargo run --locked
```

支持按模块设置级别，例如 `netease_music_gpui::state::resource=debug`。共享请求入口记录任务名称、开始、完成耗时、取消和失败；不记录 Cookie、请求正文或响应体。

首次加载失败提供简短重试入口；已有内容的后台刷新失败只记日志。分页失败保留已加载评论并提供重试。评论提交、登录保存等需要用户采取行动的错误仍在对应控件旁显示；可选区块或互动计数失败不铺满页面。

## 导航、通知与布局

跨 View 导航经 `ui/events.rs` 的 Entity 事件交给 shell；键位与原生菜单使用 Action，主窗口启动时设置根焦点。
`PlaybackController::publish` 区分歌曲/队列、位置、音量和预加载通知，歌曲列表不订阅每 100ms 的位置更新。
背景只跟随导航、歌单元数据和窗口布局，避免 hover、评论滚动触发整窗重绘。

播放页的两屏滚动由 `now_playing/page/scroll.rs` 管理。评论屏到位后列表才接管滚轮；移动过的事件由列表消费，抵达边界后的下一事件交还外层。
评论页的头部是虚拟列表首条目，吸顶输入框只绘制一份。变高评论使用 GPUI `list` 实测行高，歌曲表格用固定行高的 `components/virtual_table.rs`。

## 播放与缓存

UI 只经 `PlaybackController` 提交命令，读取只读 `snapshot()`。页面歌曲与播放队列是两份数据，排序和离开页面不会改变正在播放的队列。
队列游标指向具体条目，支持重复歌曲；随机播放只洗牌一次，退出时恢复原顺序。播放方式、音质、队列和进度由 `Persistence` 保存，恢复时暂停。

心动模式由 `playback/controller/heart.rs` 持有会话、补曲与取消。播放器的喜欢歌单续播入口和推荐卡片的新队列入口是不同操作。
换队列、账号或退出心动使旧请求失效；API 的 `songInfo: null` 按 ID 补详情并保持服务端顺序，不把 `count` 当分页游标。

音频链路：`MusicApi::song_source` → `StreamingAudio` → HTTP Range 字节缓存 → Symphonia 解码 → 有界 PCM 队列 → CPAL。
网络和解码在后台，音频回调不访问磁盘或网络，只消费完整声道帧；缺数据输出静音，错误不能当正常曲终。

- 首次请求验证 Range、长度和 Content-Range；服务器忽略 Range 时回退顺序下载。每次等待有超时，重试和等待可被 seek / 取消打断。
- Seek 清空旧 PCM、递增音源代次并取消旧读取；失败后仍允许再次 seek。MP3 使用 Coarse 跳转，VBR 定位可能近似。
- 默认输出设备按歌曲采样率、声道数与 f32 创建流，相同格式复用；应用不重采样。macOS 与 Windows 共享后端由系统适配，其他平台拒绝格式时报告错误。
- 音频缓存预算 2 GiB、单曲上限 512 MiB；在途任务预留容量。租约保护当前/下一首，未占用的完整资源按 LRU 淘汰。
- `.part` 校验后原子提交；启动清理孤儿文件，目录锁防止多实例同时清理。索引不保存 Cookie 或签名 URL。
- 完整文件命中前仍重新获取播放地址确认权限。距末尾 60 秒内准备下一首；UI 独立预取封面。当前实现不承诺声卡级 gapless。

播放状态位于应用数据目录，可淘汰音频位于系统缓存目录的 `NeteaseMusicGpui/audio-v1`（Linux 为 `netease-music-gpui/audio-v1`），清理音频不会删除播放记录。Cookie 仅发送给音乐接口，图片和音频客户端不携带账号 Cookie。

## 验证与依赖

```bash
cargo fmt --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

UI、动画和鼠标交互手动运行验收。普通测试不启动窗口、不联网、不使用声卡；播放控制器使用临时持久化目录。
回归保留接口解析、请求取消与过期结果、分页合并、播放队列、音频、缓存和持久化，不为每种展示行为写测试。
真实接口测试默认忽略，凭据优先使用 `COOKIE`，否则读取已保存的登录会话；只运行所需的命名测试，发布评论测试会产生真实写入。

GPUI 与 reqwest 适配器目前均为 0.3.8；CPAL 0.17.3、Symphonia 0.5.5、音频 reqwest 0.12。以 Cargo.lock 和本地解析源码为准，不能只改锁文件来替换不同版本的客户端类型。补丁与打包见 [vendor/README.md](../vendor/README.md) 和 [README.md](../README.md)。
