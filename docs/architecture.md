# 项目结构与播放链路

## 运行和验证

应用通过 `COOKIE` 环境变量读取登录凭据，不自行读取 `.env`。
开发时可使用 `dotenv run cargo run`。`cargo check`、`cargo test` 不启动界面；真实接口验证使用 `dotenv run cargo test -- --include-ignored`。

## 模块职责

```text
src/
├── main.rs                  初始化服务、创建 Entity、打开窗口
├── api.rs                   网易云接口及共享网络客户端、Tokio 运行时
├── models/                  歌曲、歌单、用户资料、播放资源信息与播放方式
├── state/                   账号、音乐库索引、当前浏览的歌单详情
├── playback.rs              对外类型与只读 PlaybackSnapshot
├── playback/
│   ├── controller.rs        GPUI Entity：控制入口、队列、请求、通知
│   ├── audio_cache.rs       磁盘租约、资源索引、容量预留、LRU 淘汰与崩溃恢复
│   ├── engine.rs            CPAL 输出流、暂停、音量与设备错误
│   ├── stream.rs            音源组装、跳转和取消生命周期
│   └── stream/
│       ├── download.rs      HTTP Range 整曲落盘、可读区间、Read + Seek
│       ├── decode.rs        Symphonia 解析、定位和解码
│       ├── output.rs        有界 PCM 队列与 f32 帧输出
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
| `play_list(source, songs, song_id, cx)` | 用一份新列表替换当前播放列表并播放其中一首；继续当前未播完的歌曲时保留进度。 |
| `pause(cx)` / `resume(cx)` / `toggle(cx)` | 控制当前播放；加载期间保留用户的播放意图。 |
| `previous(cx)` / `next(cx)` | 手动切歌；能否跨过列表两端由当前播放方式决定，随机模式下沿洗好的顺序走。 |
| `cycle_mode(cx)` / `set_mode(PlayMode, cx)` | 切换播放方式：顺序播放 → 单曲循环 → 列表循环 → 随机播放 → 回到顺序播放；进入随机时把列表洗一遍。 |
| `insert_next(song, cx)` | 插到当前歌曲之后，下一首就是它；目前尚未接入 UI，留给「下一首插入」和心动模式。 |
| `seek_to(Duration, cx)` | 提交时间位置，由后台解码线程完成跳转。 |
| `set_volume(f32, cx)` | 0..=1，超出范围钳制，忽略非有限值；切歌与重试保留音量。 |
| `set_quality(AudioQualityLevel, cx)` | 选择与 `song_url_v1` 的 `level` 一一对应的九档音质（标准、较高、极高、无损、Hi-Res、高清臻音、沉浸声、全景声、超清母带）；重载当前歌曲并保留位置及暂停意图，选择写入缓存。 |
| `snapshot()` | 只读当前歌曲、位置、时长、播放/加载状态、播放方式、错误和音量。 |
| `can_seek()` / `is_play_requested()` | 给现有播放控件提供交互依据。 |

使用示例：

```rust,ignore
playback.update(cx, |controller, cx| {
    controller.pause(cx);
    controller.seek_to(std::time::Duration::from_secs(125), cx);
    controller.set_volume(0.5, cx);
});
```

## 播放列表与播放方式

控制器持有**一份自己的播放列表**（`queue: Vec<Song>` + `queue_cursor` + `queue_source`）。它与任何页面上的歌单都是两份数据：选歌时把页面当前顺序复制进来，之后排序、洗牌、插入、切走页面都不会带偏播放；`queue_source` 只用于显示「来自哪个歌单」，不参与任何播放决策。

游标 `queue_cursor` 是列表的唯一位置来源，不再按歌曲 id 反查——同一首歌在列表里出现两次也不会认错位置，`insert_next` 这类按位置插入的操作才有确定的落点。未来的队列界面直接显示这份列表即可：它此刻的样子就是接下来要播的顺序。

四种方式与官方一致，枚举在 `models/play_mode.rs`。它只声明策略，不做下标推进（`shuffles()` 是否洗牌、`wraps()` 是否越过两端、`repeats_current()` 是否重复当前这首），移动由列表自己完成，因此可以用固定期望值回归。

| 方式 | 列表怎么来的 | 播完一首之后 | 手动上一首／下一首 |
| --- | --- | --- | --- |
| 顺序播放（默认） | 页面选择的顺序 | 下一首；最后一首播完停在末尾 | 沿列表移动，两端不动 |
| 单曲循环 | 页面选择的顺序 | 回到开头重播同一首 | 仍沿列表移动，两端不动 |
| 列表循环 | 页面选择的顺序 | 下一首，队尾回到队首 | 队尾下一首是队首，队首上一首是队尾 |
| 随机播放 | 进入该模式时洗一遍 | 下一首；洗后的队尾播完即止 | 沿洗好的顺序移动，两端不动 |

随机播放因此**不需要每次现掷**：切到它的那一刻用 `slice.shuffle`（`rand` 的 Fisher–Yates 实现）就地洗牌，之后与其他方式唯一的差别就是列表顺序不同。副作用正好是想要的——「上一首」回到刚才那首歌，不必另外维护播放历史。洗牌只加到模式切换和换列表这两个时机；离开随机不还原（也没法还原，原始顺序没有被保存），但列表本身不会排丢任何一首歌。

自动续播发生在 `tick` 里检测到 `finished()` 之后：单曲循环走 `replay`，用 `seek(0)` 把同一个音源回绕到开头，不重新请求播放地址也不重建解码线程（`seek` 会复位 `output_drained`，这是同一份音源能播第二次的前提）；否则列表前进一格，拿不到下一首时释放音源并停在末尾。

播放方式和选择的音质都随缓存持久化（`PlaybackState.mode` / `quality` 都带 `#[serde(default)]`，旧缓存缺字段时回落到默认值而不是整份丢弃）。保存的就是列表当前的样子，随机洗好的顺序跟着一起落盘，重启后继续顺着它播，不会重新洗。播放栏只有一个按钮：`cycle_mode` 在四种方式间循环，图标和 aria 标签都由 `PlayMode` 给出，UI 不判断模式。`心动模式.svg` 素材已在仓库里；官方它是以红心歌曲为锚点往队列里**穿插**推荐曲目（与需要接口的推荐服务配合），入口就是 `insert_next`，暂未实现。

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
    P -->|BufferedSource| E[PlayerEngine: CPAL 输出回调]
    E --> C
    C -->|snapshot + notify| UI
```

`MusicApi::song_source` 使用现有 COOKIE 调用 `song_url_v1`，按控制器设置请求音质，并记录响应实际返回的音质。账号权限和歌曲资源可能导致降级或拒绝播放。当前播放栏恢复原有 SVG 图标，不提供音质菜单，控制器接口仍保留。音频服务器连接、状态检查、下载超时和读取放在播放模块。音频直接使用 reqwest，GPUI 图片使用 reqwest-client；两种客户端各自复用连接池，共用 Tokio 运行时，不向资源服务器发送账号 COOKIE。

首次请求发送 HTTP Range，以实际的 206 响应确认支持分段下载，并校验 Content-Range、文件长度和响应数据长度。一次请求覆盖最多 4 MiB，磁盘可读区间仍按 256 KiB 记账。当前读取区间优先，其余时间持续下载到文件末尾，再补齐 seek 留下的空洞。已有 ETag 或 Last-Modified 时使用 If-Range，防止混用发生变化的资源。服务器忽略首次 Range 并返回 200 时回退到顺序落盘。连接及每次等待网络数据都有 20 秒超时，Range 请求失败最多重试两次，重试退避也可被 seek 打断。

网络数据经阻塞线程写入文件后才发布可读区间，不必等满一个块才开始解码；内存不保存整首歌曲的压缩字节。后台 Symphonia 解码线程通过 `Read + Seek` 从该文件读取，缺数据时等待下载。解码结果进入约 1 秒的有界 PCM 队列，起播、seek 和断流恢复攒够 500 ms 后才输出；短于门槛的资源在正常 EOF 时放行。容量依据实际样本量计算，避免 Hi-Res 小包导致缓冲时长缩水；CPAL 输出回调只取已准备好的样本，缺数据时输出完整声道帧的静音，不访问网络或磁盘。解码错误、文件截断与校验失败会记录为错误，并使相关本地缓存失效，不作为正常 EOF 自动切歌。

引擎使用默认输出设备，CPAL 流使用 Symphonia 解码出的歌曲采样率和声道数，样本格式固定为 `f32`，不读取设备默认格式。应用不做采样率或声道转换，也不按设备支持范围筛选歌曲格式。采样率和声道数都相同的歌曲复用输出流，任意一项变化时释放旧流并创建新流；正常停止只释放音源并取消下载。macOS 默认输出设备使用 DefaultOutput Audio Unit，由系统适配设备采样率；CPAL 0.17.3 的 Windows WASAPI 后端使用共享模式并启用 AUTOCONVERTPCM / SRC_DEFAULT_QUALITY，由 Windows 音频引擎转换到系统混音格式。不提供独占模式或 ASIO。其他后端若不接受歌曲采样率、声道数或 `f32` 格式，流创建会报告错误，不回退到应用重采样。设备报错后释放输出流，下次播放重新打开设备；暂不监听默认设备变化。

输出回调直接持有 `BufferedSource`，无额外输出包装层；每个输出帧读取一个歌曲 PCM 帧，没有插值、跳帧或重采样预读。解码线程将不同编码的 PCM 统一转换成交错排列的 `f32` 样本，输出回调直接读取到 CPAL 缓冲，无额外帧缓冲、声道映射或样本类型转换；不显式声明多声道扬声器布局。用户播放/暂停按声道帧应用 80 ms 的线性增益渐变；暂停淡出结束后不再消费 PCM，快速反向操作从当前增益继续，切歌和自动重播直接起播。音源替换通过独立互斥锁同步，设备回调只使用 `try_lock`，遇到切歌持锁便写静音，不等待控制器；生产者在锁外分配和复制样本，缩短换块时的竞争。设备错误反馈给控制器并停止播放。控制器每 100 ms 读取已消费的歌曲样本位置，缓冲静音不计入进度；这不是声卡实际输出时间的精确测量。正常 EOF 且 PCM 消费完毕后，由控制器按当前播放方式挑选下一首（见「播放方式」）；输出源保留到控制器停止，末尾仍可 seek。顺序播放走到队列末尾后停在末尾位置；下载或解码失败则停止并记录错误，点击播放可重试，保留失败时的播放位置。请求代次防止过期加载结果覆盖新歌曲。

Seek 清空旧 PCM 并立即提交目标，同时唤醒正在等待网络的旧读取器并取消旧 Range 请求。解码线程为新 seek 重建读取器和解码器，避免被中断的解析留下半包状态；seek 失败后线程仍等待后续跳转，可以恢复。新一轮解析需要的文件头来自缓存，随后请求目标附近区间。FLAC、MP4 使用格式自身的定位机制；MP3 使用 Coarse 定位，VBR 文件的跳转位置可能近似，避免 Accurate 模式从头扫描。服务器不支持 Range 时，未下载的位置仍需等待顺序下载。

时长优先取解码器，其次取播放地址接口（包含试听片段），最后取歌曲详情；MP3 优先使用播放地址接口时长，因为没有 Xing 时解码器时长可能仅是估计值。MP3 的粗定位会按可信时长换算比例，正常 EOF 依据实际文件结束判断。快照的 revision 每次重载递增，进度条用它隔离跨歌曲、跨音质的旧拖动事件。播放栏保留歌曲和歌手信息，不显示额外状态文本；右侧工具恢复原有纯 SVG 实现。

## 磁盘音频缓存与下一首预加载

缓存由一个共享 `AudioCacheStore` 管理，第一次加载时在 Tokio 阻塞线程初始化。播放状态保留在应用数据目录；可淘汰的音频使用系统缓存目录：macOS 为 `~/Library/Caches/NeteaseMusicGpui/audio-v1`，Windows 为 `%LOCALAPPDATA%/NeteaseMusicGpui/Cache/audio-v1`，Linux 为 `$XDG_CACHE_HOME/netease-music-gpui/audio-v1`（默认 `~/.cache`）。清理媒体缓存不会丢失队列和进度。

资源键使用 SHA-256 摘要，输入包含歌曲 ID、实际返回的音质、接口内容标识（MD5，缺失时用完整 URL）、声明大小和资源时长。签名 URL 更新后，相同内容仍能命中；不同音质、试听时长或资源版本互相隔离。索引只保存摘要、文件名、大小和访问顺序，不保存签名 URL、COOKIE 或播放权限。正式新加载仍先请求播放地址接口以确认当前权限；本地完整文件命中后不再请求音频服务器。预加载的待完成请求或已准备好的音源可以直接交接。

默认磁盘预算 2 GiB，单曲上限 512 MiB。新任务先预留接口声明的大小，大小未知时预留单曲上限；下载中的预留容量也计入预算。租约固定当前播放和下一首的文件，LRU 只淘汰无人使用的完整文件。下载写入唯一 `.part` 文件，全部区间和长度校验成功后同步文件、原子重命名为 `.audio`，再原子更新 JSON 索引。取消或失败的半成品不进入持久化索引，最后一个租约释放时删除；启动会清理孤儿文件并核对已索引文件的存在与长度。目录锁防止另一个应用实例清理仍在使用的文件。缓存属于播放加速设施，不等同于用户管理的歌曲下载功能。

当前歌曲已经整曲下载完、播放位置距末尾不超过 60 秒时，控制器准备下一首：获取播放资源、开始整曲缓存，并提前解码首批 PCM。只保留一个下一首音源，因此不会为整个队列分配 PCM。候选由当前游标和播放模式决定：顺序/随机跟随现有队列，列表循环在队尾回到队首，单曲循环复用当前音源。队列、播放模式、音质变化或停止会取消不再适用的任务。匹配的预加载在切歌时直接接管；预加载失败不影响当前歌曲，正式切到该曲时按普通加载流程重试一次。

封面通过 GPUI 的 `fetch_asset::<ImgResourceLoader>` 提前加载界面实际使用的 80px 和 480px 缩略图，和黑胶、播放栏、封面取色共用同一份资源缓存；不额外创建图片 HTTP 客户端。控制器只保留它预取的当前/下一首封面，淘汰已不相关的预取资源。图片预加载完成包括下载和解码，GPU 上传仍在绘制时进行。音频输出的续播检测间隔为 100 ms，因此该实现减少切歌等待，但不承诺声卡层面的无缝 gapless 拼接。

## 流控制与状态归属

`StreamingAudio` 组装三个内部模块，并负责一次跳转的协调和整个音源的取消。下载缓存不再引用 PCM 队列：两者共享独立的 `StreamControl`，其中只有跳转代次和一个取消标志。跳转时持有 PCM 锁，先清空旧样本、提交目标并递增代次，再中断旧下载，最后释放锁。这样解码线程不会先启动新一轮读取、随后又被旧请求的取消操作打断。释放音源时取消下载任务，并在各自等待锁内唤醒字节读者和解码线程；音源统一从 `pcm.control` 访问跳转代次和取消标志，不重复保存控制句柄。输出源按完整声道帧读取 PCM，缺数据时整帧写静音，取消后返回 false 并写静音。

这里有两种独立的代次。控制器只保存快照中的 `revision`，在切歌、重载音质或停止时递增，同时隔离旧的接口结果和 UI 拖动事件。音源内部的 `StreamControl.generation` 只在 seek 时递增，使旧读取器、解码结果和 PCM 失效，但保留同一资源已下载的字节。

| 状态 | 写入者 | 含义 |
| --- | --- | --- |
| `loading` | 控制器 | 获取播放地址或等待首批解码样本。 |
| `play_when_ready` | 控制器 | 用户的播放意图，加载期间也能暂停或继续。 |
| `is_playing` | 控制器 | 音源已加载并请求播放；缓冲时可能仍为 true。 |
| `decode_finished` | 解码线程；seek 时由音源重置 | 解码正常结束，仍可能有未消费的 PCM。 |
| `output_drained` | 输出源；seek 时由音源重置 | 最后一个 PCM 已交给输出端，控制器才允许自动切歌；不表示设备缓冲也已排空。 |
| `buffering` | 输出源，控制器同步到快照 | 输出暂时取不到 PCM；快照在暂停或加载时为 false。 |
| `position_us` | 输出源；seek 时由音源设置目标 | 已交给输出链路的歌曲位置，缓冲静音不推进它。 |

下载错误明确分为 `Range` 和 `Sequential`：Range 错误绑定跳转代次，新 seek 可以清除并重新请求；顺序下载失败是整个资源的错误，seek 不会掩盖它。下载指令用 `offset: None` 表示 seek 打断旧请求。新读取器每次跨块都会提交读取位置，包括缓存命中；下载器据此安排整曲下载与缺失区间的优先级。

## 歌单与账号状态

账号资料返回后立即通知音乐库，会员资料独立补充。音乐库并行加载歌单索引和喜欢状态；打开歌单才加载详情与歌曲。所有 trackIds 每 200 首请求一次，按原始顺序整理歌曲，缺失的歌曲不伪造。

“我喜欢的音乐”从音乐库中解析 specialType == 5 的歌单 ID，普通歌单使用 ContentPage::Playlist(id)，都进入同一个 PlaylistPage 和 PlaylistDetail。导航只由 shell 协调，页面的显示顺序、标签、列宽和 hover 留在 View。切换歌单取消旧请求并清空旧歌曲，请求代次阻止 A → B → A 的过期结果生效。

选歌时把当前显示顺序复制成控制器自己的播放列表（见「播放列表与播放方式」），因此播放与当前浏览的歌单相互独立，页面上的排序或后续改动都不会影响它。离开页面不会停止音乐。歌单标题、封面、创建者和计数取当前歌单；账号头像和会员标识取 AccountState。

## 依赖版本

使用 CPAL 0.17.3、Symphonia 0.5.5 和 reqwest 0.12。Symphonia 处理 MP3、FLAC、AAC、ALAC、Vorbis 和 PCM 解码，CPAL 直接输出 PCM，不经过混音器或播放器包装。GPUI 与其 reqwest 适配器保持一致的 0.3.8。洗牌使用 rand 0.9，与 gpui 依赖的 rand 是同一版本。磁盘缓存使用 sha2 0.10 计算资源键、tempfile 3 完成临时文件和原子提交，这两者已经存在于依赖树中。

音频 reqwest 跟随网易云 SDK 的 0.12，GPUI HTTP 适配器使用自己的上游依赖；不能只修改锁文件就把不同版本的客户端类型替换成同一个版本。

## 输出结构参考

核对了以下固定版本源码，借鉴输出流复用、音源适配固定输出格式的结构，没有复制其代码或引入其播放器抽象：

- [Psst CPAL 输出](https://github.com/jpochyla/psst/blob/3c3621aa79f820c737dd899e7e359b1359292466/psst-core/src/audio/output/cpal.rs)：优先双声道 F32 / 44.1 kHz，否则回退到设备默认配置；通过替换回调音源复用输出流。
- [Psst 音源适配](https://github.com/jpochyla/psst/blob/3c3621aa79f820c737dd899e7e359b1359292466/psst-core/src/player/worker.rs)：不同采样率使用 libsamplerate 的 SincMediumQuality，并映射输出声道；重采样包装由输出回调拉取，不是在解码线程里完成。
- [librespot 输出配置](https://github.com/librespot-org/librespot/blob/e023adbbf017ae1fc10d01531dbe50c409786f2d/playback/src/audio_backend/rodio.rs)：优先固定双声道 44.1 kHz，再回退到设备默认采样率和配置；它的 Symphonia 解码路径限制输入为双声道 44.1 kHz，不是任意采样率播放的完整参考。

本项目借鉴输出流复用结构，按歌曲采样率向系统提交 PCM，由系统完成必要的采样率转换；保留多格式输出，未引入 Psst 的 actor、音源 trait 或重采样依赖。Windows 自动转换标志的含义见 [Microsoft WASAPI 文档](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants)。
