#![cfg(target_os = "macos")]

#[path = "../src/ocr_worker.rs"]
pub mod ocr_worker;

use image::{Rgba, RgbaImage};
use snip_core::{
    annotation::{Annotation, Document, Tool},
    geometry::Point,
    ocr::Cancellation,
    ocr_config::OcrConfig,
};
use std::{path::Path, sync::Arc, time::Instant};

#[test]
fn vision_worker_recognizes_the_bundled_fixture_with_each_preset() {
    let source = image::load_from_memory(include_bytes!("fixtures/ocr-bilingual.png"))
        .unwrap()
        .into_rgba8();
    let executable = Path::new(env!("CARGO_BIN_EXE_simple-snip"));
    for mode in 0..3 {
        let preset = OcrConfig::preset(mode);
        let recognized = ocr_worker::recognize(
            executable,
            &source,
            &preset.vision,
            &Cancellation::default(),
        )
        .expect("bundled Vision fixture through the production worker");
        let text = recognized.result.text();
        assert!(
            text.to_lowercase().contains("offline") && text.contains("2026"),
            "Vision preset {mode}: {text}"
        );
        if mode > 0 {
            if let Some(notice) = recognized.notice {
                eprintln!("Vision preset {mode}: {notice}。本轮未验证精确模式的中文结果。");
            } else {
                assert!(
                    text.contains("测试"),
                    "Vision precise preset {mode}: {text}"
                );
            }
        }
    }
}

#[test]
fn vision_worker_recognizes_a_dense_code_image_in_precise_mode() {
    let source = Arc::new(RgbaImage::from_pixel(1440, 970, Rgba([26, 30, 37, 255])));
    let mut document = Document::new(source);
    let font = std::fs::read("/System/Library/Fonts/Menlo.ttc").unwrap();
    document.font = Some(Arc::new(
        snip_core::fontdue::Font::from_bytes(font, Default::default()).unwrap(),
    ));
    for index in 0..32 {
        document.objects.push(Annotation {
            tool: Tool::Text,
            points: vec![Point::new(16., 12. + 29. * index as f32)],
            color: [224, 235, 250, 255],
            width: 2.,
            text: format!("func formatRecord{index}(value int) int {{ return value + {index} }}"),
            arrow_style: Default::default(),
            watermark: Default::default(),
        });
    }
    let image = document.flatten();
    let mut preset = OcrConfig::preset(2).vision;
    preset.correction = false;
    preset.auto_language = false;
    let start = Instant::now();
    let ocr_worker::Recognition { result, notice } = ocr_worker::recognize(
        Path::new(env!("CARGO_BIN_EXE_simple-snip")),
        &image,
        &preset,
        &Cancellation::default(),
    )
    .unwrap();
    eprintln!(
        "Vision code fixture: {} lines in {:?}, fallback: {}",
        result.lines.len(),
        start.elapsed(),
        notice.unwrap_or("none"),
    );
    assert!(
        result.lines.len() >= 24,
        "Vision returned only {} lines",
        result.lines.len()
    );
    assert!(result.text().contains("formatRecord"));
}
