use std::path::{Path, PathBuf};

use reqwest::Client;
use scraper::{Html, Selector};
use tokio::{fs::File, io::AsyncWriteExt};

use super::{mirrors_by_preference, PAGE_TIMEOUT};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("no download link on the page")]
    LinkNotFound,
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("mirror served a web page instead of the file")]
    NotAFile,
}

pub fn parse_download_href(body: &str) -> Option<String> {
    let document = Html::parse_document(body);
    let anchor_selector = Selector::parse("a[href*=\"get.php\"]").unwrap();

    document
        .select(&anchor_selector)
        .find_map(|anchor| anchor.value().attr("href"))
        .map(str::to_string)
}

pub async fn resolve_url(
    client: &Client,
    mirror: &str,
    md5: &str,
) -> Result<String, DownloadError> {
    let url = format!("https://{}/ads.php?md5={}", mirror, md5);
    let body = client
        .get(url)
        .timeout(PAGE_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let href = parse_download_href(&body).ok_or(DownloadError::LinkNotFound)?;

    if href.starts_with("http://") || href.starts_with("https://") {
        Ok(href)
    } else {
        Ok(format!(
            "https://{}/{}",
            mirror,
            href.trim_start_matches('/')
        ))
    }
}

pub async fn resolve_url_with_failover(
    client: &Client,
    mirrors: &[String],
    preferred: &str,
    md5: &str,
) -> Result<String, DownloadError> {
    let mut last_error = None;

    for mirror in mirrors_by_preference(mirrors, preferred) {
        match resolve_url(client, &mirror, md5).await {
            Ok(url) => return Ok(url),
            Err(e) => {
                log::warn!("Download link lookup on {} failed: {}", mirror, e);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or(DownloadError::LinkNotFound))
}

pub fn sanitize_filename(title: &str, extension: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            ' ' => '_',
            c => c,
        })
        .collect();

    let stem = stem.trim_matches(|c| c == '.' || c == '_');
    let stem = if stem.is_empty() { "book" } else { stem };

    if extension.is_empty() {
        stem.to_string()
    } else {
        format!("{}.{}", stem, extension)
    }
}

pub fn destination_path(directory: &str, title: &str, extension: &str) -> PathBuf {
    Path::new(directory).join(sanitize_filename(title, extension))
}

pub fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

async fn write_chunks(mut response: reqwest::Response, path: &Path) -> Result<u64, DownloadError> {
    let write_error = |source| DownloadError::Write {
        path: path.display().to_string(),
        source,
    };

    let mut file = File::create(path).await.map_err(write_error)?;
    let mut written = 0;

    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await.map_err(write_error)?;
        written += chunk.len() as u64;
    }

    file.flush().await.map_err(write_error)?;
    Ok(written)
}

pub async fn download_to_file(
    client: &Client,
    url: &str,
    destination: &Path,
) -> Result<u64, DownloadError> {
    let response = client.get(url).send().await?.error_for_status()?;

    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));

    if is_html {
        return Err(DownloadError::NotAFile);
    }

    // Written under a temporary name so an interrupted download never looks finished.
    let partial = partial_path(destination);

    let written = match write_chunks(response, &partial).await {
        Ok(written) => written,
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
    };

    tokio::fs::rename(&partial, destination)
        .await
        .map_err(|source| DownloadError::Write {
            path: destination.display().to_string(),
            source,
        })?;

    Ok(written)
}
