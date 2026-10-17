pub mod config;
pub mod downloads;
pub mod focus;

use std::fs;
use std::path::PathBuf;

use ratatui::widgets::TableState;
use reqwest::Client;
use tui_textarea::TextArea;

pub use config::AppConfig;
pub use downloads::{Download, DownloadStatus, Downloads};
pub use focus::Focus;

use crate::libgen::{self, download, Book};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsState {
    Idle,
    Searching,
    QueryTooShort,
    NoResults,
    Failed(String),
    Results,
}

pub struct App {
    pub client: Client,
    pub config: AppConfig,
    pub downloads: Downloads,
    pub mirrors: Vec<String>,
    pub active_mirror: Option<String>,

    pub search_bar: TextArea<'static>,
    pub search_results: Vec<Book>,
    pub table_state: TableState,
    pub results_state: ResultsState,

    pub focus: Focus,
    pub show_popup: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
        let download_dir = PathBuf::from(&config.download_directory);
        if !download_dir.exists() {
            fs::create_dir_all(&download_dir)
                .expect("Failed to create directory to install files.");
        }

        App {
            client: libgen::build_client(),
            config,
            downloads: Downloads::new(),
            mirrors: config::default_mirrors(),
            active_mirror: None,
            search_bar: TextArea::default(),
            search_results: Vec::new(),
            table_state: TableState::default(),
            results_state: ResultsState::Idle,
            focus: Focus::SearchBar,
            show_popup: false,
            should_quit: false,
        }
    }

    pub fn query(&self) -> String {
        self.search_bar.lines().join(" ").trim().to_string()
    }

    pub fn selected_book(&self) -> Option<&Book> {
        self.table_state
            .selected()
            .and_then(|index| self.search_results.get(index))
    }

    pub fn select_next(&mut self) {
        if let Some(index) = self.table_state.selected() {
            if index + 1 < self.search_results.len() {
                self.table_state.select(Some(index + 1));
            }
        }
    }

    pub fn select_previous(&mut self) {
        if let Some(index) = self.table_state.selected() {
            if index > 0 {
                self.table_state.select(Some(index - 1));
            }
        }
    }

    pub fn set_results(&mut self, results: Vec<Book>) {
        if results.is_empty() {
            self.table_state.select(None);
            self.results_state = ResultsState::NoResults;
            self.focus = Focus::SearchBar;
        } else {
            self.table_state.select(Some(0));
            self.results_state = ResultsState::Results;
            self.focus = Focus::Table;
        }

        self.search_results = results;
    }

    pub fn start_selected_download(&mut self) {
        let (Some(mirror), Some(book)) =
            (self.active_mirror.clone(), self.selected_book().cloned())
        else {
            return;
        };

        let destination = download::destination_path(
            &self.config.download_directory,
            &book.title,
            &book.extension,
        );

        self.downloads.start(&book.title, &book.md5);

        let client = self.client.clone();
        let mirrors = self.mirrors.clone();
        let downloads = self.downloads.clone();

        tokio::spawn(async move {
            let result = async {
                let url =
                    download::resolve_url_with_failover(&client, &mirrors, &mirror, &book.md5)
                        .await?;
                download::download_to_file(&client, &url, &destination).await
            }
            .await;

            match result {
                Ok(_) => downloads.complete(&book.md5),
                Err(e) => {
                    log::error!("Download of {} failed: {}", book.title, e);
                    downloads.fail(&book.md5, e);
                }
            }
        });
    }
}
