#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("Simple Snip supports only macos-arm64 and win-amd64");
mod app;
mod editor;
mod effects;
mod hotkeys;
mod overlay;
mod pin;
mod settings;
mod task_toolbar;
mod window_pool;
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "simple_snip=info,snip_platform=info".into()),
        )
        .init();
    let config_path = snip_core::config::Config::path();
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    if let Some(dir) = config_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let instance = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config_path.with_extension(if smoke_test { "smoke.lock" } else { "lock" }));
    let _instance = match instance {
        Ok(file) => {
            if file.try_lock().is_err() {
                tracing::error!("Simple Snip 已在运行，请从系统托盘打开。");
                return;
            }
            file
        }
        Err(e) => {
            tracing::error!("无法创建应用锁: {e}");
            return;
        }
    };
    let application = gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(gpui_kit::QuitMode::Explicit);
    application.on_reopen(app::open_settings);
    application.run(move |cx| {
        gpui_kit::init(cx);
        app::start(cx, smoke_test);
        if std::env::args().any(|arg| arg == "--permission-dialog-test") {
            app::send(cx, app::Event::ScreenPermissionDenied);
        }
        if smoke_test {
            app::smoke_test(cx);
        }
    });
}
