//! 远端快照与请求生命周期。刷新保留旧值，换 key 时由 store 调用 reset。

use crate::api::MusicApi;
use gpui::{Context, ReadGlobal};
use std::{future::Future, time::Instant};
use tokio::task::AbortHandle;

pub struct Resource<T> {
    value: Option<T>,
    loading: bool,
    error: Option<String>,
    generation: u64,
    request: Option<AbortHandle>,
}

impl<T> Resource<T> {
    pub fn new() -> Self {
        Self {
            value: None,
            loading: false,
            error: None,
            generation: 0,
            request: None,
        }
    }

    pub fn with_value(value: T) -> Self {
        let mut resource = Self::new();
        resource.value = Some(value);
        resource
    }

    /// 替换在途请求；同 key 的重复进入应先用 is_loading 合并。
    pub fn begin(&mut self) -> u64 {
        self.cancel();
        self.loading = true;
        self.error = None;
        self.generation
    }

    pub fn track(&mut self, request: AbortHandle) {
        if let Some(old) = self.request.replace(request) {
            old.abort();
        }
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation == generation && self.loading
    }

    pub fn apply(&mut self, generation: u64, result: Result<T, String>) -> bool {
        self.update(generation, result, |value, next| *value = Some(next))
    }

    /// 分页等增量更新也使用同一套过期校验与错误状态。
    pub fn update<U>(
        &mut self,
        generation: u64,
        result: Result<U, String>,
        merge: impl FnOnce(&mut Option<T>, U),
    ) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok(next) => {
                merge(&mut self.value, next);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    pub fn set(&mut self, value: T) {
        self.cancel();
        self.value = Some(value);
        self.error = None;
    }

    pub fn reset(&mut self) {
        self.cancel();
        self.value = None;
        self.error = None;
    }

    /// 取消也使已排入主线程的回调失效。
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }

    pub fn state(&self) -> DataState<'_, T> {
        match &self.value {
            Some(value) => DataState::Ready(value),
            None => match &self.error {
                Some(_) => DataState::Failed,
                None if self.loading => DataState::Loading,
                None => DataState::Idle,
            },
        }
    }

    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }
    pub fn value_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }
    pub fn is_loading(&self) -> bool {
        self.loading
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// 普通快照请求的统一入口；分页和分阶段请求共用下面的 request 桥接。
    pub fn fetch<V: 'static>(
        &mut self,
        name: &'static str,
        cx: &mut Context<V>,
        select: fn(&mut V) -> &mut Self,
        future: impl Future<Output = Result<T, String>> + Send + 'static,
    ) where
        T: Send + 'static,
    {
        if self.is_loading() {
            return;
        }
        let generation = self.begin();
        self.track(request(cx, name, future, move |view, result, cx| {
            if select(view).apply(generation, result) {
                cx.notify();
            }
        }));
        cx.notify();
    }
}

impl<T> Default for Resource<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Resource<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// 无数据时才显示加载或重试；后台刷新失败不覆盖已有内容。
pub enum DataState<'a, T> {
    Idle,
    Loading,
    Failed,
    Ready(&'a T),
}

/// Tokio 网络任务 → GPUI 主线程。日志不记录 cookie、正文或响应体。
pub fn request<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    name: &'static str,
    future: impl Future<Output = Result<T, String>> + Send + 'static,
    apply: impl FnOnce(&mut V, Result<T, String>, &mut Context<V>) + 'static,
) -> AbortHandle {
    let started = Instant::now();
    log::debug!("{name}: started");
    let session = MusicApi::global(cx).session_generation;
    let task = MusicApi::global(cx).runtime.spawn(future);
    let abort = task.abort_handle();
    cx.spawn(async move |this, cx| {
        let result = match task.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => {
                log::debug!("{name}: cancelled");
                return;
            }
            Err(error) => {
                log::error!("{name}: task failed: {error}");
                Err("请求任务失败，请重试".into())
            }
        };
        match &result {
            Ok(_) => log::debug!("{name}: completed in {}ms", started.elapsed().as_millis()),
            Err(error) => log::warn!("{name}: {error} ({}ms)", started.elapsed().as_millis()),
        }
        let _ = this.update(cx, |view, cx| {
            let result = if MusicApi::global(cx).session_generation == session {
                result
            } else {
                log::debug!("{name}: result invalidated by session change");
                Err("登录状态已改变，请重试".into())
            };
            apply(view, result, cx);
        });
    })
    .detach();
    abort
}

#[cfg(test)]
mod tests {
    use super::{DataState, Resource};

    fn resource() -> Resource<Vec<u64>> {
        Resource::new()
    }

    #[test]
    fn a_loading_resource_has_no_value_yet() {
        let mut res = resource();
        assert!(matches!(res.state(), DataState::Idle));
        res.begin();
        assert!(res.is_loading());
        assert!(res.value().is_none());
    }

    #[test]
    fn old_data_stays_visible_while_refreshing() {
        let mut res = resource();
        let generation = res.begin();
        assert!(res.apply(generation, Ok(vec![1, 2])));
        // 刷新：旧值不清空，新值到了才换。
        res.begin();
        assert!(res.value().is_some());
        assert!(res.is_loading());
        assert!(matches!(res.state(), DataState::Ready(_)));
        assert_eq!(res.value().unwrap(), &[1, 2]);
        let generation = res.generation;
        assert!(res.apply(generation, Ok(vec![3])));
        assert_eq!(res.value().unwrap(), &[3]);
    }

    #[test]
    fn a_failure_keeps_the_previous_value() {
        let mut res = resource();
        let generation = res.begin();
        assert!(res.apply(generation, Ok(vec![1])));
        let generation = res.begin();
        assert!(res.apply(generation, Err("断网".into())));
        assert_eq!(res.value().unwrap(), &[1]);
        assert_eq!(res.error(), Some("断网"));
        assert!(!res.is_loading());
    }

    #[test]
    fn a_stale_result_is_discarded() {
        let mut res = resource();
        let stale = res.begin();
        res.begin();
        assert!(!res.apply(stale, Ok(vec![1])));
        assert!(res.value().is_none());
        // 过期的失败也不该把错误挂上去。
        assert!(!res.apply(stale, Err("旧请求失败".into())));
        assert!(res.error().is_none());
    }

    #[test]
    fn reset_drops_the_value_and_invalidates_pending_results() {
        let mut res = resource();
        let generation = res.begin();
        assert!(res.apply(generation, Ok(vec![1])));
        res.reset();
        assert!(res.value().is_none());
        // 换资源后在途的结果认不出来了。
        assert!(!res.apply(generation, Ok(vec![2])));
        assert!(res.value().is_none());
    }

    #[test]
    fn set_gives_a_value_without_a_request() {
        let mut res = resource();
        let generation = res.begin();
        res.set(vec![]);
        assert_eq!(res.value().unwrap().len(), 0);
        // 在途的结果认不出来，不会覆盖它。
        assert!(!res.apply(generation, Ok(vec![1])));
        assert_eq!(res.value().unwrap().len(), 0);
    }
    #[tokio::test]
    async fn replacing_setting_resetting_and_dropping_abort_real_tasks() {
        let mut res = resource();
        let task = tokio::spawn(std::future::pending::<()>());
        let stale = res.begin();
        res.track(task.abort_handle());
        res.begin();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!res.apply(stale, Ok(vec![9])));

        let task = tokio::spawn(std::future::pending::<()>());
        res.track(task.abort_handle());
        res.set(vec![1]);
        assert!(task.await.unwrap_err().is_cancelled());

        let task = tokio::spawn(std::future::pending::<()>());
        res.begin();
        res.track(task.abort_handle());
        res.reset();
        assert!(task.await.unwrap_err().is_cancelled());

        let task = tokio::spawn(std::future::pending::<()>());
        res.begin();
        res.track(task.abort_handle());
        drop(res);
        assert!(task.await.unwrap_err().is_cancelled());
    }

    #[test]
    fn empty_success_and_initial_failure_are_different_states() {
        let mut res = resource();
        let generation = res.begin();
        res.apply(generation, Err("offline".into()));
        assert!(matches!(res.state(), DataState::Failed));
        let generation = res.begin();
        res.apply(generation, Ok(vec![]));
        assert!(matches!(res.state(), DataState::Ready(value) if value.is_empty()));
        assert!(!res.apply(generation, Err("duplicate completion".into())));
    }
}
