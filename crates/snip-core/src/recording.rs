use crate::config::GifConfig;
use anyhow::{Result, ensure};
use image::{RgbaImage, imageops};
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub struct TimedFrame {
    pub image: RgbaImage,
    pub timestamp: Duration,
}
#[derive(Clone, Default)]
pub struct RecordingControl {
    pub stop: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub cancelled: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
    pub elapsed_ms: Arc<AtomicU64>,
}
impl RecordingControl {
    pub fn finish(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.finish();
    }
    pub fn pause(&self, value: bool) {
        self.paused.store(value, Ordering::Relaxed);
    }
}
pub fn delay_centiseconds(delta: Duration) -> u16 {
    ((delta.as_millis() + 5) / 10).clamp(2, u16::MAX as u128) as u16
}
/// The producer never blocks the GUI or accumulates an unbounded frame vector.
/// Actual capture timestamps determine delays; overload drops frames, not time.
pub fn record_gif(
    config: GifConfig,
    path: PathBuf,
    control: RecordingControl,
    mut capture: impl FnMut() -> Result<RgbaImage> + Send + 'static,
    complete: impl FnOnce(Result<PathBuf>) + Send + 'static,
) {
    thread::spawn(move || {
        if ![5, 10, 15, 20, 30].contains(&config.fps)
            || !(1..=600).contains(&config.max_seconds)
            || !(64..=4096).contains(&config.max_width)
        {
            control.done.store(true, Ordering::Relaxed);
            complete(Err(anyhow::anyhow!("录制参数超出允许范围")));
            return;
        }
        let (tx, rx) = mpsc::sync_channel::<TimedFrame>(3);
        let partial = path.with_extension("gif.part");
        let output = partial.clone();
        let options = config.clone();
        let encoder = thread::spawn(move || -> Result<()> {
            let mut pending: Option<TimedFrame> = None;
            let mut last_timestamp = Duration::ZERO;
            let mut dimensions = None;
            let mut writer: Option<gif::Encoder<File>> = None;
            let write = |frame: TimedFrame,
                         delay: u16,
                         writer: &mut Option<gif::Encoder<File>>|
             -> Result<()> {
                let (fw, fh) = frame.image.dimensions();
                let mut pixels = frame.image.into_raw();
                let w = writer.as_ref().context("GIF 编码器未初始化")?;
                let _ = w;
                let enc = writer.as_mut().unwrap();
                let mut f = gif::Frame::from_rgba_speed(fw as u16, fh as u16, &mut pixels, 10);
                f.delay = delay;
                enc.write_frame(&f)?;
                Ok(())
            };
            for mut frame in rx {
                last_timestamp = frame.timestamp;
                let width = frame.image.width().min(options.max_width);
                let height = ((frame.image.height() as f64 * width as f64
                    / frame.image.width() as f64)
                    .round() as u32)
                    .max(1);
                ensure!(
                    width <= u16::MAX as u32 && height <= u16::MAX as u32,
                    "GIF 尺寸超出格式限制"
                );
                if frame.image.width() != width {
                    frame.image = imageops::resize(
                        &frame.image,
                        width,
                        height,
                        imageops::FilterType::Triangle,
                    );
                }
                if let Some(expected) = dimensions {
                    ensure!(expected == (width, height), "录制期间图像尺寸改变");
                }
                dimensions = Some((width, height));
                if writer.is_none() {
                    let mut e = gif::Encoder::new(
                        File::create(&output)?,
                        width as u16,
                        height as u16,
                        &[],
                    )?;
                    e.set_repeat(if options.repeat == 0 {
                        gif::Repeat::Infinite
                    } else {
                        gif::Repeat::Finite(options.repeat)
                    })?;
                    writer = Some(e);
                }
                if let Some(old) = pending.take() {
                    if old.image == frame.image {
                        pending = Some(old);
                        continue;
                    }
                    let delay = delay_centiseconds(frame.timestamp.saturating_sub(old.timestamp));
                    write(old, delay, &mut writer)?;
                }
                pending = Some(frame);
            }
            ensure!(pending.is_some(), "录制未采集到任何帧");
            if let Some(frame) = pending {
                let remaining = last_timestamp.saturating_sub(frame.timestamp)
                    + Duration::from_secs_f64(1. / options.fps as f64);
                write(frame, delay_centiseconds(remaining), &mut writer)?;
            }
            Ok(())
        });
        let result = (|| -> Result<()> {
            let start = Instant::now();
            let interval = Duration::from_secs_f64(1. / config.fps as f64);
            let mut paused = Duration::ZERO;
            let mut frames = 0u32;
            while !control.stop.load(Ordering::Relaxed)
                && start.elapsed().saturating_sub(paused)
                    < Duration::from_secs(config.max_seconds as u64)
            {
                if control.paused.load(Ordering::Relaxed) {
                    let pause_start = Instant::now();
                    thread::sleep(Duration::from_millis(50));
                    paused += pause_start.elapsed();
                    continue;
                }
                control.elapsed_ms.store(
                    start.elapsed().saturating_sub(paused).as_millis() as u64,
                    Ordering::Relaxed,
                );
                let tick = Instant::now();
                let mut image = capture()?;
                ensure!(
                    image.as_raw().len() <= 128 * 1024 * 1024,
                    "录制区域过大，超过单帧 128 MiB 保护限制"
                );
                if image.width() > config.max_width {
                    let height = (image.height() as f64 * config.max_width as f64
                        / image.width() as f64)
                        .round()
                        .max(1.) as u32;
                    image = imageops::resize(
                        &image,
                        config.max_width,
                        height,
                        imageops::FilterType::Triangle,
                    );
                }
                ensure!(
                    image.as_raw().len() <= 32 * 1024 * 1024,
                    "输出帧超过 32 MiB 上限，请降低输出尺寸"
                );
                offer_frame(
                    &tx,
                    TimedFrame {
                        image,
                        timestamp: start.elapsed().saturating_sub(paused),
                    },
                )?;
                frames += 1;
                if frames >= config.fps * config.max_seconds {
                    break;
                }
                if let Some(rest) = interval.checked_sub(tick.elapsed()) {
                    thread::sleep(rest);
                }
            }
            Ok(())
        })();
        drop(tx);
        let encoded = encoder
            .join()
            .map_err(|_| anyhow::anyhow!("GIF 编码线程异常退出"))
            .and_then(|x| x);
        let result = result.and(encoded).and_then(|_| {
            std::fs::rename(&partial, &path)?;
            Ok(path.clone())
        });
        control.done.store(true, Ordering::Relaxed);
        if control.cancelled.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&partial);
            let _ = std::fs::remove_file(&path);
            complete(Err(anyhow::anyhow!("录制已取消")));
        } else {
            complete(result.map_err(|e| {
                anyhow::anyhow!("{e}；如有已编码帧，可尝试恢复 {}", partial.display())
            }));
        }
    });
}
use anyhow::Context;
fn offer_frame(tx: &SyncSender<TimedFrame>, frame: TimedFrame) -> Result<()> {
    match tx.try_send(frame) {
        Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
        Err(mpsc::TrySendError::Disconnected(_)) => anyhow::bail!("编码线程已停止"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_delays() {
        assert_eq!(delay_centiseconds(Duration::from_millis(67)), 7);
        assert_eq!(delay_centiseconds(Duration::from_millis(1)), 2);
        assert_eq!(delay_centiseconds(Duration::from_millis(200)), 20);
    }
    #[test]
    fn bounded_encode_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.gif");
        let (tx, rx) = mpsc::channel();
        let mut i = 0;
        record_gif(
            GifConfig {
                fps: 5,
                max_seconds: 1,
                countdown: 0,
                ..Default::default()
            },
            path.clone(),
            RecordingControl::default(),
            move || {
                i += 1;
                Ok(RgbaImage::from_pixel(
                    32,
                    20,
                    image::Rgba([i, 50, 100, 255]),
                ))
            },
            move |r| {
                tx.send(r).unwrap();
            },
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
            path
        );
        let decoder = gif::DecodeOptions::new()
            .read_info(File::open(path).unwrap())
            .unwrap();
        assert_eq!(decoder.width(), 32);
    }
    #[test]
    fn duplicate_frames_preserve_duration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("static.gif");
        let (tx, rx) = mpsc::channel();
        record_gif(
            GifConfig {
                fps: 5,
                max_seconds: 1,
                countdown: 0,
                ..Default::default()
            },
            path.clone(),
            RecordingControl::default(),
            || {
                Ok(RgbaImage::from_pixel(
                    16,
                    16,
                    image::Rgba([50, 100, 150, 255]),
                ))
            },
            move |result| {
                tx.send(result).unwrap();
            },
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        let mut decoder = gif::DecodeOptions::new()
            .read_info(File::open(path).unwrap())
            .unwrap();
        let mut delay = 0;
        let mut frames = 0;
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            delay += frame.delay;
            frames += 1;
        }
        assert_eq!(frames, 1);
        assert!((95..=130).contains(&delay), "delay was {delay}");
    }
}
