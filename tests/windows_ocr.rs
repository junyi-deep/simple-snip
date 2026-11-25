#![cfg(target_os = "windows")]

use std::io::{Read, Seek, Write};
use std::process::{Command, Stdio};

#[test]
fn bundled_ocr_worker_handles_a_bad_image_without_starting_the_gui() {
    let mut input = tempfile::tempfile().unwrap();
    input.write_all(&0u32.to_le_bytes()).unwrap();
    input.write_all(&4u32.to_le_bytes()).unwrap();
    let result = worker(input);
    assert!(result.unwrap_err().contains("图片为空"));
}

#[test]
fn bundled_rapidocr_worker_reads_bilingual_text_offline() {
    use snip_core::ocr_config::OcrConfig;
    let source = image::load_from_memory(include_bytes!("fixtures/ocr-bilingual.png"))
        .unwrap()
        .into_rgba8();
    for index in 0..3 {
        let mut input = tempfile::tempfile().unwrap();
        let mode = serde_json::to_vec(&OcrConfig::preset(index).rapid).unwrap();
        for value in [source.width(), source.height(), mode.len() as u32] {
            input.write_all(&value.to_le_bytes()).unwrap();
        }
        input.write_all(&mode).unwrap();
        input.write_all(source.as_raw()).unwrap();
        let result = worker(input).unwrap();
        let text = result.text();
        assert!(
            text.to_lowercase().contains("offline")
                && text.contains("2026")
                && text.contains("测试"),
            "profile {index}: {text}"
        );
        assert!(
            result
                .lines
                .iter()
                .all(|line| line.bounds.width > 0. && !line.words.is_empty())
        );
    }
}

fn worker(mut input: std::fs::File) -> Result<snip_core::ocr::OcrResult, String> {
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
        if started.elapsed().as_secs() >= 30 {
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
    serde_json::from_str(&json).unwrap()
}
