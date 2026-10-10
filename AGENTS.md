这是一个试图用GPUI来复刻网易云音乐的项目，是一个练手项目，你应该多辅助用户查阅文档及做错误修正，或者向用户提供接下来如何编写的帮助，在做了某些更改之后，你也应该将它的原理解释清楚，让用户真正理解这样编写的用意，而不仅仅是以完成任务为目的。
UI 的视觉、动画和鼠标交互通过手动运行应用验收，根据实际效果调整，不为每种展示行为保留自动化测试。

自动化测试主要覆盖接口解析、播放队列、音频下载与解码、缓存和持久化，以及少量数据边界。普通测试不启动应用窗口，也不需要外网或声卡；播放控制器测试使用临时存储目录，不读写真实播放记录。播放回归测试使用本地 HTTP 服务器、内存生成的 WAV 和 `tests/fixtures/` 中的 MP3、AAC/M4A、96 kHz FLAC 测试音频，覆盖 Range 跳转、取消、失败恢复和损坏数据。真实接口测试默认忽略，配置 COOKIE 后可以单独运行：

```bash
cargo test --locked cookie_can_load_real_library -- --ignored
```

GPUI offers three different registers depending on your needs:

State management and communication with Entity's. Whenever you need to store application state that communicates between different parts of your application, you'll want to use GPUI's entities. Entities are owned by GPUI and are only accessible through an owned smart pointer similar to an Rc. See the app::context module for more information.

High level, declarative UI with views. All UI in GPUI starts with a view. A view is simply an Entity that can be rendered, by implementing the Render trait. At the start of each frame, GPUI will call this render method on the root view of a given window. Views build a tree of elements, lay them out and style them with a tailwind-style API, and then give them to GPUI to turn into pixels. See the div element for an all purpose swiss-army knife of rendering.

Low level, imperative UI with Elements. Elements are the building blocks of UI in GPUI, and they provide a nice wrapper around an imperative API that provides as much flexibility and control as you need. Elements have total control over how they and their child elements are rendered and can be used for making efficient views into large lists, implement custom layouting for a code editor, and anything else you can think of. See the element module for more information.

Each of these registers has one or more corresponding contexts that can be accessed from all GPUI services. This context is your main interface to GPUI, and is used extensively throughout the framework.