use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const RELEASE_API: &str = "https://api.github.com/repos/Trigger-CN/trepo/releases/latest";
const USER_AGENT: &str = concat!("trepo/", env!("CARGO_PKG_VERSION"));
const CHECKSUM_ASSET: &str = "SHA256SUMS";
const MAX_ARCHIVE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn run(check_only: bool) -> Result<()> {
    let platform = platform_id()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).context("invalid current version")?;
    println!("Checking GitHub Releases for trepo updates...");

    let release: Release = request(RELEASE_API, MAX_METADATA_BYTES)?;
    let latest = parse_tag(&release.tag_name)?;
    if latest <= current {
        println!("trepo {current} is up to date.");
        return Ok(());
    }
    println!("Update available: {current} -> {latest}");
    if check_only {
        return Ok(());
    }

    let archive_name = archive_name(&release.tag_name, platform);
    let archive_asset = find_asset(&release, &archive_name)?;
    let checksum_asset = find_asset(&release, CHECKSUM_ASSET)?;
    let sums = request(&checksum_asset.browser_download_url, MAX_METADATA_BYTES)?;
    let sums = String::from_utf8(sums).context("SHA256SUMS is not valid UTF-8")?;
    let expected = checksum_for(&sums, &archive_name)?;

    println!("Downloading {archive_name}...");
    let archive: Vec<u8> = request(&archive_asset.browser_download_url, MAX_ARCHIVE_BYTES)?;
    verify_checksum(&archive, &expected)?;

    let staging = tempfile::tempdir().context("failed to create update staging directory")?;
    let new_binary = staging.path().join(binary_name(platform));
    extract_binary(
        &archive,
        &archive_name,
        &release.tag_name,
        platform,
        &new_binary,
    )?;
    make_executable(&new_binary)?;
    self_replace::self_replace(&new_binary).context("failed to replace the running executable")?;
    println!("Updated trepo to {latest}.");
    Ok(())
}

fn request<T>(url: &str, limit: u64) -> Result<T>
where
    T: FromResponse,
{
    let tls = ureq::native_tls::TlsConnector::new()
        .context("failed to initialize the system TLS backend")?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .build();
    let response = agent
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("request failed: {url}"))?;
    T::from_response(response, limit)
}

trait FromResponse: Sized {
    fn from_response(response: ureq::Response, limit: u64) -> Result<Self>;
}

impl FromResponse for Vec<u8> {
    fn from_response(response: ureq::Response, limit: u64) -> Result<Self> {
        if response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > limit)
        {
            anyhow::bail!("download exceeds the {limit}-byte limit");
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .context("failed to read response body")?;
        if bytes.len() as u64 > limit {
            anyhow::bail!("download exceeds the {limit}-byte limit");
        }
        Ok(bytes)
    }
}

impl FromResponse for Release {
    fn from_response(response: ureq::Response, limit: u64) -> Result<Self> {
        let bytes = Vec::<u8>::from_response(response, limit)?;
        serde_json::from_slice(&bytes).context("invalid GitHub Release response")
    }
}

fn parse_tag(tag: &str) -> Result<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .with_context(|| format!("release tag is not semantic versioning: {tag}"))
}

fn find_asset<'a>(release: &'a Release, name: &str) -> Result<&'a ReleaseAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("release {} has no asset named {name}", release.tag_name))
}

fn archive_name(tag: &str, platform: &str) -> String {
    let extension = if platform.starts_with("windows-") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("trepo-{tag}-{platform}.{extension}")
}

fn binary_name(platform: &str) -> &'static str {
    if platform.starts_with("windows-") {
        "trepo.exe"
    } else {
        "trepo"
    }
}

fn platform_id() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x86_64"),
        ("macos", "x86_64") => Ok("macos-x86_64"),
        ("macos", "aarch64") => Ok("macos-aarch64"),
        ("windows", "x86_64") => Ok("windows-x86_64"),
        (os, arch) => anyhow::bail!("updates are not published for {os}/{arch}"),
    }
}

fn checksum_for(sums: &str, archive_name: &str) -> Result<String> {
    for line in sums.lines() {
        let mut fields = line.split_whitespace();
        let Some(digest) = fields.next() else {
            continue;
        };
        let Some(name) = fields.next() else {
            continue;
        };
        if name.trim_start_matches('*').trim_start_matches("./") == archive_name {
            if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(digest.to_ascii_lowercase());
            }
            anyhow::bail!("SHA256SUMS contains an invalid digest for {archive_name}");
        }
    }
    anyhow::bail!("SHA256SUMS has no entry for {archive_name}")
}

fn verify_checksum(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        anyhow::bail!("checksum mismatch for release archive (expected {expected}, got {actual})");
    }
    Ok(())
}

fn extract_binary(
    archive: &[u8],
    archive_name: &str,
    tag: &str,
    platform: &str,
    destination: &Path,
) -> Result<()> {
    let path = format!("trepo-{tag}-{platform}/{}", binary_name(platform));
    if archive_name.ends_with(".zip") {
        extract_zip(archive, &path, destination)
    } else {
        extract_tar_gz(archive, &path, destination)
    }
}

fn extract_tar_gz(bytes: &[u8], expected_path: &str, destination: &Path) -> Result<()> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("invalid tar archive")? {
        let mut entry = entry.context("invalid tar entry")?;
        if entry.path().context("invalid tar path")? == Path::new(expected_path) {
            if !entry.header().entry_type().is_file() {
                anyhow::bail!("release binary is not a regular file");
            }
            write_binary(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("release archive has no {expected_path}")
}

fn extract_zip(bytes: &[u8], expected_path: &str, destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("invalid zip archive")?;
    let mut entry = archive
        .by_name(expected_path)
        .with_context(|| format!("release archive has no {expected_path}"))?;
    if entry.is_dir() {
        anyhow::bail!("release binary is not a regular file");
    }
    write_binary(&mut entry, destination)
}

fn write_binary(reader: &mut impl Read, destination: &Path) -> Result<()> {
    let mut file = File::create(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let copied = io::copy(&mut reader.take(MAX_ARCHIVE_BYTES + 1), &mut file)
        .context("failed to extract release binary")?;
    if copied > MAX_ARCHIVE_BYTES {
        drop(file);
        let _ = std::fs::remove_file(destination);
        anyhow::bail!("release binary exceeds the extraction limit");
    }
    file.flush().context("failed to flush release binary")?;
    file.sync_all().context("failed to sync release binary")?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = path.metadata()?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versioned_asset_checksum() {
        let sums = concat!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.tar.gz\n",
            "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD *./trepo-v1.2.3-linux-x86_64.tar.gz\n"
        );
        assert_eq!(
            checksum_for(sums, "trepo-v1.2.3-linux-x86_64.tar.gz").unwrap(),
            "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
        );
        assert!(checksum_for(sums, "missing.tar.gz").is_err());
    }

    #[test]
    fn rejects_checksum_mismatch() {
        assert!(verify_checksum(b"archive", &"00".repeat(32)).is_err());
    }

    #[test]
    fn builds_release_asset_name() {
        let name = archive_name("v1.2.3", "linux-x86_64");
        assert_eq!(name, "trepo-v1.2.3-linux-x86_64.tar.gz");
    }

    #[test]
    fn extracts_expected_tar_layout() {
        let mut bytes = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let payload = b"binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, "trepo-v1.2.3-linux-x86_64/trepo", &payload[..])
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("trepo");
        extract_binary(
            &bytes,
            "trepo-v1.2.3-linux-x86_64.tar.gz",
            "v1.2.3",
            "linux-x86_64",
            &output,
        )
        .unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"binary");
    }

    #[test]
    fn extracts_expected_zip_layout() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut bytes);
            archive
                .start_file(
                    "trepo-v1.2.3-windows-x86_64/trepo.exe",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive.write_all(b"binary").unwrap();
            archive.finish().unwrap();
        }

        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("trepo.exe");
        extract_binary(
            &bytes.into_inner(),
            "trepo-v1.2.3-windows-x86_64.zip",
            "v1.2.3",
            "windows-x86_64",
            &output,
        )
        .unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"binary");
    }

    #[test]
    fn rejects_unexpected_archive_layout() {
        let mut bytes = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let payload = b"binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, "trepo", &payload[..])
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }

        let temp = tempfile::tempdir().unwrap();
        assert!(extract_binary(
            &bytes,
            "trepo-v1.2.3-linux-x86_64.tar.gz",
            "v1.2.3",
            "linux-x86_64",
            &temp.path().join("trepo"),
        )
        .is_err());
    }

    #[test]
    fn parses_prefixed_semver_tag() {
        assert_eq!(parse_tag("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(parse_tag("nightly").is_err());
    }
}
