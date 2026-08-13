use std::collections::HashMap;
use std::fs;
use std::path::Path;

use thiserror::Error;

use crate::manifest::RemoteMod;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkshopMetadata {
    pub mod_id: String,
    pub file_size: u64,
    pub last_updated: Option<i64>,
}

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("could not read SteamCMD metadata {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
}

pub fn read_acf(path: &Path) -> Result<Vec<WorkshopMetadata>, MetadataError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path).map_err(|source| MetadataError::Read {
        path: path.display().to_string(),
        source,
    })?;
    Ok(parse_acf(&contents))
}

pub fn parse_acf(contents: &str) -> Vec<WorkshopMetadata> {
    let mut items = HashMap::<String, HashMap<String, String>>::new();
    let mut current_id = None;
    let mut in_items = false;

    for line in contents.lines() {
        let tokens = quoted_tokens(line);
        match tokens.as_slice() {
            [key] if key == "WorkshopItems" => in_items = true,
            [key] if in_items && key.chars().all(|character| character.is_ascii_digit()) => {
                current_id = Some(key.clone());
                items.entry(key.clone()).or_default();
            }
            [key, value] if in_items && key != "WorkshopItems" => {
                if current_id.is_some() {
                    items
                        .entry(current_id.clone().unwrap())
                        .or_default()
                        .insert(key.clone(), value.clone());
                }
            }
            _ if line.contains('}') && current_id.is_some() => current_id = None,
            _ => {}
        }
    }

    let mut result = items
        .into_iter()
        .map(|(mod_id, values)| WorkshopMetadata {
            mod_id,
            file_size: values
                .get("size")
                .and_then(|value| value.parse().ok())
                .unwrap_or_default(),
            last_updated: values
                .get("timeupdated")
                .and_then(|value| value.parse().ok()),
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    result
}

pub fn requested_remote(requested_ids: &[String], metadata: &[WorkshopMetadata]) -> Vec<RemoteMod> {
    requested_ids
        .iter()
        .map(|mod_id| {
            let item = metadata.iter().find(|item| item.mod_id == *mod_id);
            RemoteMod {
                name: mod_id.clone(),
                mod_id: mod_id.clone(),
                file_size: item.map_or(0, |item| item.file_size),
                last_updated: item.and_then(|item| item.last_updated),
                available: item.is_some(),
            }
        })
        .collect()
}

fn quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    for character in line.chars() {
        match (quoted, character) {
            (false, '"') => quoted = true,
            (true, '"') => {
                tokens.push(std::mem::take(&mut token));
                quoted = false;
            }
            (true, character) => token.push(character),
            _ => {}
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workshop_item_metadata() {
        let contents = r#"
            "WorkshopItems"
            {
                "123"
                {
                    "timeupdated" "1700000000"
                    "size" "4567"
                }
            }
        "#;

        assert_eq!(
            parse_acf(contents),
            vec![WorkshopMetadata {
                mod_id: "123".into(),
                file_size: 4567,
                last_updated: Some(1700000000),
            }]
        );
    }
}
