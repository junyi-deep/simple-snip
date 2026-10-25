//! Synthetic data for manual F3/transform tests. Never uses user screen content.
use snip_platform::ClipboardProvider;
fn main() -> anyhow::Result<()> {
    let image = image::RgbaImage::from_fn(640, 400, |x, y| {
        let edge = x < 8 || y < 8 || x >= 632 || y >= 392;
        let stripe = x > 100 && x < 540 && y > 180 && y < 220;
        image::Rgba(if edge {
            [240, 70, 90, 255]
        } else if stripe {
            [255, 255, 255, 255]
        } else {
            [(x * 255 / 640) as u8, 150, (y * 255 / 400) as u8, 255]
        })
    });
    snip_platform::NativeClipboard.write_image(&image)?;
    println!("Synthetic 640x400 RGBA fixture copied; press F3 in Simple Snip.");
    Ok(())
}
