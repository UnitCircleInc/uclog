//! Logdata resolution by `app_hash` (protocol.md §6.4, plans README §5.3).
//!
//! Order: manual files → local cache → fw.unitcircle.ca → sibling of a known FW image.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha512};
use uclog_logdata::LogData;

/// Size of the signed-image header that `app_hash` skips.
pub const IMAGE_HEADER: usize = 512;

#[derive(Debug, Clone, Default)]
pub struct ResolverConfig {
    /// Files chosen manually (`-l`). Used for any app_hash unless their name carries a
    /// different hash.
    pub manual: Vec<PathBuf>,
    /// `<cache>/<hash>.logdata`.
    pub cache_dir: Option<PathBuf>,
    /// Remote lookup needs a domain.
    pub domain: Option<String>,
    pub token: Option<String>,
    /// Default `https://fw.unitcircle.ca`.
    pub url: Option<String>,
    /// FW images (e.g. the last flashed `zephyr.signed.bin`) whose directory may hold the ELF.
    pub images: Vec<PathBuf>,
}

/// Where logdata came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Manual(PathBuf),
    Cache(PathBuf),
    Remote(String),
    Sibling(PathBuf),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Manual(p) => write!(f, "manual {}", p.display()),
            Source::Cache(p) => write!(f, "cache {}", p.display()),
            Source::Remote(u) => write!(f, "remote {u}"),
            Source::Sibling(p) => write!(f, "image sibling {}", p.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub logdata: Vec<Arc<LogData>>,
    pub source: Source,
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub resolved: Option<Resolved>,
    /// Problems worth showing (remote errors etc.), even when resolution succeeded later.
    pub notes: Vec<String>,
}

/// SHA-512 of a signed image after its 512-byte header.
pub fn image_hash(data: &[u8]) -> Option<[u8; 64]> {
    (data.len() > IMAGE_HEADER).then(|| Sha512::digest(&data[IMAGE_HEADER..]).into())
}

pub async fn resolve(cfg: &ResolverConfig, app_hash: &[u8]) -> Outcome {
    let hash = hex::encode(app_hash);
    let mut out = Outcome::default();

    // 1. Manual.
    let mut manual = vec![];
    for p in &cfg.manual {
        match LogData::load(p) {
            Ok(ld) if ld.app_hash.as_deref().is_none_or(|h| h == app_hash) => manual.push(Arc::new(ld)),
            Ok(_) => out.notes.push(format!("{}: app_hash does not match the device; ignored", p.display())),
            Err(e) => out.notes.push(format!("{}: {e}", p.display())),
        }
    }
    if !manual.is_empty() {
        out.resolved = Some(Resolved { logdata: manual, source: Source::Manual(cfg.manual[0].clone()) });
        return out;
    }

    // 2. Cache.
    if let Some(dir) = &cfg.cache_dir {
        let p = dir.join(format!("{hash}.logdata"));
        if p.exists() {
            match LogData::load(&p) {
                Ok(ld) => {
                    out.resolved = Some(Resolved { logdata: vec![Arc::new(ld)], source: Source::Cache(p) });
                    return out;
                }
                Err(e) => out.notes.push(format!("cache {}: {e}", p.display())),
            }
        }
    }

    // 3. Remote.
    if let Some(domain) = &cfg.domain {
        let base = cfg.url.as_deref().unwrap_or("https://fw.unitcircle.ca").trim_end_matches('/');
        let url = format!("{base}/logdata/{domain}/{hash}");
        match fetch(&url, cfg.token.as_deref()).await {
            Ok(data) => match LogData::from_cbor(&data, url.clone()) {
                Ok(mut ld) => {
                    ld.app_hash = Some(app_hash.to_vec());
                    if let Some(dir) = &cfg.cache_dir {
                        let p = dir.join(format!("{hash}.logdata"));
                        let _ = std::fs::create_dir_all(dir);
                        if let Err(e) = std::fs::write(&p, &data) {
                            out.notes.push(format!("cache write {}: {e}", p.display()));
                        }
                    }
                    out.resolved = Some(Resolved { logdata: vec![Arc::new(ld)], source: Source::Remote(url) });
                    return out;
                }
                Err(e) => out.notes.push(format!("{url}: {e}")),
            },
            Err(FetchError::NotFound) => {}
            Err(e) => out.notes.push(format!("{url}: {e}")),
        }
    }

    // 4. Sibling of a known image.
    for img in &cfg.images {
        let Ok(data) = std::fs::read(img) else { continue };
        if image_hash(&data).as_ref().map(|h| &h[..]) != Some(app_hash) {
            continue;
        }
        if let Some(p) = sibling_logdata(img) {
            match LogData::load(&p) {
                Ok(ld) => {
                    out.resolved = Some(Resolved { logdata: vec![Arc::new(ld)], source: Source::Sibling(p) });
                    return out;
                }
                Err(e) => out.notes.push(format!("{}: {e}", p.display())),
            }
        }
    }
    out
}

/// `zephyr.signed.bin` → `zephyr.elf` (or any `.logdata`) in the same directory.
fn sibling_logdata(img: &Path) -> Option<PathBuf> {
    let dir = img.parent()?;
    let stem = img.file_name()?.to_str()?.split('.').next()?;
    [dir.join(format!("{stem}.elf")), dir.join(format!("{stem}.logdata")), dir.join("zephyr.elf")]
        .into_iter()
        .find(|p| p.exists())
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("not found")]
    NotFound,
    #[error("HTTP {status}{}{}", .detail.as_ref().map(|d| format!(": {d}")).unwrap_or_default(), .request_id.as_ref().map(|r| format!(" (request id {r})")).unwrap_or_default())]
    Http { status: u16, detail: Option<String>, request_id: Option<String> },
    #[error("{0}")]
    Net(String),
}

/// GET with an optional bearer token. Follows the 302 to the short-lived download link.
pub async fn fetch(url: &str, token: Option<&str>) -> Result<Vec<u8>, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| FetchError::Net(e.to_string()))?;
    let mut req = client.get(url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.map_err(|e| FetchError::Net(e.to_string()))?;
    let status = r.status().as_u16();
    if status == 404 {
        return Err(FetchError::NotFound);
    }
    if !(200..300).contains(&status) {
        let request_id = r.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(str::to_string);
        let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(|s| format!("retry after {s}s"));
        // RFC 9457 problem+json: show title/detail when present.
        let body = r.text().await.unwrap_or_default();
        let detail = problem_detail(&body).or(retry);
        return Err(FetchError::Http { status, detail, request_id });
    }
    r.bytes().await.map(|b| b.to_vec()).map_err(|e| FetchError::Net(e.to_string()))
}

fn problem_detail(body: &str) -> Option<String> {
    // Tiny extraction to avoid a JSON dependency: "detail":"..." or "title":"...".
    for key in ["\"detail\"", "\"title\""] {
        if let Some(i) = body.find(key) {
            let rest = &body[i + key.len()..];
            let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
            return rest.split('"').next().map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_problem() {
        let mut img = vec![0xAAu8; IMAGE_HEADER];
        img.extend_from_slice(b"abc");
        assert_eq!(hex::encode(image_hash(&img).unwrap())[..16], hex::encode(Sha512::digest(b"abc"))[..16]);
        assert_eq!(
            problem_detail(r#"{"type":"x","title":"Forbidden","detail":"token expired"}"#).as_deref(),
            Some("token expired")
        );
    }

    #[tokio::test]
    async fn cache_and_manual() {
        let d = tempfile::tempdir().unwrap();
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../spec/vectors/sample.logdata");
        let hash = [7u8; 64];
        std::fs::copy(&sample, d.path().join(format!("{}.logdata", hex::encode(hash)))).unwrap();
        let cfg = ResolverConfig { cache_dir: Some(d.path().into()), ..Default::default() };
        let o = resolve(&cfg, &hash).await;
        assert!(matches!(o.resolved.unwrap().source, Source::Cache(_)));
        assert!(resolve(&cfg, &[8u8; 64]).await.resolved.is_none());
        let cfg = ResolverConfig { manual: vec![sample], ..cfg };
        assert!(matches!(resolve(&cfg, &[8u8; 64]).await.resolved.unwrap().source, Source::Manual(_)));
    }
}
