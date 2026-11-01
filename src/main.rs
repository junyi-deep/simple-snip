#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
compile_error!("Simple Snip supports only macos-arm64 and win-amd64");
mod animation;
mod app;
#[cfg(debug_assertions)]
mod docs_demo;
mod editor;
mod effects;
mod hotkeys;
mod overlay;
mod perf_probe;
mod pin;
mod settings;
mod task_toolbar;
mod window_pool;
pub fn is_docs_demo() -> bool {
    cfg!(debug_assertions) && std::env::args().any(|arg| arg == "--docs-demo")
}
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "simple_snip=info,snip_platform=info".into()),
        )
        .init();
    let config_path = snip_core::config::Config::path();
    let panic_path = config_path.with_file_name("panic.log");
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Some(parent) = panic_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::File::create(&panic_path) {
            let _ = writeln!(
                file,
                "{info}\n{}",
                std::backtrace::Backtrace::force_capture()
            );
        }
        previous_hook(info);
    }));
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    let perf_test = std::env::args().any(|arg| arg == "--perf-test");
    let docs_test = is_docs_demo();
    if let Some(dir) = config_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let instance = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config_path.with_extension(if docs_test {
            "docs.lock"
        } else if perf_test {
            "perf.lock"
        } else if smoke_test {
            "smoke.lock"
        } else {
            "lock"
        }));
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
        app::start(cx, smoke_test || perf_test || docs_test);
        if std::env::args().any(|arg| arg == "--permission-dialog-test") {
            app::send(cx, app::Event::ScreenPermissionDenied);
        }
        if smoke_test {
            app::smoke_test(cx);
        }
        if perf_test {
            perf_probe::run(cx);
        }
        #[cfg(debug_assertions)]
        if docs_test {
            docs_demo::run(cx);
        }
    });
}
