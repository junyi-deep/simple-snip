//! Bounded GIF decoding shared by animated pins and region previews.
use crate::app;
use gpui_kit::RenderImage;
use image::{AnimationDecoder, RgbaImage};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub temporary: Option<Arc<tempfile::TempDir>>,
}
impl Source {
    pub fn file(path: PathBuf) -> Self {
        Self {
            path,
            temporary: None,
        }
    }
}
pub struct Frame {
    pub pixels: Arc<RgbaImage>,
    pub texture: Arc<RenderImage>,
    pub delay: Duration,
}
pub type FrameResult = Result<Frame, String>;
pub struct Playback {
    pub source: Source,
    pub receiver: async_channel::Receiver<FrameResult>,
    pub stopped: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub display_scale: Arc<AtomicU32>,
}
impl Playback {
    pub fn new(source: Source) -> Self {
        let (receiver, stopped, paused, display_scale, _worker) = stream(source.clone());
        Self {
            source,
            receiver,
            stopped,
            paused,
            display_scale,
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.receiver.close();
    }
}
pub type Stream = (
    async_channel::Receiver<FrameResult>,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    Arc<AtomicU32>,
    std::thread::JoinHandle<()>,
);
pub fn stream(source: Source) -> Stream {
    let stopped = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let display_scale = Arc::new(AtomicU32::new(1f32.to_bits()));
    let scale = display_scale.clone();
    let (stop, pause) = (stopped.clone(), paused.clone());
    let (tx, rx) = async_channel::bounded(1);
    let worker = std::thread::spawn(move || {
        let _keep_temporary = source.temporary;
        while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
            let result = (|| -> anyhow::Result<()> {
                let reader = std::io::BufReader::new(std::fs::File::open(&source.path)?);
                let decoder = image::codecs::gif::GifDecoder::new(reader)?;
                let mut count = 0;
                for frame in decoder.into_frames() {
                    while pause.load(Ordering::Relaxed)
                        && !stop.load(Ordering::Relaxed)
                        && !tx.is_closed()
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if stop.load(Ordering::Relaxed) || tx.is_closed() {
                        return Ok(());
                    }
                    let frame = frame?;
                    let (n, d) = frame.delay().numer_denom_ms();
                    let delay = Duration::from_millis((u64::from(n) / u64::from(d.max(1))).max(10));
                    let pixels = Arc::new(frame.into_buffer());
                    // Scale ordinary GIFs on the GPU. Resampling every small
                    // frame in software costs more CPU than it saves.
                    let display_scale = if pixels.width() <= 1280 && pixels.height() <= 1280 {
                        1.
                    } else {
                        f32::from_bits(scale.load(Ordering::Relaxed))
                    };
                    let texture = app::texture_scaled(&pixels, display_scale);
                    count += 1;
                    if tx
                        .send_blocking(Ok(Frame {
                            pixels,
                            texture,
                            delay,
                        }))
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                anyhow::ensure!(count > 0, "GIF 没有帧");
                Ok(())
            })();
            if let Err(error) = result {
                let _ = tx.send_blocking(Err(error.to_string()));
                break;
            }
        }
    });
    (rx, stopped, paused, display_scale, worker)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_a_paused_gif_releases_its_temporary_source_and_bounded_queue() {
        let directory = Arc::new(tempfile::tempdir().unwrap());
        let weak = Arc::downgrade(&directory);
        let path = directory.path().join("loop.gif");
        {
            let mut encoder =
                image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    4,
                    4,
                    image::Rgba([10, 20, 30, 255]),
                )))
                .unwrap();
        }
        let player = Playback::new(Source {
            path,
            temporary: Some(directory.clone()),
        });
        drop(directory);
        let frame = player.receiver.recv_blocking().unwrap().unwrap();
        assert_eq!(frame.pixels.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(&frame.texture.as_bytes(0).unwrap()[..4], &[30, 20, 10, 255]);
        player.paused.store(true, Ordering::Relaxed);
        assert!(player.receiver.len() <= 1);
        drop(player);
        for _ in 0..200 {
            if weak.upgrade().is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("closed GIF decoder retained its temporary file");
    }
}
