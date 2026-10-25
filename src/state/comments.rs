use super::resource::{Resource, request};
use crate::{
    api::MusicApi,
    models::{CommentResource, SongComment, SongComments},
};
use gpui::{Context, EventEmitter, ReadGlobal};
use std::collections::HashSet;

#[derive(Default)]
pub struct Comments {
    pub items: Vec<SongComment>,
    pub total: u64,
    pub hot_count: usize,
    pub more: bool,
    offset: usize,
    seen: HashSet<u64>,
}

impl Comments {
    fn merge(&mut self, offset: usize, page: SongComments) {
        if offset == 0 {
            *self = Self::default();
        }
        self.total = page.total;
        self.more = page.more;
        if offset == 0 {
            self.hot_count = page.hot_count;
        }
        let before = self.items.len();
        self.items.extend(
            page.comments
                .into_iter()
                .filter(|comment| self.seen.insert(comment.id)),
        );
        if offset != 0 && before == self.items.len() {
            self.more = false;
        }
        self.offset = offset + 20;
    }
}

pub struct CommentPublished;

#[derive(Default)]
pub struct CommentsStore {
    pub resource: Option<CommentResource>,
    pub data: Resource<Comments>,
    pub publish: Resource<()>,
    /// 变化表示整份替换；只追加分页时不变，View 据此保留滚动锚点。
    pub revision: u64,
    estimated_total: Option<u64>,
    refresh_pending: bool,
    refreshing: bool,
}

impl EventEmitter<CommentPublished> for CommentsStore {}

impl CommentsStore {
    pub fn comments(&self) -> &[SongComment] {
        self.data
            .value()
            .map(|data| data.items.as_slice())
            .unwrap_or(&[])
    }
    pub fn total(&self) -> u64 {
        self.data
            .value()
            .map(|data| data.total)
            .or(self.estimated_total)
            .unwrap_or(0)
    }
    pub fn hot_count(&self) -> usize {
        self.data.value().map_or(0, |data| data.hot_count)
    }
    pub fn load_failed(&self) -> bool {
        self.data.error().is_some() && (!self.refreshing || self.data.value().is_none())
    }
    pub fn more(&self) -> bool {
        self.data.value().is_some_and(|data| data.more)
    }

    pub fn set_resource(&mut self, resource: Option<CommentResource>, cx: &mut Context<Self>) {
        if self.resource == resource {
            return;
        }
        self.resource = resource;
        self.data.reset();
        self.publish.reset();
        self.estimated_total = None;
        self.refresh_pending = false;
        self.revision = self.revision.wrapping_add(1);
        if resource.is_some() {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub fn estimate_total(&mut self, total: Option<u64>, cx: &mut Context<Self>) {
        if self.estimated_total.is_none() && self.data.value().is_none() {
            self.estimated_total = total;
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.data.is_loading() {
            // 分页在途时排队刷新；重复的第一页进入合并为同一个请求。
            self.refresh_pending |= !self.refreshing;
            return;
        }
        self.load_page(0, cx);
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        let offset = self.data.value().map_or(0, |data| data.offset);
        self.load_page(offset, cx);
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.refresh(cx);
        } else {
            self.load_more(cx);
        }
    }

    fn load_page(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Some(resource) = self.resource else {
            return;
        };
        if self.data.is_loading() {
            return;
        }
        self.refreshing = offset == 0;
        log::debug!("comments: {resource:?}, offset={offset}");
        let generation = self.data.begin();
        let client = MusicApi::global(cx).client.clone();
        self.data.track(request(
            cx,
            "comments",
            async move {
                match resource {
                    CommentResource::Song(id) => MusicApi::song_comments(client, id, offset).await,
                    CommentResource::Playlist(id) => {
                        MusicApi::playlist_comments(client, id, offset).await
                    }
                }
            },
            move |this, result, cx| {
                let success = result.is_ok();
                if !this.data.update(generation, result, |data, page| {
                    data.get_or_insert_with(Comments::default)
                        .merge(offset, page);
                }) {
                    return;
                }
                if success && offset == 0 {
                    this.revision = this.revision.wrapping_add(1);
                }
                if this.refresh_pending {
                    this.refresh_pending = false;
                    this.refresh(cx);
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn publish(&mut self, content: String, cx: &mut Context<Self>) {
        let Some(resource) = self.resource else {
            return;
        };
        let content = content.trim().to_string();
        if content.is_empty() || self.publish.is_loading() {
            return;
        }
        let generation = self.publish.begin();
        let api = MusicApi::global(cx);
        let client = api.client.clone();
        let cookie = api.cookie.clone();
        self.publish.track(request(
            cx,
            "publish comment",
            async move { MusicApi::publish_comment(client, cookie, resource, &content).await },
            move |this, result, cx| {
                let success = result.is_ok();
                if !this.publish.apply(generation, result) {
                    return;
                }
                if success {
                    // 即使第一页已在途，也必须在写成功之后再拉一次。
                    if this.data.is_loading() {
                        this.refresh_pending = true;
                    } else {
                        this.refresh(cx);
                    }
                    cx.emit(CommentPublished);
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn clear_publish_error(&mut self, cx: &mut Context<Self>) {
        if self.publish.error().is_some() {
            self.publish.reset();
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(ids: &[u64]) -> SongComments {
        SongComments {
            total: 100,
            hot_count: 1,
            more: true,
            comments: ids
                .iter()
                .map(|&id| SongComment {
                    id,
                    nickname: String::new(),
                    avatar_url: String::new(),
                    content: String::new(),
                    time: 0,
                    liked_count: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn pages_deduplicate_and_refresh_replaces_all_older_pages() {
        let mut data = Comments::default();
        data.merge(0, page(&[1, 2, 2]));
        data.merge(20, page(&[2, 3]));
        assert_eq!(
            data.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(data.offset, 40);
        data.merge(40, page(&[3]));
        assert!(!data.more);
        data.merge(0, page(&[9, 1]));
        assert_eq!(
            data.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            [9, 1]
        );
        assert_eq!(data.offset, 20);
        assert_eq!(data.total, 100);
    }
}
