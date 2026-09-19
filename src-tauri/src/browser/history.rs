use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use super::model::BrowserHistoryEntry;

pub(crate) fn append(root: &Path, entry: &BrowserHistoryEntry) -> Result<(), String> {
    let directory = root.join("profiles").join(&entry.profile_id);
    fs::create_dir_all(&directory).map_err(|e| format!("could not create history folder: {e}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("history.jsonl"))
        .map_err(|e| format!("could not open browser history: {e}"))?;
    let line =
        serde_json::to_string(entry).map_err(|e| format!("could not encode history: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("could not save browser history: {e}"))
}

pub(crate) fn read(
    root: &Path,
    profile_id: &str,
    query: Option<&str>,
    before: Option<&str>,
    limit: usize,
) -> Result<(Vec<BrowserHistoryEntry>, Option<String>), String> {
    let path = root.join("profiles").join(profile_id).join("history.jsonl");
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), None)),
        Err(e) => return Err(format!("could not read browser history: {e}")),
    };
    let before_entry = parse_cursor(before)?;
    let needle = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);

    let mut entries: Vec<_> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<BrowserHistoryEntry>(&line).ok())
        .filter(|entry| {
            before_entry
                .as_ref()
                .is_none_or(|(at, id)| (entry.visited_at, entry.id.as_str()) < (*at, id.as_str()))
        })
        .filter(|entry| {
            needle.as_ref().is_none_or(|needle| {
                entry.url.to_lowercase().contains(needle)
                    || entry.title.to_lowercase().contains(needle)
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.visited_at
            .cmp(&a.visited_at)
            .then_with(|| b.id.cmp(&a.id))
    });

    let has_more = entries.len() > limit;
    entries.truncate(limit);
    let cursor = has_more.then(|| {
        let last = entries
            .last()
            .expect("a continuation requires at least one entry");
        format!("{}:{}", last.visited_at, last.id)
    });
    Ok((entries, cursor))
}

pub(crate) fn parse_cursor(value: Option<&str>) -> Result<Option<(u64, String)>, String> {
    let Some(value) = value else { return Ok(None) };
    let (at, id) = value
        .split_once(':')
        .ok_or_else(|| "browser history cursor is invalid".to_string())?;
    if id.is_empty() {
        return Err("browser history cursor is invalid".into());
    }
    let at = at
        .parse::<u64>()
        .map_err(|_| "browser history cursor is invalid".to_string())?;
    Ok(Some((at, id.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, title: &str, at: u64) -> BrowserHistoryEntry {
        BrowserHistoryEntry {
            id: id.into(),
            profile_id: "profile".into(),
            url: format!("https://example.com/{id}"),
            title: title.into(),
            visited_at: at,
        }
    }

    #[test]
    fn history_is_newest_first_searchable_and_paginated() {
        let root = std::env::temp_dir().join(format!("conduit-history-{}", uuid::Uuid::new_v4()));
        append(&root, &entry("one", "First", 10)).unwrap();
        append(&root, &entry("two", "Matching page", 30)).unwrap();
        append(&root, &entry("three", "Matching later", 20)).unwrap();

        let (first, cursor) = read(&root, "profile", Some("matching"), None, 1).unwrap();
        assert_eq!(first[0].id, "two");
        let (second, next) =
            read(&root, "profile", Some("matching"), cursor.as_deref(), 1).unwrap();
        assert_eq!(second[0].id, "three");
        assert!(next.is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cursor_does_not_skip_entries_with_the_same_timestamp() {
        let root = std::env::temp_dir().join(format!("conduit-history-{}", uuid::Uuid::new_v4()));
        append(&root, &entry("b", "B", 30)).unwrap();
        append(&root, &entry("a", "A", 30)).unwrap();
        let (first, cursor) = read(&root, "profile", None, None, 1).unwrap();
        assert_eq!(first[0].id, "b");
        let (second, _) = read(&root, "profile", None, cursor.as_deref(), 1).unwrap();
        assert_eq!(second[0].id, "a");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_cursor_is_rejected_instead_of_restarting_pagination() {
        assert!(parse_cursor(Some("not-a-cursor")).is_err());
        assert!(parse_cursor(Some("123:")).is_err());
    }
}
