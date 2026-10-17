use libgen_tui::app::config::default_mirrors;
use libgen_tui::libgen::{build_client, download, mirror, search};

#[tokio::test]
async fn pipeline_works_end_to_end() {
    let client = build_client();
    let builtin = default_mirrors();

    let active = mirror::find_active(&client, &builtin)
        .await
        .expect("no mirror is reachable");
    println!("mirror: {}", active.host);
    println!("discovered siblings: {:?}", active.siblings);

    assert!(
        !active.siblings.is_empty(),
        "{} advertised no sibling domains, so mirror discovery has broken",
        active.host
    );

    let mirrors = mirror::merge(&[&active.siblings, &builtin]);
    println!("session mirror list: {:?}", mirrors);

    let (books, served_by) =
        search::search(&client, &mirrors, &active.host, "rust programming", 25)
            .await
            .expect("search failed");
    assert!(!books.is_empty(), "search returned no books");
    println!("{} books from {}", books.len(), served_by);

    let book = &books[0];
    assert!(!book.title.is_empty(), "title was not parsed");
    assert_eq!(book.md5.len(), 32, "md5 was not parsed: {:?}", book.md5);
    assert!(!book.extension.is_empty(), "extension was not parsed");

    let populated = |field: fn(&libgen_tui::libgen::Book) -> &String| {
        books.iter().filter(|b| !field(b).is_empty()).count()
    };
    for (name, count) in [
        ("author", populated(|b| &b.author)),
        ("year", populated(|b| &b.year)),
        ("languages", populated(|b| &b.languages)),
        ("size", populated(|b| &b.size)),
        ("extension", populated(|b| &b.extension)),
    ] {
        assert!(
            count > books.len() / 2,
            "column {} resolved for only {}/{} rows, header mapping is off",
            name,
            count,
            books.len()
        );
    }

    // Large enough that it cannot finish inside a single request timeout, which is
    // how downloads used to be cut off part way.
    let mut large: Vec<_> = books
        .iter()
        .filter_map(|book| size_in_mb(&book.size).map(|mb| (mb, book)))
        .filter(|(mb, _)| *mb >= MIN_DOWNLOAD_MB)
        .collect();
    large.sort_by_key(|(mb, _)| *mb);
    assert!(
        !large.is_empty(),
        "no result is {} MB or larger, so the download test needs a different query",
        MIN_DOWNLOAD_MB
    );

    let destination = std::env::temp_dir().join("libgen-tui-pipeline-test.bin");
    let mut failures = Vec::new();

    for (mb, book) in large.into_iter().take(3) {
        let url =
            match download::resolve_url_with_failover(&client, &mirrors, &served_by, &book.md5)
                .await
            {
                Ok(url) => url,
                Err(e) => {
                    failures.push(format!("{}: {}", book.title, e));
                    continue;
                }
            };
        assert!(url.contains("key="), "download url carries no key: {}", url);

        match download::download_to_file(&client, &url, &destination).await {
            Ok(written) => {
                let on_disk = std::fs::metadata(&destination)
                    .expect("file was not written")
                    .len();
                println!(
                    "downloaded {:?}: {} bytes (listed as {} MB) from {}",
                    book.title, written, mb, url
                );
                assert_eq!(on_disk, written, "file on disk does not match bytes received");
                assert!(
                    written >= (mb - 1) * 1024 * 1024,
                    "download stopped early: {} bytes for a {} MB book",
                    written,
                    mb
                );
                assert!(
                    !download::partial_path(&destination).exists(),
                    "partial file was left behind"
                );
                let _ = std::fs::remove_file(&destination);
                return;
            }
            Err(e) => failures.push(format!("{}: {}", book.title, e)),
        }
    }

    panic!(
        "every download failed. A 5xx here is a libgen CDN outage rather than a \
         problem with this crate; anything else is worth investigating:\n  {}",
        failures.join("\n  ")
    );
}

const MIN_DOWNLOAD_MB: u64 = 25;

fn size_in_mb(size: &str) -> Option<u64> {
    let (number, unit) = size.split_once(' ')?;
    let number: u64 = number.parse().ok()?;
    match unit.to_ascii_uppercase().as_str() {
        "MB" => Some(number),
        "GB" => Some(number * 1024),
        _ => None,
    }
}

#[test]
fn placeholder_pages_are_not_mirrors() {
    let nginx = "<html><head><title>Welcome to nginx!</title></head>\
                 <body><h1>Welcome to nginx!</h1></body></html>";
    let libgen = "<html><body><form id=\"formlibgen\" action=\"index.php\">\
                  <input name=\"req\" type=\"text\"></form></body></html>";

    assert!(!mirror::is_libgen_page(nginx));
    assert!(mirror::is_libgen_page(libgen));
}
