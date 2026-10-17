pub mod download;
pub mod mirror;
pub mod search;

use std::time::Duration;

use reqwest::Client;

pub use search::Book;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
// Applies between reads, so a slow but steady download is never cut off.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
// Total limit for page requests. Downloads have none, as their size is unbounded.
pub const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn build_client() -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("Failed to build http client.")
}

pub fn mirrors_by_preference(mirrors: &[String], preferred: &str) -> Vec<String> {
    let mut ordered = vec![preferred.to_string()];
    ordered.extend(
        mirrors
            .iter()
            .filter(|mirror| mirror.as_str() != preferred)
            .cloned(),
    );
    ordered
}
