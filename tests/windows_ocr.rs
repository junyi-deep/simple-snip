#![cfg(target_os = "windows")]

use std::io::{Read, Seek, Write};
use std::process::{Command, Stdio};

#[test]
fn bundled_ocr_worker_handles_a_bad_image_without_starting_the_gui() {
    let mut input = tempfile::tempfile().unwrap();
    input.write_all(&0u32.to_le_bytes()).unwrap();
    input.write_all(&4u32.to_le_bytes()).unwrap();
    input.rewind().unwrap();
    let mut output = tempfile::tempfile().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_simple-snip"))
        .arg("--ocr-worker")
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed().as_secs() >= 10 {
            let _ = child.kill();
            let _ = child.wait();
            panic!("OCR worker did not return");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    assert!(status.success(), "{status}");
    output.rewind().unwrap();
    let mut json = String::new();
    output.read_to_string(&mut json).unwrap();
    let result: Result<snip_core::ocr::OcrResult, String> = serde_json::from_str(&json).unwrap();
    assert!(result.unwrap_err().contains("图片为空"));
}
