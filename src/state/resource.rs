//! 一份远端数据的统一形态。
//!
//! 显示层只管读数据，不关心它存不存在、在不在加载中；数据层的规则只有两条：
//! 「拉取，然后更新数据」。请求发出时旧数据留着（先显示旧的，新的到了再换），
//! 只有成功才替换值——失败时旧内容还在，只是多了一条错误。
//!
//! 各 store 各手搓过一遍「代次 + 在途句柄 + apply/finish」，形状高度一致，
//! 这里收成一处。

use tokio::task::AbortHandle;

/// 一份远端数据。
///
/// - `value` 有值就显示它，哪怕正在刷新；
/// - `value` 没有时看 `loading` / `error`；
/// - 每次加载递增代次，迟到的旧结果被丢掉——abort 撤不走在途的结果，
///   已经排进主线程的结果只能靠代次认出来。
pub struct Resource<T> {
    /// 最近一次拿到的数据。`None` = 还没拿到过。
    value: Option<T>,
    loading: bool,
    error: Option<String>,
    /// 请求代次：一次加载一个，迟到的结果靠它认出来。
    generation: u64,
    /// 在途请求的句柄：换资源或销毁时取消。
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

    /// 带初值建一份资源：占位内容先摆上，数据到了再替换。
    pub fn with_value(value: T) -> Self {
        Self {
            value: Some(value),
            ..Self::new()
        }
    }

    /// 开始一次加载。旧数据留着：先显示旧的，新的到了再换。
    ///
    /// 返回这次加载的代次，结果回来时用它核对。
    pub fn begin(&mut self) -> u64 {
        self.loading = true;
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    /// 记下在途请求：下一次 `begin`（换资源、重新加载）或 `abort` 会取消它。
    pub fn track(&mut self, request: AbortHandle) {
        self.request = Some(request);
    }

    /// 结果是不是这次加载的。过期的一律丢掉。
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// 落地结果。成功替换值，失败只记错误（旧值留着）。
    ///
    /// 返回结果有没有被采纳——过期结果返回 false，调用方据此决定要不要重绘。
    pub fn apply(&mut self, generation: u64, result: Result<T, String>) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.request = None;
        self.loading = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    /// 直接给一份数据，不走请求：游客这种「确定没有」的场合用。
    ///
    /// 代次照样递增，在途的结果回来时认不出来，不会覆盖它。
    pub fn set(&mut self, value: T) {
        self.generation = self.generation.wrapping_add(1);
        self.value = Some(value);
        self.error = None;
        self.loading = false;
    }

    /// 换资源：旧值和错误都丢掉。在途结果也一并作废。
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.value = None;
        self.error = None;
        self.loading = false;
        self.abort();
    }

    /// 取消在途请求。已经送达主线程的结果撤不走，靠代次挡住。
    pub fn abort(&mut self) {
        if let Some(request) = self.request.take() {
            request.abort();
        }
    }

    /// 显示状态。
    pub fn state(&self) -> DataState<'_, T> {
        match &self.value {
            Some(value) => DataState::Ready(value),
            None => match &self.error {
                Some(error) => DataState::Failed(error),
                None => DataState::Loading,
            },
        }
    }

    /// 拿数据。没拿到过就是 `None`。
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// 可变地拿数据（「一键已读」这类纯本地修改用）。
    pub fn value_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }

    /// 正在拉（包括「刷新中」：旧值还在，但新的在路上）。
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// 手上有旧数据、同时正在刷新。显示层可以用它画一个「刷新中」的角标。
    pub fn is_refreshing(&self) -> bool {
        self.loading && self.value.is_some()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl<T> Default for Resource<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// 一份数据的显示状态。
pub enum DataState<'a, T> {
    /// 正在拉，还没有拿到过。
    Loading,
    /// 拉失败了。
    Failed(&'a str),
    /// 拿到了。内容空不空由数据自己说。
    Ready(&'a T),
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
        assert!(matches!(res.state(), DataState::Loading));
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
        assert!(res.is_refreshing());
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
}
