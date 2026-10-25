//! Explicit, offline-until-requested GitHub release checks. No screenshot data
//! enters this module. Passwords are session-only and never part of Config.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use snip_core::config::{ProxyMode, UpdateConfig};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const REPOSITORY: &str = "junyi-deep/simple-snip";
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;
#[derive(Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}
#[derive(Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}
pub fn validate_proxy(config: &UpdateConfig) -> Result<()> {
    if config.proxy_mode == ProxyMode::Http {
        let url = Url::parse(&config.proxy_url).map_err(|_| anyhow::anyhow!("代理地址格式无效"))?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "请输入 HTTP 代理地址，例如 http://127.0.0.1:7890"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "请在独立的用户名和密码框填写认证信息"
        );
        ensure!(
            url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
            "代理地址只应包含协议、主机和端口"
        );
    }
    Ok(())
}
fn client(config: &UpdateConfig, password: &str) -> Result<Client> {
    validate_proxy(config)?;
    let mut builder = Client::builder()
        .user_agent("SimpleSnip-Updater")
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(180))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(6));
    match config.proxy_mode {
        ProxyMode::System => {} // reqwest system-proxy: OS settings + environment.
        ProxyMode::Direct => builder = builder.no_proxy(),
        ProxyMode::Http => {
            let mut proxy = reqwest::Proxy::all(&config.proxy_url)
                .map_err(|_| anyhow::anyhow!("无法设置代理"))?;
            if !config.proxy_username.is_empty() || !password.is_empty() {
                proxy = proxy.basic_auth(&config.proxy_username, password);
            }
            builder = builder.no_proxy().proxy(proxy);
        }
    }
    builder
        .build()
        .map_err(|_| anyhow::anyhow!("无法初始化更新网络连接"))
}
fn request(client: &Client, url: &str) -> Result<reqwest::blocking::Response> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| {
            anyhow::anyhow!(if e.is_timeout() {
                "连接更新服务超时，请检查网络或代理"
            } else {
                "无法连接更新服务，请检查网络、代理或证书"
            })
        })?;
    let status = response.status();
    ensure!(status.is_success(), "更新服务返回 HTTP {}", status.as_u16());
    Ok(response)
}
fn bounded_text(response: impl Read, limit: u64) -> Result<String> {
    let mut bytes = vec![];
    response.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "更新服务响应超过大小限制");
    String::from_utf8(bytes).context("更新服务响应编码无效")
}
pub fn newer(release: &Release, current: &str) -> Result<bool> {
    let latest = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
    let current = semver::Version::parse(current)?;
    Ok(!release.draft && !release.prerelease && latest.pre.is_empty() && latest > current)
}
pub fn check(config: &UpdateConfig, password: &str, current: &str) -> Result<Option<Release>> {
    let client = client(config, password)?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| anyhow::anyhow!("无法检查更新，请检查网络或代理"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    ensure!(
        response.status().is_success(),
        "检查更新返回 HTTP {}",
        response.status().as_u16()
    );
    let release: Release = serde_json::from_str(&bounded_text(response, 2 * 1024 * 1024)?)?;
    if newer(&release, current)? {
        Ok(Some(release))
    } else {
        Ok(None)
    }
}
pub fn target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win-amd64"
    }
}
pub fn package(release: &Release, target: &str) -> Result<Asset> {
    let extension = if target == "macos-arm64" {
        ".dmg"
    } else if target == "win-amd64" {
        ".zip"
    } else {
        bail!("不支持此更新平台");
    };
    release
        .assets
        .iter()
        .find(|a| a.name.contains(target) && a.name.ends_with(extension))
        .cloned()
        .context("该版本尚未提供当前平台的更新包")
}
fn trusted_asset(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("github.com")
            && u.username().is_empty()
            && u.password().is_none()
            && u.path()
                .starts_with(&format!("/{REPOSITORY}/releases/download/"))
    })
}
pub fn download(
    release: &Release,
    config: &UpdateConfig,
    password: &str,
    directory: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    let asset = package(release, target())?;
    ensure!(
        asset.size > 0 && asset.size <= MAX_PACKAGE,
        "更新包大小不符合限制"
    );
    ensure!(
        Path::new(&asset.name).file_name().and_then(|s| s.to_str()) == Some(&asset.name)
            && !asset.name.contains(['/', '\\']),
        "更新包文件名无效"
    );
    let checksum = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .context("该版本缺少 SHA256 校验文件")?;
    ensure!(
        trusted_asset(&asset.browser_download_url) && trusted_asset(&checksum.browser_download_url),
        "拒绝非项目 GitHub Release 的更新地址"
    );
    let client = client(config, password)?;
    let manifest = bounded_text(request(&client, &checksum.browser_download_url)?, 64 * 1024)?;
    let expected = checksum_for(&manifest, &asset.name)?;
    std::fs::create_dir_all(directory)?;
    let mut tmp = tempfile::NamedTempFile::new_in(directory)?;
    let mut response = request(&client, &asset.browser_download_url)?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = response.read(&mut buffer).context("读取更新包失败")?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(
            total <= MAX_PACKAGE && total <= asset.size,
            "更新包超过大小限制"
        );
        hash.update(&buffer[..n]);
        tmp.write_all(&buffer[..n])?;
        progress(total, asset.size);
    }
    ensure!(total == asset.size, "更新包下载不完整");
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "更新包 SHA256 校验失败，请重新下载"
    );
    tmp.as_file().sync_all()?;
    let destination = directory.join(asset.name);
    tmp.persist(&destination).map_err(|e| e.error)?;
    Ok(destination)
}
fn checksum_for(manifest: &str, file: &str) -> Result<String> {
    for line in manifest.lines() {
        let Some((hash, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if name.trim().trim_start_matches('*') == file {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "SHA256 格式无效"
            );
            return Ok(hash.to_ascii_lowercase());
        }
    }
    bail!("校验文件未包含此平台的更新包")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_http_proxy_receives_connect_without_origin_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut data = vec![];
                    let mut byte = [0u8; 1];
                    while socket.read(&mut byte).unwrap_or(0) > 0 && data.len() < 8192 {
                        data.push(byte[0]);
                        if data.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    let headers = String::from_utf8(data).unwrap().to_lowercase();
                    assert!(headers.starts_with("connect example.invalid:443 "));
                    assert!(
                        headers.contains(
                            "proxy-authorization: basic dGVzdDpwYXNz"
                                .to_lowercase()
                                .as_str()
                        )
                    );
                    assert!(!headers.contains("\r\nauthorization:"));
                    let _ = socket.write_all(
                        b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                    );
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "proxy connection not received"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let config = UpdateConfig {
            proxy_mode: ProxyMode::Http,
            proxy_url: format!("http://{address}"),
            proxy_username: "test".into(),
        };
        assert!(
            client(&config, "pass")
                .unwrap()
                .get("https://example.invalid")
                .send()
                .is_err()
        );
        server.join().unwrap();
    }
    fn release() -> Release {
        Release {
            tag_name: "v0.0.2".into(),
            html_url: format!("https://github.com/{REPOSITORY}/releases/tag/v0.0.2"),
            draft: false,
            prerelease: false,
            assets: vec![],
        }
    }
    #[test]
    fn versions_and_channels() {
        let mut r = release();
        assert!(newer(&r, "0.0.1").unwrap());
        assert!(!newer(&r, "0.1.0").unwrap());
        r.prerelease = true;
        assert!(!newer(&r, "0.0.1").unwrap());
        r.prerelease = false;
        r.tag_name = "v0.0.3-beta.1".into();
        assert!(!newer(&r, "0.0.1").unwrap());
    }
    #[test]
    fn proxy_modes_and_credentials_not_in_url() {
        let mut c = UpdateConfig::default();
        assert_eq!(c.proxy_mode, ProxyMode::System);
        assert!(client(&c, "").is_ok());
        c.proxy_mode = ProxyMode::Direct;
        assert!(client(&c, "").is_ok());
        c.proxy_mode = ProxyMode::Http;
        c.proxy_url = "http://127.0.0.1:7890".into();
        assert!(client(&c, "test-password").is_ok());
        c.proxy_url = "http://user:secret@127.0.0.1:7890".into();
        assert!(validate_proxy(&c).is_err());
        c.proxy_url = "file:///tmp/test".into();
        assert!(validate_proxy(&c).is_err());
    }
    #[test]
    fn checksum_and_asset_trust() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum_for(&format!("{hash}  app.dmg\n"), "app.dmg").unwrap(),
            hash
        );
        assert!(checksum_for("bad  app.dmg", "app.dmg").is_err());
        assert!(checksum_for("", "app.dmg").is_err());
        assert!(trusted_asset(&format!(
            "https://github.com/{REPOSITORY}/releases/download/v0.0.2/app.dmg"
        )));
        assert!(!trusted_asset("https://example.com/app.dmg"));
        let mut r = release();
        for target in ["win-amd64", "macos-arm64"] {
            r.assets.push(Asset {
                name: format!(
                    "app_{target}{}",
                    if target == "win-amd64" {
                        ".zip"
                    } else {
                        ".dmg"
                    }
                ),
                browser_download_url: String::new(),
                size: 10,
            });
        }
        assert!(package(&r, "win-amd64").unwrap().name.ends_with(".zip"));
        assert!(package(&r, "macos-arm64").unwrap().name.ends_with(".dmg"));
        assert!(package(&r, "linux").is_err());
    }
}
