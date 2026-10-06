# 项目结构与播放链路

## 运行和验证

应用通过 `COOKIE` 环境变量读取登录凭据，不自行读取 `.env`。
开发时可使用 `dotenv run cargo run`。`cargo check`、`cargo test` 不启动界面；真实接口验证使用 `dotenv run cargo test -- --include-ignored`。

## 模块职责

```text
src/
├── main.rs                  初始化服务、创建 Entity、打开窗口
├── api.rs                   网易云接口及共享网络客户端、Tokio 运行时
├── models/                  歌曲、歌单、用户资料、播放资源信息
├── state/                   账号、音乐库索引、当前浏览的歌单详情
├── playback.rs              对外类型与只读 PlaybackSnapshot
├── playback/
│   ├── controller.rs        GPUI Entity：控制入口、队列、请求、通知
│   ├── engine.rs            rodio 设备、当前播放源、暂停、音量
│   ├── stream.rs            音源组装、跳转和取消生命周期
│   └── stream/
│       ├── download.rs      HTTP Range 下载、字节缓存、Read + Seek
│       ├── decode.rs        Symphonia 解析、定位和解码
│       ├── output.rs        有界 PCM 队列与 rodio Source 适配
│       └── tests.rs         本地 HTTP 与音频回归测试
└── ui/
    ├── shell.rs             主窗口布局、导航协调、页面生命周期
    ├── sidebar.rs           侧栏 View 与导航事件
    ├── pages/               页面 View
    ├── components/          播放栏、进度条、标签栏、虚拟表格
    ├── assets.rs            静态资源及远程缩略图 URL
    └── theme.rs             字体、主题与 UI 样式常量
```

`models` 是数据类型，不依赖 GPUI。登录资料与歌单创建者都使用 `UserProfile`，但只有 `AccountState` 负责加载登录账号与会员资料，歌单创建者不会携带账号加载逻辑。

GPUI 的共享业务对象放在 Entity 中：`AccountState`、`MusicLibrary`、`PlaylistDetail`、`PlaybackController`。View 观察 Entity 的通知并渲染；底层 `PlayerEngine` 和流式解码实现不依赖 GPUI 或网易云 SDK。

## 统一播放入口

界面只持有 `Entity<PlaybackController>`，用 `snapshot()` 读取状态，所有修改通过控制器方法提交。

| 接口 | 用途 |
| --- | --- |
| `play_from_queue(songs, song_id, cx)` | 按给定队列选歌；继续当前未播完的歌曲时保留进度。 |
| `pause(cx)` / `resume(cx)` / `toggle(cx)` | 控制当前播放；加载期间保留用户的播放意图。 |
| `previous(cx)` / `next(cx)` | 按队列顺序切歌，队列边界不循环。 |
| `seek_to(Duration, cx)` | 提交时间位置，由后台解码线程完成跳转。 |
| `set_volume(f32, cx)` | 0..=1，超出范围钳制，忽略非有限值；切歌与重试保留音量。 |
| `set_quality(AudioQualityLevel, cx)` | 选择与 `song_url_v1` 的 `level` 一一对应的九档音质（标准、较高、极高、无损、Hi-Res、高清臻音、沉浸声、全景声、超清母带）；重载当前歌曲并保留位置及暂停意图。 |
| `snapshot()` | 只读当前歌曲、位置、时长、播放/加载状态、错误和音量。 |
| `can_seek()` / `is_play_requested()` | 给现有播放控件提供交互依据。 |

使用示例：

```rust,ignore
playback.update(cx, |controller, cx| {
    controller.pause(cx);
    controller.seek_to(std::time::Duration::from_secs(125), cx);
    controller.set_volume(0.5, cx);
});
```

歌曲 ID 使用 `u64`，时间使用 `Duration`。进度条的 SliderState 使用 `0..歌曲时长毫秒数`，步长为 1 毫秒；绘制比例由 Slider 的 `percentage()` 提供，释放时将毫秒值转换为 `Duration`，并拒绝非有限值、钳制到歌曲范围。实际时长修正时重建滑块范围并取消旧手势。控制器不依赖 Slider 类型，保存私有快照，不提供可变借用，避免 UI 修改状态但未同步到音频引擎。

## 播放地址与流式音频

```mermaid
flowchart LR
    UI[歌单 / 播放栏 / 进度条] --> C[PlaybackController]
    C --> API[MusicApi song_source]
    API --> I[AudioSourceInfo: URL / 文件长度 / 时长]
    I --> S[StreamingAudio: 组装与生命周期]
    S --> D[download: HTTP Range 字节缓存]
    D -->|CacheReader: Read + Seek| F[decode: Symphonia 解码]
    F --> P[output: 有界 PCM 队列]
    P -->|BufferedSource| E[PlayerEngine: rodio Player / MixerDeviceSink]
    E --> C
    C -->|snapshot + notify| UI
```

`MusicApi::song_source` 使用现有 COOKIE 调用 `song_url_v1`，按控制器设置请求音质，并记录响应实际返回的音质。账号权限和歌曲资源可能导致降级或拒绝播放。当前播放栏恢复原有 SVG 图标，不提供音质菜单，控制器接口仍保留。音频服务器连接、状态检查、下载超时和读取放在播放模块。音频直接使用 reqwest，GPUI 图片使用 reqwest-client；两种客户端各自复用连接池，共用 Tokio 运行时，不向资源服务器发送账号 COOKIE。

首次请求发送 HTTP Range，以实际的 206 响应确认支持分段下载，并校验 Content-Range、文件长度和响应数据长度。缓存按 256 KiB 分块，解码器需要哪个字节区间，就请求哪个区间；Range 缓存上限为 32 MiB，保留文件头，超出时淘汰旧块。已有 ETag 或 Last-Modified 时使用 If-Range，防止混用发生变化的资源。服务器忽略首次 Range 并返回 200 时，自动回退到顺序下载，缓存上限为 256 MiB。

分段数据到达时立即发布到缓存，不必等满一个 256 KiB 块才开始播放；尚未完成的首次请求同样可以被新的 seek 抢占。后台 Symphonia 解码线程通过 `Read + Seek` 读取缓存，缺数据时在该线程等待。解码结果进入约 400 ms 的有界 PCM 队列，容量依据实际样本量计算，避免 Hi-Res 小包导致缓冲时长缩水；rodio 输出线程只取已准备好的样本，缺数据时输出完整声道帧的静音，不等待网络。解码错误、文件截断与校验失败会记录为错误，不作为正常 EOF 自动切歌。

引擎长期保留 MixerDeviceSink，换歌时创建新的 Player，释放旧播放源并取消旧下载。控制器每 250 ms 读取已交给输出链路的歌曲样本位置，缓冲静音不计入进度；这不是声卡实际输出时间的精确测量。正常 EOF 且 PCM 消费完毕后，由控制器自动切歌；播放源在此之前仍留在 Player 中，避免末尾 seek 与源被移除发生竞争。队列末尾停止；下载或解码失败则停止并记录错误，点击播放可重试，保留失败时的播放位置。请求代次防止过期加载结果覆盖新歌曲。

Seek 清空旧 PCM 并立即提交目标，同时唤醒正在等待网络的旧读取器并取消旧 Range 请求。解码线程为新 seek 重建读取器和解码器，避免被中断的解析留下半包状态；seek 失败后线程仍等待后续跳转，可以恢复。新一轮解析需要的文件头来自缓存，随后请求目标附近区间。FLAC、MP4 使用格式自身的定位机制；MP3 使用 Coarse 定位，VBR 文件的跳转位置可能近似，避免 Accurate 模式从头扫描。服务器不支持 Range 时，未下载的位置仍需等待顺序下载。

时长优先取解码器，其次取播放地址接口（包含试听片段），最后取歌曲详情；MP3 优先使用播放地址接口时长，因为没有 Xing 时解码器时长可能仅是估计值。MP3 的粗定位会按可信时长换算比例，正常 EOF 依据实际文件结束判断。快照的 revision 每次重载递增，进度条用它隔离跨歌曲、跨音质的旧拖动事件。播放栏保留歌曲和歌手信息，不显示额外状态文本；右侧工具恢复原有纯 SVG 实现。

播放字节缓存不持久化，切歌或重载音质会释放。`stream/download.rs` 中的 Range 和顺序读取只服务当前音源，不负责把歌曲保存为文件。未来的歌曲下载功能应放在独立模块，管理下载任务、文件写入和完成状态；顺序读取只是播放服务器不支持 Range 时的兼容路径。当前没有音量调整 UI，但接口已经作用于引擎并保留设置。

## 流控制与状态归属

`StreamingAudio` 组装三个内部模块，并负责一次跳转的协调和整个音源的取消。下载缓存不再引用 PCM 队列：两者共享独立的 `StreamControl`，其中只有跳转代次和一个取消标志。跳转时持有 PCM 锁，先清空旧样本、提交目标并递增代次，再中断旧下载，最后释放锁。这样解码线程不会先启动新一轮读取、随后又被旧请求的取消操作打断。释放音源时取消下载任务，并在各自等待锁内唤醒字节读者和解码线程；输出源看到同一个取消标志后结束。

这里有两种独立的代次。控制器的 `load_revision` 在切歌或重载音质时递增，同时写入快照 `revision`，隔离旧的接口结果和 UI 拖动事件。音源内部的 `StreamControl.generation` 只在 seek 时递增，使旧读取器、解码结果和 PCM 失效，但保留同一资源已下载的字节。

| 状态 | 写入者 | 含义 |
| --- | --- | --- |
| `loading` | 控制器 | 获取播放地址或等待首批解码样本。 |
| `play_when_ready` | 控制器 | 用户的播放意图，加载期间也能暂停或继续。 |
| `is_playing` | 控制器 | 音源已加载并请求播放；缓冲时可能仍为 true。 |
| `decode_finished` | 解码线程；seek 时由音源重置 | 解码正常结束，仍可能有未消费的 PCM。 |
| `output_drained` | 输出源；seek 时由音源重置 | 最后一个 PCM 已交给 rodio，控制器才允许自动切歌；不表示设备缓冲也已排空。 |
| `buffering` | 输出源，控制器同步到快照 | 输出暂时取不到 PCM；快照在暂停或加载时为 false。 |
| `position_us` | 输出源；seek 时由音源设置目标 | 已交给输出链路的歌曲位置，缓冲静音不推进它。 |

下载错误明确分为 `Range` 和 `Sequential`：Range 错误绑定跳转代次，新 seek 可以清除并重新请求；顺序下载失败是整个资源的错误，seek 不会掩盖它。下载指令用 `offset: None` 表示只中断旧请求，不再用最大字节位置充当特殊指令。实际区间由新读取器在缺数据时提交。

## 歌单与账号状态

账号资料返回后立即通知音乐库，会员资料独立补充。音乐库并行加载歌单索引和喜欢状态；打开歌单才加载详情与歌曲。所有 trackIds 每 200 首请求一次，按原始顺序整理歌曲，缺失的歌曲不伪造。

“我喜欢的音乐”从音乐库中解析 specialType == 5 的歌单 ID，普通歌单使用 ContentPage::Playlist(id)，都进入同一个 PlaylistPage 和 PlaylistDetail。导航只由 shell 协调，页面的显示顺序、标签、列宽和 hover 留在 View。切换歌单取消旧请求并清空旧歌曲，请求代次阻止 A → B → A 的过期结果生效。

选歌时将当前显示顺序复制为播放队列，因此播放与当前浏览的歌单相互独立。离开页面不会停止音乐。歌单标题、封面、创建者和计数取当前歌单；账号头像和会员标识取 AccountState。

## 依赖版本

使用 rodio 0.22.2、Symphonia 0.5.5 和 reqwest 0.13.5。Symphonia 直接处理 MP3、FLAC、AAC、ALAC、Vorbis 和 PCM 解码，rodio 负责设备输出，不开启录音功能。GPUI 与其 reqwest 适配器保持一致的 0.3.7。

SDK 内部依赖的 reqwest 0.12、GPUI 分支依赖等由各自上游决定，本项目的音频下载使用 0.13；不能只修改锁文件就把这些类型替换成同一个版本。
