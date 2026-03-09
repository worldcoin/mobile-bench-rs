use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

pub mod compare;
pub mod manifest;
pub mod summary;

pub struct HistoryBundle {
    files: BTreeMap<String, Vec<u8>>,
}

impl HistoryBundle {
    pub fn from_zip(bytes: &[u8]) -> Result<Self> {
        let reader = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader).context("opening history bundle zip")?;
        let mut files = BTreeMap::new();

        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .with_context(|| format!("reading zip entry {index}"))?;
            if entry.is_dir() {
                continue;
            }

            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .with_context(|| format!("reading {}", entry.name()))?;
            files.insert(entry.name().to_string(), contents);
        }

        Ok(Self { files })
    }

    pub fn read_json<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let bytes = self
            .files
            .get(path)
            .with_context(|| format!("missing bundle entry {path}"))?;

        serde_json::from_slice(bytes).with_context(|| format!("parsing {path}"))
    }

    pub fn read_text(&self, path: &str) -> Result<String> {
        let bytes = self
            .files
            .get(path)
            .with_context(|| format!("missing bundle entry {path}"))?;

        String::from_utf8(bytes.clone()).with_context(|| format!("decoding {path} as UTF-8"))
    }
}
