use tracing_subscriber::{Layer, filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init() {
    let default = if cfg!(debug_assertions) {
        "warn,netease_music_gpui=debug"
    } else {
        "warn,netease_music_gpui=info"
    };
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| default.into());
    let filter = filter.parse::<Targets>().unwrap_or_else(|_| {
        eprintln!("无效的 RUST_LOG，使用默认日志级别");
        default.parse().unwrap()
    });
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(std::io::stderr)
                .with_filter(filter),
        )
        .try_init();
}
