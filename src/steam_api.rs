use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

const DETAILS_PATH: &str = "/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const COLLECTION_PATH: &str = "/ISteamRemoteStorage/GetCollectionDetails/v1/";
const API_BASE: &str = "https://api.steampowered.com";

/// Steam rejects very large form posts, and a rejection here costs a full
/// re-download, so details are requested in batches.
const BATCH_SIZE: usize = 100;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// `k_EResultFileNotFound` — Steam positively reports the item as gone, as
/// opposed to merely not being visible to an anonymous caller.
const RESULT_OK: i32 = 1;
const RESULT_FILE_NOT_FOUND: i32 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedFile {
    pub title: String,
    pub file_size: Option<u64>,
    pub time_updated: Option<i64>,
    /// Steam's content handle for the item's files. It changes whenever new
    /// content is published, so a difference between two runs is definitive
    /// proof the files changed — unlike `time_updated`, which Steam's cache
    /// can serve stale for a long time after an update.
    pub hcontent_file: Option<String>,
    /// `true` only when Steam explicitly reported the item as deleted.
    /// Items merely hidden from an unauthenticated request are not deleted.
    pub deleted: bool,
    /// Raw `EResult` Steam returned for this item. Anything other than
    /// `RESULT_OK` means SteamCMD will not be able to download it either.
    pub result: i32,
    /// App the item belongs to. An item published for another app cannot be
    /// downloaded under `--app-id`, and SteamCMD reports that as a plain
    /// download failure.
    pub app_id: Option<u32>,
}

impl PublishedFile {
    /// Human-readable reason an item is unusable, or `None` when Steam
    /// reported it as available.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        match self.result {
            RESULT_OK => None,
            2 => Some("a generic failure"),
            8 => Some("an invalid item, which usually means it is hidden or unlisted"),
            RESULT_FILE_NOT_FOUND => Some("deleted or otherwise gone"),
            15 => Some("access denied"),
            16 => Some("a timeout"),
            17 => Some("banned"),
            _ => Some("unavailable"),
        }
    }
}

#[derive(Debug, Error)]
pub enum SteamApiError {
    #[error("Steam Workshop metadata request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Steam Workshop metadata returned an invalid response")]
    InvalidResponse,
    #[error("`{0}` is not a Workshop collection ID or URL")]
    InvalidCollection(String),
}

#[derive(Debug, Deserialize)]
struct DetailsResponse {
    response: DetailsList,
}

#[derive(Debug, Deserialize)]
struct DetailsList {
    publishedfiledetails: Vec<Details>,
}

#[derive(Debug, Deserialize)]
struct Details {
    publishedfileid: String,
    result: Option<i32>,
    title: Option<String>,
    file_size: Option<String>,
    time_updated: Option<i64>,
    hcontent_file: Option<String>,
    consumer_app_id: Option<u32>,
    creator_app_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CollectionResponse {
    response: CollectionList,
}

#[derive(Debug, Deserialize)]
struct CollectionList {
    collectiondetails: Vec<CollectionDetails>,
}

#[derive(Debug, Deserialize)]
struct CollectionDetails {
    result: Option<i32>,
    children: Option<Vec<CollectionChild>>,
}

#[derive(Debug, Deserialize)]
struct CollectionChild {
    publishedfileid: String,
}

/// Base URL for the Steam Web API, overridable for mirrors and test rigs.
fn api_base() -> String {
    std::env::var("SWMCTL_API_BASE_URL").unwrap_or_else(|_| API_BASE.to_string())
}

fn client() -> Result<reqwest::blocking::Client, SteamApiError> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?)
}

pub fn fetch_published_files(
    mod_ids: &[String],
) -> Result<HashMap<String, PublishedFile>, SteamApiError> {
    if mod_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let client = client()?;
    let mut files = HashMap::new();
    for batch in mod_ids.chunks(BATCH_SIZE) {
        let mut form = vec![("itemcount".to_string(), batch.len().to_string())];
        for (index, mod_id) in batch.iter().enumerate() {
            form.push((format!("publishedfileids[{index}]"), mod_id.clone()));
        }

        let response = client
            .post(format!("{}{}", api_base(), DETAILS_PATH))
            .form(&form)
            .send()?
            .error_for_status()?
            .json::<DetailsResponse>()?;

        files.extend(parse_published_files(response).ok_or(SteamApiError::InvalidResponse)?);
    }
    Ok(files)
}

fn parse_published_files(response: DetailsResponse) -> Option<HashMap<String, PublishedFile>> {
    if response.response.publishedfiledetails.is_empty() {
        return None;
    }

    Some(
        response
            .response
            .publishedfiledetails
            .into_iter()
            .map(|detail| {
                let result = detail.result.unwrap_or(RESULT_OK);
                (
                    detail.publishedfileid,
                    PublishedFile {
                        title: detail.title.unwrap_or_default(),
                        file_size: detail.file_size.and_then(|size| size.parse().ok()),
                        time_updated: detail.time_updated,
                        hcontent_file: detail.hcontent_file.filter(|handle| !handle.is_empty()),
                        deleted: result == RESULT_FILE_NOT_FOUND,
                        result,
                        // Steam reports the consuming app; the creating app is
                        // the same for ordinary Workshop items and is only a
                        // fallback for entries that omit it.
                        app_id: detail.consumer_app_id.or(detail.creator_app_id),
                    },
                )
            })
            .collect(),
    )
}

/// Extract the `id` query parameter from a Workshop URL, or accept a bare
/// numeric ID. Reading the query properly matters: a naive `split("id=")`
/// also matches the `appid=` parameter and silently resolves the wrong
/// collection.
fn collection_id(collection: &str) -> Option<String> {
    let trimmed = collection.trim().trim_matches('/');
    if !trimmed.is_empty() && trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let query = trimmed.split_once('?').map(|(_, query)| query)?;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "id")
        .map(|(_, value)| value.trim_matches('/').to_string())
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
}

pub fn fetch_collection_items(collection: &str) -> Result<Vec<String>, SteamApiError> {
    let collection_id = collection_id(collection)
        .ok_or_else(|| SteamApiError::InvalidCollection(collection.to_string()))?;

    let form = [
        ("collectioncount", "1".to_string()),
        ("publishedfileids[0]", collection_id),
    ];
    let response = client()?
        .post(format!("{}{}", api_base(), COLLECTION_PATH))
        .form(&form)
        .send()?
        .error_for_status()?
        .json::<CollectionResponse>()?;
    let collection = response
        .response
        .collectiondetails
        .into_iter()
        .next()
        .ok_or(SteamApiError::InvalidResponse)?;
    if collection.result.unwrap_or(RESULT_OK) != RESULT_OK {
        return Err(SteamApiError::InvalidResponse);
    }
    Ok(collection
        .children
        .unwrap_or_default()
        .into_iter()
        .map(|child| child.publishedfileid)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_response_details() {
        let response: DetailsResponse = serde_json::from_str(
            r#"{"response":{"publishedfiledetails":[{"publishedfileid":"123","result":1,"title":"Example","file_size":"456","time_updated":1700000000,"hcontent_file":"9876543210"}]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");
        assert_eq!(files["123"].title, "Example");
        assert_eq!(files["123"].file_size, Some(456));
        assert!(!files["123"].deleted);
        assert_eq!(files["123"].result, 1);
        assert_eq!(files["123"].hcontent_file.as_deref(), Some("9876543210"));
        assert!(files["123"].unavailable_reason().is_none());
    }

    #[test]
    fn reads_the_app_the_item_belongs_to() {
        let response: DetailsResponse = serde_json::from_str(
            r#"{"response":{"publishedfiledetails":[
                {"publishedfileid":"123","result":1,"consumer_app_id":107410,"creator_app_id":107410},
                {"publishedfileid":"456","result":1,"creator_app_id":221100},
                {"publishedfileid":"789","result":1}
            ]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");

        assert_eq!(files["123"].app_id, Some(107410));
        // Items that omit the consuming app fall back to the creating app.
        assert_eq!(files["456"].app_id, Some(221100));
        assert_eq!(files["789"].app_id, None);
    }

    #[test]
    fn describes_why_an_item_is_unavailable() {
        let response: DetailsResponse = serde_json::from_str(
            r#"{"response":{"publishedfiledetails":[
                {"publishedfileid":"hidden","result":8},
                {"publishedfileid":"denied","result":15},
                {"publishedfileid":"banned","result":17}
            ]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");

        assert!(files["hidden"].unavailable_reason().is_some());
        assert!(files["denied"].unavailable_reason().is_some());
        assert_eq!(files["banned"].unavailable_reason(), Some("banned"));
        // Only a positive "file not found" may drive deletion.
        assert!(!files["hidden"].deleted && !files["denied"].deleted && !files["banned"].deleted);
    }

    #[test]
    fn marks_only_file_not_found_as_deleted() {
        let response: DetailsResponse = serde_json::from_str(
            r#"{"response":{"publishedfiledetails":[
                {"publishedfileid":"gone","result":9},
                {"publishedfileid":"private","result":8}
            ]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");

        assert!(files["gone"].deleted);
        // A private or login-gated item is not deleted; SteamCMD may still be
        // able to download it with credentials.
        assert!(!files["private"].deleted);
    }

    #[test]
    fn reads_the_id_parameter_not_the_appid() {
        assert_eq!(collection_id("123456789").as_deref(), Some("123456789"));
        assert_eq!(
            collection_id("https://steamcommunity.com/sharedfiles/filedetails/?id=123456789")
                .as_deref(),
            Some("123456789")
        );
        // Regression: `appid=` contains `id=`.
        assert_eq!(
            collection_id(
                "https://steamcommunity.com/sharedfiles/filedetails/?appid=107410&id=123456789"
            )
            .as_deref(),
            Some("123456789")
        );
        assert_eq!(
            collection_id("https://steamcommunity.com/sharedfiles/filedetails/?id=123&searchtext=")
                .as_deref(),
            Some("123")
        );
    }

    #[test]
    fn rejects_collections_without_a_numeric_id() {
        assert_eq!(collection_id("https://example.com/?appid=107410"), None);
        assert_eq!(collection_id("not-a-collection"), None);
        assert_eq!(collection_id(""), None);
    }
}
