//! Run the Windows OCR engine outside the desktop process. A fatal WinRT
//! failure leaves the editor alive with its current annotations.
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use snip_core::ocr::{Cancellation, OcrEngine, OcrResult};
use std::{
    io::{self, Read, Seek, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

fn image_byte_count(width: u32, height: u32) -> Result<usize> {
    let length = (u64::from(width) * u64::from(height)).saturating_mul(4);
    ensure!(
        length > 0 && length <= MAX_IMAGE_BYTES,
        "图片为空或超过 OCR 内存上限，请裁剪后重试"
    );
    Ok(length as usize)
}

pub fn recognize(image: &RgbaImage, cancel: &Cancellation) -> Result<OcrResult> {
    ensure!(!cancel.cancelled(), "识别已取消");
    image_byte_count(image.width(), image.height())?;
    let mut input = tempfile::tempfile().context("无法创建 OCR 输入")?;
    input.write_all(&image.width().to_le_bytes())?;
    input.write_all(&image.height().to_le_bytes())?;
    input.write_all(image.as_raw())?;
    input.rewind()?;
    let mut output = tempfile::tempfile().context("无法创建 OCR 输出")?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--ocr-worker")
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null());
    run_worker(&mut command, cancel, TIMEOUT)?;
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
    let mut child = command.spawn().context("无法启动 Windows OCR")?;
    let result = loop {
        if cancel.cancelled() {
            break Err(anyhow::anyhow!("识别已取消"));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                bail!("Windows OCR 意外退出（{status}）。请重试或裁剪图片后再识别");
            }
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => break Err(anyhow::anyhow!("Windows OCR 超时，请裁剪图片后重试")),
            Err(error) => break Err(error).context("无法读取 Windows OCR 进程状态"),
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
    input.read_exact(&mut pixels)?;
    let image = RgbaImage::from_raw(width, height, pixels).context("无效的 OCR 图片")?;
    snip_platform::NativeOcr.recognize(&image, &Cancellation::default())
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
}
