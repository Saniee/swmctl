use std::collections::HashMap;
use std::fs;
use std::path::Path;

use thiserror::Error;

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

/// Sections of `appworkshop_<appid>.acf` that hold per-item records.
/// SteamCMD writes installed sizes under `WorkshopItemsInstalled` and
/// timestamps under `WorkshopItemDetails`; both are keyed by Workshop ID.
const ITEM_SECTIONS: [&str; 3] = [
    "WorkshopItemsInstalled",
    "WorkshopItemDetails",
    // Older SteamCMD builds used this name.
    "WorkshopItems",
];

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
    let mut current_id: Option<String> = None;
    // Depth of the brace we entered the item section at, so the section is
    // closed again rather than swallowing everything that follows it.
    let mut section_depth: Option<usize> = None;
    let mut depth = 0usize;

    for line in contents.lines() {
        let tokens = quoted_tokens(line);
        match tokens.as_slice() {
            [key] if ITEM_SECTIONS.contains(&key.as_str()) => section_depth = Some(depth),
            [key]
                if section_depth.is_some()
                    && key.chars().all(|character| character.is_ascii_digit()) =>
            {
                current_id = Some(key.clone());
                items.entry(key.clone()).or_default();
            }
            [key, value] => {
                if let Some(id) = &current_id
                    && section_depth.is_some()
                {
                    items
                        .entry(id.clone())
                        .or_default()
                        .insert(key.clone(), value.clone());
                }
            }
            _ => {}
        }

        for character in line.chars() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if let Some(start) = section_depth {
                        if depth <= start {
                            section_depth = None;
                            current_id = None;
                        } else if depth == start + 1 {
                            current_id = None;
                        }
                    }
                }
                _ => {}
            }
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

    /// Verbatim shape of an `appworkshop_107410.acf` written by SteamCMD,
    /// including the surrounding `AppWorkshop` block and the trailing
    /// `WorkshopItemDetails` section.
    const REAL_ACF: &str = r#"
"AppWorkshop"
{
	"appid"		"107410"
	"SizeOnDisk"		"3742403"
	"NeedsUpdate"		"0"
	"NeedsDownload"		"0"
	"TimeLastUpdated"		"1548969100"
	"WorkshopItemsInstalled"
	{
		"463939057"
		{
			"size"		"3742403"
			"timeupdated"		"1548969057"
			"manifest"		"7847265094"
		}
		"450814997"
		{
			"size"		"120"
			"timeupdated"		"1548969058"
			"manifest"		"7847265095"
		}
	}
	"WorkshopItemDetails"
	{
		"463939057"
		{
			"manifest"		"7847265094"
			"timeupdated"		"1548969057"
			"timetouched"		"1548969099"
		}
	}
}
"#;

    #[test]
    fn parses_a_real_steamcmd_manifest() {
        assert_eq!(
            parse_acf(REAL_ACF),
            vec![
                WorkshopMetadata {
                    mod_id: "450814997".into(),
                    file_size: 120,
                    last_updated: Some(1548969058),
                },
                WorkshopMetadata {
                    mod_id: "463939057".into(),
                    file_size: 3742403,
                    last_updated: Some(1548969057),
                },
            ]
        );
    }

    #[test]
    fn ignores_keys_outside_the_item_sections() {
        // `appid` and `SizeOnDisk` sit beside the item sections and must not be
        // mistaken for item records.
        let parsed = parse_acf(REAL_ACF);
        assert!(parsed.iter().all(|item| item.mod_id != "107410"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn parses_the_legacy_section_name() {
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
