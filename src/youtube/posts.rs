//! Channel community posts ("Posts" tab) via the unofficial `InnerTube` browse
//! endpoint. The Data API has no posts resource, so this reads the same JSON
//! the `YouTube` web client renders and may break when `YouTube` changes it.

use serde_json::Value;

/// One public post from the first page of a channel's Posts tab.
#[derive(Debug, PartialEq)]
pub struct Post {
    pub id: String,
    pub content: String,
    pub images: Vec<String>,
    /// How long ago `YouTube` says it was published ("16 hours ago"); None when
    /// the label is not in a recognised form.
    pub age_seconds: Option<i64>,
}

/// Widest image rendition kept; the feed grid never shows a post image larger.
const MAX_IMAGE_WIDTH: u64 = 640;
pub const BROWSE_URL: &str = "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false";
/// Selects the Posts tab of a channel browse.
const POSTS_TAB_PARAMS: &str = "EgVwb3N0c_IGBAoCSgA=";
/// Current stable Chrome on Windows in UA-reduction form (versionhistory API,
/// 2026-10-19). A browser UA keeps the response the one the web client gets.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";
/// ponytail: pinned web client version; refresh it if `YouTube` starts rejecting it.
const CLIENT_VERSION: &str = "2.20261015.00.00";

/// Fetch and parse the first page of a channel's Posts tab, without signing in.
pub async fn fetch(
    http: &reqwest::Client,
    url: &str,
    channel_id: &str,
) -> Result<Vec<Post>, String> {
    let body = serde_json::json!({
        "context": {"client": {
            "clientName": "WEB",
            "clientVersion": CLIENT_VERSION,
            "hl": "en",
            "gl": "JP",
        }},
        "browseId": channel_id,
        "params": POSTS_TAB_PARAMS,
    });
    let response: Value = http
        .post(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .json(&body)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    parse(&response).map_err(str::to_string)
}

/// Read the first page of posts from a browse response for the Posts tab.
/// Members-only posts are skipped. Err means the page is not shaped as
/// expected, which is how an upstream format change shows up.
pub fn parse(response: &Value) -> Result<Vec<Post>, &'static str> {
    let tab = response
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(Value::as_array)
        .and_then(|tabs| {
            tabs.iter()
                .map(|tab| &tab["tabRenderer"])
                .find(|tab| tab["selected"] == true)
        })
        .ok_or("no selected tab")?;
    // Channels without Posts fall back to another selected tab.
    if !tab
        .pointer("/endpoint/commandMetadata/webCommandMetadata/url")
        .and_then(Value::as_str)
        .is_some_and(|url| url.ends_with("/posts"))
    {
        return Ok(vec![]);
    }
    let sections = tab
        .pointer("/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .ok_or("selected tab has no section list")?;

    Ok(sections
        .iter()
        .filter_map(|section| section.pointer("/itemSectionRenderer/contents"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|item| item.pointer("/backstagePostThreadRenderer/post/backstagePostRenderer"))
        .filter(|post| post.get("sponsorsOnlyBadge").is_none())
        .filter_map(|post| {
            Some(Post {
                id: post["postId"].as_str()?.to_string(),
                content: runs_text(&post["contentText"]),
                images: images(&post["backstageAttachment"]),
                age_seconds: age_seconds(&runs_text(&post["publishedTimeText"])),
            })
        })
        .collect())
}

fn runs_text(text: &Value) -> String {
    text["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| run["text"].as_str())
        .collect()
}

fn images(attachment: &Value) -> Vec<String> {
    let renderers: Vec<&Value> = match attachment["postMultiImageRenderer"]["images"].as_array() {
        Some(images) => images
            .iter()
            .map(|image| &image["backstageImageRenderer"])
            .collect(),
        None => vec![&attachment["backstageImageRenderer"]],
    };
    renderers
        .into_iter()
        .filter_map(|renderer| {
            let thumbnails = renderer["image"]["thumbnails"].as_array()?;
            let width = |thumbnail: &&Value| thumbnail["width"].as_u64().unwrap_or(0);
            thumbnails
                .iter()
                .filter(|thumbnail| width(thumbnail) <= MAX_IMAGE_WIDTH)
                .max_by_key(width)
                .or(thumbnails.first())?["url"]
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

/// "16 hours ago" / "2 years ago (edited)" → seconds. Months and years are
/// approximations; the result only seeds an estimated publish time.
fn age_seconds(label: &str) -> Option<i64> {
    let mut words = label.split_whitespace();
    let count: i64 = words.next()?.parse().ok()?;
    let unit = match words.next()?.trim_end_matches('s') {
        "second" => 1,
        "minute" => 60,
        "hour" => 3600,
        "day" => 86400,
        "week" => 7 * 86400,
        "month" => 30 * 86400,
        "year" => 365 * 86400,
        _ => return None,
    };
    (words.next()? == "ago").then_some(count * unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cut from real browse responses (2026-10-19): a single image post, a
    // multi-image post, a text-only post and an edited post, followed by the
    // continuation item that ends the first page.
    const POSTS_TAB: &str = include_str!("testdata/posts_tab.json");
    // A channel without public posts: the Posts tab holds only a continuation.
    const EMPTY_TAB: &str = include_str!("testdata/posts_tab_empty.json");

    fn posts_tab() -> Value {
        serde_json::from_str(POSTS_TAB).unwrap()
    }

    fn post_renderers(response: &mut Value) -> Vec<&mut Value> {
        response["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][1]["tabRenderer"]["content"]
            ["sectionListRenderer"]["contents"][0]["itemSectionRenderer"]["contents"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .filter_map(|item| {
                item.pointer_mut("/backstagePostThreadRenderer/post/backstagePostRenderer")
            })
            .collect()
    }

    #[test]
    fn parses_every_public_post_in_page_order() {
        let ids: Vec<_> = parse(&posts_tab())
            .unwrap()
            .into_iter()
            .map(|post| post.id)
            .collect();
        assert_eq!(
            ids,
            [
                "UgkxYBmEIF2JV4LO2o-9gOZe8tEreRViRKtV",
                "Ugkxf_WC8RUHmnPON-0_IjQkCEXg1RFqeizI",
                "UgkxIzeIbGtv2m-u5xIxuhZl1SURg-Eomt-T",
                "UgkxbPGDHVn_XI7JgCVnykOoSFU2JrfeX18y",
            ]
        );
    }

    #[test]
    fn joins_content_runs_and_keeps_line_breaks() {
        let posts = parse(&posts_tab()).unwrap();
        assert_eq!(
            posts[2].content,
            "if you could only watch one creator forever, who would it be??"
        );
        assert!(posts[3].content.starts_with(
            "💐 兎田ぺこら生誕祭LIVE2023 💐\n\r\nメンバー限定アンコールライブはコチラ❣\n"
        ));
        assert!(
            posts[3]
                .content
                .ends_with("https://youtube.com/live/uqlHiTwjuNI"),
            "link runs are part of the text"
        );
    }

    #[test]
    fn picks_one_url_per_image_capped_at_640px() {
        let posts = parse(&posts_tab()).unwrap();
        assert_eq!(posts[0].images.len(), 1);
        assert!(posts[0].images[0].starts_with("https://yt3.ggpht.com/"));
        assert!(
            posts[0].images[0].contains("=s427-"),
            "widest available when none exceeds 640px"
        );
        assert!(
            posts[1].images.len() > 1,
            "every image of a multi-image post"
        );
        assert!(posts[1].images.iter().all(|url| url.contains("=s640-")));
        assert!(posts[2].images.is_empty(), "text-only post");
    }

    #[test]
    fn reads_the_relative_age_including_edited_labels() {
        let ages: Vec<_> = parse(&posts_tab())
            .unwrap()
            .into_iter()
            .map(|post| post.age_seconds)
            .collect();
        assert_eq!(
            ages,
            [
                Some(365 * 86400),
                Some(3 * 86400),
                Some(86400),
                Some(2 * 365 * 86400)
            ]
        );
    }

    #[test]
    fn skips_members_only_posts() {
        // Anonymous responses never carried a members-only post in 21 sampled
        // channels, so the badge (named as in youtubei.js) is added to a real post.
        let mut response = posts_tab();
        post_renderers(&mut response)[1]["sponsorsOnlyBadge"] =
            serde_json::json!({"sponsorsOnlyBadgeRenderer": {}});
        let ids: Vec<_> = parse(&response)
            .unwrap()
            .into_iter()
            .map(|post| post.id)
            .collect();
        assert!(!ids.contains(&"Ugkxf_WC8RUHmnPON-0_IjQkCEXg1RFqeizI".to_string()));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn a_posts_tab_without_posts_is_an_empty_page() {
        let response: Value = serde_json::from_str(EMPTY_TAB).unwrap();
        assert_eq!(parse(&response), Ok(vec![]));
    }

    #[test]
    fn a_channel_falling_back_to_videos_has_no_posts() {
        // UC9RVroxQd_uSipkkak5o-tw, public browse response, 2026-10-19.
        let response: Value =
            serde_json::from_str(include_str!("testdata/posts_fallback_videos.json")).unwrap();
        assert_eq!(parse(&response), Ok(vec![]));
    }

    #[test]
    fn a_channel_falling_back_to_shorts_has_no_posts() {
        // UC-oOcKk6bbILgAJCsXYnBpA, public browse response, 2026-10-19.
        let response: Value =
            serde_json::from_str(include_str!("testdata/posts_fallback_shorts.json")).unwrap();
        assert_eq!(parse(&response), Ok(vec![]));
    }

    #[test]
    fn a_channel_falling_back_to_home_has_no_posts() {
        // UC-AgWw025aOGg6Fx9yq7AYw, public browse response, 2026-10-19.
        let response: Value =
            serde_json::from_str(include_str!("testdata/posts_fallback_featured.json")).unwrap();
        assert_eq!(parse(&response), Ok(vec![]));
    }

    #[test]
    fn a_selected_posts_tab_still_requires_a_section_list() {
        let mut response = posts_tab();
        response["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][1]["tabRenderer"]
            ["content"] = serde_json::json!({"richGridRenderer": {"contents": []}});
        assert_eq!(parse(&response), Err("selected tab has no section list"));
    }

    #[test]
    fn parses_a_post_from_the_selected_youtube_posts_tab() {
        // UCBR8-60-B28hp2BmDPdntcQ, public browse response, 2026-10-19.
        let response: Value =
            serde_json::from_str(include_str!("testdata/posts_tab_youtube.json")).unwrap();
        let posts = parse(&response).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, "Ugkx3dqOdBTxo7aos-jSj7q9BhAHWQOM_bQn");
        assert_eq!(
            posts[0].content,
            "watching educational videos at 2x speed to double the learning"
        );
    }

    #[test]
    fn a_response_without_a_selected_tab_is_an_error() {
        let mut response = posts_tab();
        response["contents"]["twoColumnBrowseResultsRenderer"]["tabs"][1]["tabRenderer"]
            ["selected"] = Value::Bool(false);
        assert!(parse(&response).is_err());
        assert!(parse(&serde_json::json!({})).is_err());
    }

    #[test]
    fn converts_each_relative_unit_to_seconds() {
        assert_eq!(age_seconds("1 second ago"), Some(1));
        assert_eq!(age_seconds("5 minutes ago"), Some(300));
        assert_eq!(age_seconds("16 hours ago"), Some(16 * 3600));
        assert_eq!(age_seconds("1 day ago"), Some(86400));
        assert_eq!(age_seconds("2 weeks ago"), Some(14 * 86400));
        assert_eq!(age_seconds("1 month ago"), Some(30 * 86400));
        assert_eq!(age_seconds("10 months ago (edited)"), Some(300 * 86400));
        assert_eq!(age_seconds("1 year ago"), Some(365 * 86400));
    }

    #[test]
    fn unrecognised_age_labels_are_none() {
        assert_eq!(age_seconds(""), None);
        assert_eq!(age_seconds("yesterday"), None);
        assert_eq!(age_seconds("3 fortnights ago"), None);
        assert_eq!(age_seconds("16時間前"), None);
    }
}
