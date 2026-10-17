use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use urlencoding::encode;

use super::{mirrors_by_preference, PAGE_TIMEOUT};

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub publisher: String,
    pub year: String,
    pub languages: String,
    pub pages: String,
    pub size: String,
    pub extension: String,
    pub md5: String,
}

fn text_without_italics(element: ElementRef) -> String {
    let italic_selector = Selector::parse("i").unwrap();
    let mut text = element.text().collect::<String>();

    for italic in element.select(&italic_selector) {
        let italic_text = italic.text().collect::<String>();
        if !italic_text.trim().is_empty() {
            text = text.replace(&italic_text, "");
        }
    }

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cell_text(cell: Option<&ElementRef>) -> String {
    cell.map(|cell| text_without_italics(*cell))
        .unwrap_or_default()
}

fn first_number(text: &str) -> Option<String> {
    let digits: String = text
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();

    (!digits.is_empty()).then_some(digits)
}

fn parse_pages(raw: &str) -> String {
    let without_detail = raw.split(';').next().unwrap_or(raw);
    let segments: Vec<&str> = without_detail.split('/').collect();

    for segment in segments.iter().rev() {
        match first_number(segment) {
            Some(number) if number != "0" => return number,
            _ => continue,
        }
    }

    String::new()
}

fn value_after<'a>(href: &'a str, key: &str) -> Option<&'a str> {
    let start = href.find(key)? + key.len();
    let rest = &href[start..];
    Some(rest.split('&').next().unwrap_or(rest))
}

struct Columns {
    title: usize,
    author: usize,
    publisher: usize,
    year: usize,
    languages: usize,
    pages: usize,
    size: usize,
    extension: usize,
}

impl Default for Columns {
    fn default() -> Self {
        Columns {
            title: 0,
            author: 1,
            publisher: 2,
            year: 3,
            languages: 4,
            pages: 5,
            size: 6,
            extension: 7,
        }
    }
}

fn column_of(headers: &[String], keyword: &str, fallback: usize) -> usize {
    headers
        .iter()
        .position(|header| header.contains(keyword))
        .unwrap_or(fallback)
}

fn columns_from_headers(document: &Html) -> Columns {
    let header_selector = Selector::parse("table#tablelibgen thead th").unwrap();

    let headers: Vec<String> = document
        .select(&header_selector)
        .map(|header| header.text().collect::<String>().to_lowercase())
        .collect();

    if headers.is_empty() {
        return Columns::default();
    }

    let fallback = Columns::default();

    Columns {
        title: column_of(&headers, "title", fallback.title),
        author: column_of(&headers, "author", fallback.author),
        publisher: column_of(&headers, "publisher", fallback.publisher),
        year: column_of(&headers, "year", fallback.year),
        languages: column_of(&headers, "language", fallback.languages),
        pages: column_of(&headers, "pages", fallback.pages),
        size: column_of(&headers, "size", fallback.size),
        extension: column_of(&headers, "ext", fallback.extension),
    }
}

pub fn parse_books(body: &str) -> Vec<Book> {
    let document = Html::parse_document(body);
    let row_selector = Selector::parse("table#tablelibgen tbody tr").unwrap();
    let cell_selector = Selector::parse("td").unwrap();
    let anchor_selector = Selector::parse("a").unwrap();
    let md5_anchor_selector = Selector::parse("a[href*=\"md5=\"]").unwrap();
    let file_anchor_selector = Selector::parse("a[href*=\"file.php?id=\"]").unwrap();

    let columns = columns_from_headers(&document);
    let mut books = Vec::new();

    for row in document.select(&row_selector) {
        let cells: Vec<ElementRef> = row.select(&cell_selector).collect();

        let Some(md5) = row
            .select(&md5_anchor_selector)
            .filter_map(|a| a.value().attr("href"))
            .find_map(|href| value_after(href, "md5="))
        else {
            continue;
        };

        let id = row
            .select(&file_anchor_selector)
            .filter_map(|a| a.value().attr("href"))
            .find_map(|href| value_after(href, "file.php?id="))
            .unwrap_or_default()
            .to_string();

        let title = cells
            .get(columns.title)
            .map(|cell| {
                cell.select(&anchor_selector)
                    .next()
                    .map(text_without_italics)
                    .unwrap_or_else(|| text_without_italics(*cell))
            })
            .unwrap_or_default();

        books.push(Book {
            id,
            title,
            author: cell_text(cells.get(columns.author)),
            publisher: cell_text(cells.get(columns.publisher)),
            year: cell_text(cells.get(columns.year)),
            languages: cell_text(cells.get(columns.languages)),
            pages: parse_pages(&cell_text(cells.get(columns.pages))),
            size: cell_text(cells.get(columns.size)),
            extension: cell_text(cells.get(columns.extension)),
            md5: md5.to_string(),
        });
    }

    books
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("not a libgen search page")]
    NoResultsTable,
}

pub fn has_results_table(body: &str) -> bool {
    let table_selector = Selector::parse("table#tablelibgen").unwrap();
    Html::parse_document(body).select(&table_selector).count() > 0
}

pub async fn search_mirror(
    client: &Client,
    mirror: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<Book>, SearchError> {
    let url = format!(
        "https://{}/index.php?req={}&res={}",
        mirror,
        encode(query),
        max_results
    );

    let body = client
        .get(url)
        .timeout(PAGE_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    if !has_results_table(&body) {
        return Err(SearchError::NoResultsTable);
    }

    Ok(parse_books(&body))
}

pub async fn search(
    client: &Client,
    mirrors: &[String],
    preferred: &str,
    query: &str,
    max_results: usize,
) -> Result<(Vec<Book>, String), SearchError> {
    let mut last_error = None;

    for mirror in mirrors_by_preference(mirrors, preferred) {
        match search_mirror(client, &mirror, query, max_results).await {
            Ok(books) => return Ok((books, mirror)),
            Err(e) => {
                log::warn!("Search on {} failed: {}", mirror, e);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.expect("mirror list is never empty"))
}
