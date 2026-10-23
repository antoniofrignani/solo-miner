#[path = "../src/icon.rs"]
mod icon;

fn main() -> anyhow::Result<()> {
    let output = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!("Usage: cargo run --release --example app_icon -- OUTPUT.png")
    })?;
    image::save_buffer(
        output,
        &icon::logo_rgba(1024),
        1024,
        1024,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
