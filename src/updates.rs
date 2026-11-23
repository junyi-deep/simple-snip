use crate::{app::Runtime, settings::Settings};
use gpui_kit::*;
use snip_core::config::UpdateConfig;
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) password: String,
    pub(crate) checking: bool,
    pub(crate) release: Option<snip_update::Release>,
    view: Option<(AnyWindowHandle, WeakEntity<Settings>)>,
}
impl Global for Updates {}

pub(crate) fn init(cx: &mut App) {
    if cx.try_global::<Updates>().is_none() {
        cx.set_global(Updates::default());
    }
}

pub(crate) fn attach(window: AnyWindowHandle, view: &Entity<Settings>, cx: &mut App) {
    cx.global_mut::<Updates>().view = Some((window, view.downgrade()));
    cx.defer(move |cx| show_result(CheckFeedback::Existing, cx));
}

pub(crate) fn open_section(section: usize, cx: &mut App) {
    crate::app::open_settings(cx);
    let target = cx.try_global::<Updates>().and_then(|v| v.view.clone());
    if let Some((window, view)) = target {
        let _ = window.update(cx, |_, _, cx| {
            let _ = view.update(cx, |v, cx| v.show_section(section, cx));
        });
    }
}

pub(crate) fn start(cx: &mut App) {
    init(cx);
    schedule(check, cx);
}

fn schedule(probe: fn(UpdateConfig, bool, &mut App), cx: &mut App) {
    let tick = move |cx: &mut App| {
        let config = cx.global::<Runtime>().config.updates.clone();
        if config.auto_check {
            probe(config, false, cx);
        }
    };
    tick(cx);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(INTERVAL).await;
            cx.update(tick);
        }
    })
    .detach();
}

pub(crate) fn automatic(cx: &mut App) {
    let config = cx.global::<Runtime>().config.updates.clone();
    if config.auto_check {
        check(config, false, cx);
    }
}

enum CheckFeedback {
    Existing,
    Available,
    Latest,
    Failed(String),
}

fn show_result(feedback: CheckFeedback, cx: &mut App) {
    let state = cx.global::<Updates>();
    let (Some((window, view)), release) = (state.view.clone(), state.release.clone()) else {
        return;
    };
    let _ = window.update(cx, |_, w, cx| {
        let _ = view.update(cx, |settings, cx| {
            match feedback {
                CheckFeedback::Existing | CheckFeedback::Available => {
                    if let Some(release) = release {
                        settings.notify_update(&release.tag_name, w, cx);
                    }
                }
                CheckFeedback::Latest => crate::ui::notify(w, "已是最新版本", false, cx),
                CheckFeedback::Failed(error) => crate::ui::notify(w, error, true, cx),
            }
            cx.notify();
        });
    });
}

pub(crate) fn check(config: UpdateConfig, manual: bool, cx: &mut App) {
    let state = cx.global_mut::<Updates>();
    if state.checking {
        return;
    }
    state.checking = true;
    let password = state.password.clone();
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        let result = smol::unblock(move || {
            snip_update::check(&config, &password, env!("CARGO_PKG_VERSION"))
        })
        .await;
        cx.update(|cx| {
            let still_enabled = cx.global::<Runtime>().config.updates.auto_check;
            let state = cx.global_mut::<Updates>();
            state.checking = false;
            let feedback = match result {
                Ok(Some(release)) if manual || still_enabled => {
                    let changed = state
                        .release
                        .as_ref()
                        .is_none_or(|old| old.tag_name != release.tag_name);
                    state.release = Some(release);
                    (changed || manual).then_some(CheckFeedback::Available)
                }
                Ok(None) if manual || still_enabled => {
                    state.release = None;
                    manual.then_some(CheckFeedback::Latest)
                }
                Err(error) if manual => {
                    Some(CheckFeedback::Failed(format!("检查更新失败：{error}")))
                }
                _ => None,
            };
            if let Some(feedback) = feedback {
                show_result(feedback, cx);
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

#[cfg(debug_assertions)]
pub(crate) fn fixture_notice(cx: &mut App) {
    cx.global_mut::<Updates>().release = Some(snip_update::Release {
        tag_name: "v9.9.9".into(),
        html_url: String::new(),
        draft: false,
        prerelease: false,
        assets: vec![],
    });
    cx.defer(|cx| show_result(CheckFeedback::Available, cx));
}

#[cfg(test)]
mod tests {
    use super::{Global, INTERVAL, Runtime, schedule};
    use gpui_kit::TestAppContext;
    use std::time::Duration;

    #[derive(Default)]
    struct Calls(u32);
    impl Global for Calls {}

    #[gpui_kit::test]
    fn startup_and_hourly_checks_respect_live_preference_without_a_window(cx: &mut TestAppContext) {
        let _rx = cx.update(crate::app::init_test_runtime);
        cx.update(|cx| {
            cx.set_global(Calls::default());
            schedule(
                |_, manual, cx| {
                    assert!(!manual);
                    cx.global_mut::<Calls>().0 += 1;
                },
                cx,
            );
            assert_eq!(cx.global::<Calls>().0, 1);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(INTERVAL - Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.global::<Calls>().0, 1));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(cx.global::<Calls>().0, 2);
            cx.global_mut::<Runtime>().config.updates.auto_check = false;
        });
        cx.executor().advance_clock(INTERVAL);
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.global::<Calls>().0, 2));
    }
}
