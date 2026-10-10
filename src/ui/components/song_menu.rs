//! 歌曲行「更多」图标弹出的原生菜单。
//!
//! 菜单项只是把命令派发出去，具体行为由各自的 action 处理器接住：这里先把整份菜单
//! 定下来，方便以后逐项接线。

use gpui::*;
use gpui_kit::component::native_menu::NativeMenu;

actions!(
    song_menu,
    [
        Play,
        PlayNext,
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

/// 在 `position`（窗口坐标）弹出「更多」菜单。
pub fn show_song_menu(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    NativeMenu::new()
        .menu("播放", Box::new(Play))
        .menu("下一首播放", Box::new(PlayNext))
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
