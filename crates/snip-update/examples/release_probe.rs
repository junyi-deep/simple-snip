//! Native integration probe against the public release, intentionally comparing
//! against 0.0.0 so the first published version exercises the download path.
fn main() -> anyhow::Result<()> {
    let config = snip_core::config::UpdateConfig::default();
    let release = snip_update::check(&config, "", "0.0.0")?
        .ok_or_else(|| anyhow::anyhow!("No public release"))?;
    println!("Release {} / {}", release.tag_name, snip_update::target());
    if std::env::args().any(|a| a == "--download") {
        let directory = std::env::temp_dir().join("simple-snip-update-probe");
        let path = snip_update::download(&release, &config, "", &directory, |_, _| {})?;
        println!("Downloaded and SHA256 verified: {}", path.display());
    }
    Ok(())
}
