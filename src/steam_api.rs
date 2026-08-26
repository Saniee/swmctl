use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const COLLECTION_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";

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
    /// `true` only when Steam explicitly reported the item as deleted.
    /// Items merely hidden from an unauthenticated request are not deleted.
    pub deleted: bool,
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
            .post(DETAILS_URL)
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
                        deleted: result == RESULT_FILE_NOT_FOUND,
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
        .post(COLLECTION_URL)
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
            r#"{"response":{"publishedfiledetails":[{"publishedfileid":"123","result":1,"title":"Example","file_size":"456","time_updated":1700000000}]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");
        assert_eq!(files["123"].title, "Example");
        assert_eq!(files["123"].file_size, Some(456));
        assert!(!files["123"].deleted);
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
