use futures::{future::select_all, FutureExt};
use log::info;
use reqwest::Client;
use scraper::{Html, Selector};

use super::PAGE_TIMEOUT;

pub struct ActiveMirror {
    pub host: String,
    pub siblings: Vec<String>,
}

pub fn discover_mirrors(body: &str) -> Vec<String> {
    let document = Html::parse_document(body);
    let anchor_selector = Selector::parse("a[href]").unwrap();
    let mut found = Vec::new();

    for anchor in document.select(&anchor_selector) {
        let Some(href) = anchor.value().attr("href") else {
            continue;
        };
        let Some(rest) = href.strip_prefix("https://") else {
            continue;
        };

        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);

        if !host.starts_with("libgen.") || host.ends_with(".onion") {
            continue;
        }
        if !found.iter().any(|existing| existing == host) {
            found.push(host.to_string());
        }
    }

    found
}

pub fn is_libgen_page(body: &str) -> bool {
    let search_input_selector = Selector::parse("input[name=\"req\"]").unwrap();
    Html::parse_document(body)
        .select(&search_input_selector)
        .next()
        .is_some()
}

pub fn merge(groups: &[&[String]]) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();

    for group in groups {
        for host in group.iter() {
            if !merged.contains(host) {
                merged.push(host.clone());
            }
        }
    }

    merged
}

async fn probe(client: Client, mirror: String) -> Result<ActiveMirror, ()> {
    let url = format!("https://{}/", mirror);
    info!("Testing connection to {}", url);

    let response = match client.get(&url).timeout(PAGE_TIMEOUT).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            info!("{} responded with {}", mirror, response.status());
            return Err(());
        }
        Err(e) => {
            info!("{} failed: {}", mirror, e);
            return Err(());
        }
    };

    let body = match response.text().await {
        Ok(body) => body,
        Err(e) => {
            info!("{} body failed: {}", mirror, e);
            return Err(());
        }
    };

    // Parked domains and placeholder pages still answer 200, so check it is really libgen.
    if !is_libgen_page(&body) {
        info!("{} answered but is not serving libgen", mirror);
        return Err(());
    }

    Ok(ActiveMirror {
        siblings: discover_mirrors(&body),
        host: mirror,
    })
}

pub async fn find_active(client: &Client, mirrors: &[String]) -> Option<ActiveMirror> {
    if mirrors.is_empty() {
        return None;
    }

    let mut remaining: Vec<_> = mirrors
        .iter()
        .map(|mirror| probe(client.clone(), mirror.clone()).boxed())
        .collect();

    while !remaining.is_empty() {
        let (result, _index, rest) = select_all(remaining).await;

        match result {
            Ok(active) => return Some(active),
            Err(()) => remaining = rest,
        }
    }

    None
}
