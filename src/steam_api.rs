use std::collections::HashMap;

use serde::Deserialize;
use thiserror::Error;

const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const COLLECTION_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedFile {
    pub title: String,
    pub file_size: Option<u64>,
    pub time_updated: Option<i64>,
}

#[derive(Debug, Error)]
pub enum SteamApiError {
    #[error("Steam Workshop metadata request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Steam Workshop metadata returned an invalid response")]
    InvalidResponse,
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

pub fn fetch_published_files(
    mod_ids: &[String],
) -> Result<HashMap<String, PublishedFile>, SteamApiError> {
    if mod_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut form = vec![("itemcount".to_string(), mod_ids.len().to_string())];
    for (index, mod_id) in mod_ids.iter().enumerate() {
        form.push((format!("publishedfileids[{index}]"), mod_id.clone()));
    }

    let response = reqwest::blocking::Client::new()
        .post(DETAILS_URL)
        .form(&form)
        .send()?
        .error_for_status()?
        .json::<DetailsResponse>()?;

    parse_published_files(response).ok_or(SteamApiError::InvalidResponse)
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
            .filter(|detail| detail.result.unwrap_or(1) == 1)
            .map(|detail| {
                (
                    detail.publishedfileid,
                    PublishedFile {
                        title: detail.title.unwrap_or_default(),
                        file_size: detail.file_size.and_then(|size| size.parse().ok()),
                        time_updated: detail.time_updated,
                    },
                )
            })
            .collect(),
    )
}

pub fn fetch_collection_items(collection: &str) -> Result<Vec<String>, SteamApiError> {
    let collection_id = collection
        .split("id=")
        .nth(1)
        .unwrap_or(collection)
        .split('&')
        .next()
        .unwrap_or(collection)
        .trim_matches('/');
    if !collection_id
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        return Err(SteamApiError::InvalidResponse);
    }
    let form = [
        ("collectioncount", "1".to_string()),
        ("publishedfileids[0]", collection_id.to_string()),
    ];
    let response = reqwest::blocking::Client::new()
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
    if collection.result.unwrap_or(1) != 1 {
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
    fn parses_file_size_as_a_number() {
        let detail = Details {
            publishedfileid: "123".into(),
            result: Some(1),
            title: Some("Example".into()),
            file_size: Some("456".into()),
            time_updated: Some(1700000000),
        };
        let file = PublishedFile {
            title: detail.title.unwrap_or_default(),
            file_size: detail.file_size.and_then(|size| size.parse().ok()),
            time_updated: detail.time_updated,
        };
        assert_eq!(file.file_size, Some(456));
    }

    #[test]
    fn parses_api_response_details() {
        let response: DetailsResponse = serde_json::from_str(
            r#"{"response":{"publishedfiledetails":[{"publishedfileid":"123","result":1,"title":"Example","file_size":"456","time_updated":1700000000}]}}"#,
        )
        .expect("response should parse");
        let files = parse_published_files(response).expect("details should be present");
        assert_eq!(files["123"].title, "Example");
    }
}
