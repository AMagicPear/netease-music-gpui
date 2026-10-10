pub mod assets;
// components 和 shell 里有 crate 外部（main.rs）要用的类型，所以放到 crate 可见；
// pages 和 sidebar 只在 ui 内部互相引用，保持私有。谁定义的就从谁的模块引，这里不再转发。
pub(crate) mod components;
mod cover_color;
// 「正在播放」沉浸页：共享状态 `NowPlaying` 由 main.rs 创建、shell 注入，所以要放到
// crate 可见。它自成模块而不在 `pages` 里——它不是侧边栏可切换的页面，而是窗口级浮层。
pub(crate) mod now_playing;
mod pages;
pub(crate) mod shell;
mod sidebar;
pub mod theme;
