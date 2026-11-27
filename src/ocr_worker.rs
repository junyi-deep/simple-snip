//! Isolate OCR so a stalled or crashed engine leaves the editor alive.
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use snip_core::ocr::{Cancellation, OcrResult};
use std::{
    io::{self, Read, Seek, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(target_os = "macos")]
const PRECISE_TIMEOUT: Duration = Duration::from_secs(6);
#[cfg(target_os = "macos")]
type Options = snip_core::ocr_config::VisionOptions;
#[cfg(target_os = "windows")]
type Options = snip_core::ocr_config::RapidOptions;
#[cfg(target_os = "macos")]
const ENGINE: &str = "Vision OCR";
#[cfg(target_os = "windows")]
const ENGINE: &str = "Windows OCR";

pub struct Recognition {
    pub result: OcrResult,
    pub notice: Option<&'static str>,
}

#[derive(Debug)]
struct WorkerTimedOut;
impl std::fmt::Display for WorkerTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{ENGINE} 超时，请裁剪图片或切换模式后重试")
    }
}
impl std::error::Error for WorkerTimedOut {}

fn image_byte_count(width: u32, height: u32) -> Result<usize> {
    let length = (u64::from(width) * u64::from(height)).saturating_mul(4);
    ensure!(
        length > 0 && length <= MAX_IMAGE_BYTES,
        "图片为空或超过 OCR 内存上限，请裁剪后重试"
    );
    Ok(length as usize)
}

pub fn recognize(
    executable: &Path,
    image: &RgbaImage,
    options: &Options,
    cancel: &Cancellation,
) -> Result<Recognition> {
    #[cfg(target_os = "macos")]
    if options.accurate {
        let notice = match recognize_once(executable, image, options, cancel, PRECISE_TIMEOUT) {
            Ok(result) if !result.lines.is_empty() => {
                return Ok(Recognition {
                    result,
                    notice: None,
                });
            }
            Ok(_) => "精确模式未识别到文字，已尝试快速识别",
            Err(error) if error.is::<WorkerTimedOut>() => "精确识别超时，已返回快速识别结果",
            Err(error) => return Err(error),
        };
        let mut fallback = options.clone();
        fallback.accurate = false;
        fallback.auto_language = false;
        fallback.correction = false;
        let result = recognize_once(executable, image, &fallback, cancel, TIMEOUT)?;
        return Ok(Recognition {
            result,
            notice: Some(notice),
        });
    }
    let result = recognize_once(executable, image, options, cancel, TIMEOUT)?;
    Ok(Recognition {
        result,
        notice: None,
    })
}

fn recognize_once(
    executable: &Path,
    image: &RgbaImage,
    options: &Options,
    cancel: &Cancellation,
    timeout: Duration,
) -> Result<OcrResult> {
    ensure!(!cancel.cancelled(), "识别已取消");
    image_byte_count(image.width(), image.height())?;
    let mut input = tempfile::tempfile().context("无法创建 OCR 输入")?;
    input.write_all(&image.width().to_le_bytes())?;
    input.write_all(&image.height().to_le_bytes())?;
    let config = serde_json::to_vec(options)?;
    input.write_all(&(config.len() as u32).to_le_bytes())?;
    input.write_all(&config)?;
    input.write_all(image.as_raw())?;
    input.rewind()?;
    let mut output = tempfile::tempfile().context("无法创建 OCR 输出")?;
    let mut command = Command::new(executable);
    command
        .arg("--ocr-worker")
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null());
    run_worker(&mut command, cancel, timeout)?;
    output.rewind()?;
    let mut result = Vec::new();
    output.take(MAX_RESULT_BYTES + 1).read_to_end(&mut result)?;
    ensure!(result.len() as u64 <= MAX_RESULT_BYTES, "OCR 结果过大");
    serde_json::from_slice::<Result<OcrResult, String>>(&result)
        .context("OCR 返回了无效结果，请重试")?
        .map_err(anyhow::Error::msg)
}

fn run_worker(command: &mut Command, cancel: &Cancellation, timeout: Duration) -> Result<()> {
    ensure!(!cancel.cancelled(), "识别已取消");
    let start = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("无法启动 {ENGINE}"))?;
    let result = loop {
        if cancel.cancelled() {
            break Err(anyhow::anyhow!("识别已取消"));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                bail!("{ENGINE} 意外退出（{status}）。请重试或裁剪图片后再识别");
            }
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => break Err(WorkerTimedOut.into()),
            Err(error) => break Err(error).with_context(|| format!("无法读取 {ENGINE} 进程状态")),
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn recognize_input(mut input: impl Read) -> Result<OcrResult> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    let width = u32::from_le_bytes(bytes);
    input.read_exact(&mut bytes)?;
    let height = u32::from_le_bytes(bytes);
    let mut pixels = vec![0; image_byte_count(width, height)?];
    input.read_exact(&mut bytes)?;
    let config_size = u32::from_le_bytes(bytes) as usize;
    ensure!(config_size <= 4096, "OCR 模式配置过大");
    let mut config = vec![0; config_size];
    input.read_exact(&mut config)?;
    let options: Options = serde_json::from_slice(&config)?;
    options.validate()?;
    input.read_exact(&mut pixels)?;
    let image = RgbaImage::from_raw(width, height, pixels).context("无效的 OCR 图片")?;
    #[cfg(target_os = "windows")]
    {
        snip_platform::recognize_with_rapidocr(image, options)
    }
    #[cfg(target_os = "macos")]
    {
        snip_platform::recognize_with_vision(&image, &options, &Cancellation::default())
    }
}

pub fn run_from_stdio() -> Result<()> {
    let mut output = io::BufWriter::new(io::stdout().lock());
    let result = recognize_input(io::BufReader::new(io::stdin().lock()))
        .map_err(|error| format!("{error:#}"));
    serde_json::to_writer(&mut output, &result).context("无法写入 OCR 结果")?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn failed_windows_worker_becomes_a_recoverable_error() {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(std::env::var_os("ComSpec").unwrap());
        command
            .args(["/D", "/C", "exit", "37"])
            .creation_flags(0x0800_0000);
        let error = run_worker(
            &mut command,
            &Cancellation::default(),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Windows OCR 意外退出"));
        assert!(error.to_string().contains("37"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stalled_worker_is_terminated_and_a_new_worker_can_run() {
        let mut stalled = Command::new("/bin/sleep");
        stalled.arg("10");
        let error = run_worker(
            &mut stalled,
            &Cancellation::default(),
            Duration::from_millis(40),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Vision OCR 超时"));
        run_worker(
            &mut Command::new("/usr/bin/true"),
            &Cancellation::default(),
            Duration::from_secs(2),
        )
        .unwrap();
    }
}
