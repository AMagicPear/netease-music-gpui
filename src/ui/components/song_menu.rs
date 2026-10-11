//! 歌曲行「更多」图标弹出的原生菜单。
//!
//! 菜单项只是把命令派发出去，具体行为由各自的 action 处理器接住：这里先把整份菜单
//! 定下来，方便以后逐项接线。

use gpui::Action;
use gpui::*;
use gpui_kit::component::native_menu::NativeMenu;

use crate::models::Song;

actions!(
    song_menu,
    [
        Play,
        ViewComments,
        Collect,
        Download,
        Share,
        Buy,
        CopyLink,
        Remove,
        ReduceRecommendation,
    ]
);

/// 「下一首播放」：把这首 `Song` 插到当前播放歌曲之后。
///
/// 原生菜单通过 `Window::dispatch_action` 派发，action 本身能携带数据，所以这里直接把
/// 歌曲装进 action，从源头带到处理方，不用另外记住「右键的是哪一行」。因为它不会从
/// 键位 JSON 反序列化，声明 `no_json` 以省去 `Deserialize` / `JsonSchema`。
#[derive(Action, Clone, PartialEq)]
#[action(namespace = song_menu, no_json)]
pub struct PlayNext(pub Song);

/// 在 `position`（窗口坐标）弹出「更多」菜单；`song` 是右键到的那一首，供菜单项使用。
pub fn show_song_menu(position: Point<Pixels>, song: Song, window: &mut Window, cx: &mut App) {
    NativeMenu::new()
        .menu("播放", Box::new(Play))
        .menu("下一首播放", Box::new(PlayNext(song)))
        .menu("查看评论", Box::new(ViewComments))
        .separator()
        .menu("收藏", Box::new(Collect))
        .menu("下载", Box::new(Download))
        .menu("分享", Box::new(Share))
        .menu("购买单曲", Box::new(Buy))
        .menu("复制链接", Box::new(CopyLink))
        .separator()
        .menu("从歌单中删除", Box::new(Remove))
        .menu("减少推荐", Box::new(ReduceRecommendation))
        .show(position, window, cx);
}
