use snip_core::{
    annotation::{Annotation, Document, Tool},
    geometry::Point,
    ocr::{Cancellation, OcrEngine},
};
use snip_platform::{NativeCapture, ScreenCapture};
use std::sync::Arc;

#[test]
#[ignore = "Requires an unlocked desktop and Screen Recording permission"]
fn native_capture_three_times() {
    for _ in 0..3 {
        let displays = NativeCapture
            .capture_desktop()
            .expect("native desktop capture");
        assert!(!displays.is_empty());
        for display in displays {
            assert!(display.image.width() > 0 && display.image.height() > 0);
            assert_eq!(display.map.pixel_width, display.image.width());
            eprintln!(
                "display {}: {}x{}, desktop {:?}",
                display.id,
                display.image.width(),
                display.image.height(),
                display.map.desktop
            );
        }
    }
}
#[test]
#[ignore = "Uses the native OCR runtime; needs installed system OCR languages/fonts"]
fn native_ocr_synthetic_bilingual_fixture() {
    let bytes = snip_platform::annotation_font().unwrap();
    let font = snip_core::fontdue::Font::from_bytes(bytes, Default::default()).unwrap();
    let mut doc = Document::new(Arc::new(image::RgbaImage::from_pixel(
        1200,
        240,
        image::Rgba([255; 4]),
    )));
    doc.font = Some(Arc::new(font));
    doc.add(Annotation {
        tool: Tool::Text,
        points: vec![Point::new(40., 50.)],
        color: [0, 0, 0, 255],
        width: 6.,
        text: "Offline 2026 测试文字".into(),
        arrow_style: Default::default(),
    });
    let image = doc.flatten();
    let result = snip_platform::NativeOcr
        .recognize(&image, &Cancellation::default())
        .expect("native OCR");
    eprintln!("Synthetic fixture recognized: {}", result.text());
    assert!(result.text().contains("2026"));
    assert!(result.text().to_lowercase().contains("offline"));
    assert!(result.text().contains("测试"));
    assert!(
        result
            .lines
            .iter()
            .all(|l| !l.words.is_empty() && l.bounds.width > 0.)
    );
}

#[test]
#[ignore = "Requires Screen Recording permission"]
fn native_region_retina_odd_pixel_bounds() {
    let display = NativeCapture.capture_desktop().unwrap().remove(0);
    let image = snip_platform::capture_region(
        display.id,
        snip_core::geometry::Rect::new(13., 17., 257., 129.),
    )
    .unwrap();
    assert_eq!(image.dimensions(), (257, 129));
}
