use std::sync::OnceLock;

use base64::Engine;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::HttpError;
use crate::internal::prelude::*;

static BUILD_NUMBER_CACHE: OnceLock<u32> = OnceLock::new();
static BROWSER_VERSION_CACHE: OnceLock<u32> = OnceLock::new();

/// Fallback values if fetching fails
const FALLBACK_BUILD_NUMBER: u32 = 9999;
const FALLBACK_BROWSER_VERSION: u32 = 136;

/// Represents the Discord X-Super-Properties header
///
/// This header is required for all authenticated requests to appear as a legitimate client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperProperties {
    /// Operating system (e.g., "Windows", "macOS", "Linux")
    #[serde(rename = "os")]
    pub os: String,

    /// Browser name (e.g., "Chrome", "Firefox")
    #[serde(rename = "browser")]
    pub browser: String,

    /// Device type (empty string for desktop)
    #[serde(rename = "device")]
    pub device: String,

    /// System locale (e.g., "en-US")
    #[serde(rename = "system_locale")]
    pub system_locale: String,

    /// Full browser user agent string
    #[serde(rename = "browser_user_agent")]
    pub browser_user_agent: String,

    /// Browser version (e.g., "136.0.0.0")
    #[serde(rename = "browser_version")]
    pub browser_version: String,

    /// OS version (e.g., "10" for Windows 10)
    #[serde(rename = "os_version")]
    pub os_version: String,

    /// Referrer URL (usually empty)
    #[serde(rename = "referrer")]
    pub referrer: String,

    /// Referring domain (usually empty)
    #[serde(rename = "referring_domain")]
    pub referring_domain: String,

    /// Current referrer (usually empty)
    #[serde(rename = "referrer_current")]
    pub referrer_current: String,

    /// Current referring domain (usually empty)
    #[serde(rename = "referring_domain_current")]
    pub referring_domain_current: String,

    /// Release channel (e.g., "stable", "ptb", "canary")
    #[serde(rename = "release_channel")]
    pub release_channel: String,

    /// Discord client build number
    #[serde(rename = "client_build_number")]
    pub client_build_number: u32,

    /// Client event source (optional)
    #[serde(rename = "client_event_source", skip_serializing_if = "Option::is_none")]
    pub client_event_source: Option<String>,
}

impl SuperProperties {
    /// Creates a new SuperProperties instance with default values for Windows/Chrome
    #[must_use]
    pub fn new(build_number: u32, browser_version: u32) -> Self {
        let user_agent = Self::generate_user_agent(browser_version);

        Self {
            os: "Windows".to_string(),
            browser: "Chrome".to_string(),
            device: String::new(),
            system_locale: "en-US".to_string(),
            browser_user_agent: user_agent,
            browser_version: format!("{browser_version}.0.0.0"),
            os_version: "10".to_string(),
            referrer: String::new(),
            referring_domain: String::new(),
            referrer_current: String::new(),
            referring_domain_current: String::new(),
            release_channel: "stable".to_string(),
            client_build_number: build_number,
            client_event_source: None,
        }
    }

    /// Creates SuperProperties with fetched build number and browser version
    pub async fn fetch(client: &Client) -> Result<Self> {
        let build_number = Self::get_build_number(client).await.unwrap_or_else(|e| {
            warn!("Failed to fetch build number: {e}, using fallback");
            FALLBACK_BUILD_NUMBER
        });

        let browser_version = Self::get_browser_version(client).await.unwrap_or_else(|e| {
            warn!("Failed to fetch browser version: {e}, using fallback");
            FALLBACK_BROWSER_VERSION
        });

        Ok(Self::new(build_number, browser_version))
    }

    /// Fetches the Discord client build number from the login page
    pub async fn get_build_number(client: &Client) -> Result<u32> {
        if let Some(cached) = BUILD_NUMBER_CACHE.get() {
            return Ok(*cached);
        }

        debug!("Fetching Discord build number...");

        // Fetch the login page to get asset references
        let login_page = client
            .get("https://discord.com/login")
            .send()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?
            .text()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?;

        // Find sentry asset file
        let sentry_regex = Regex::new(r"assets/(sentry\.\w+)\.js")
            .map_err(|_| Error::Other("Invalid sentry regex"))?;

        let sentry_file = sentry_regex
            .captures(&login_page)
            .and_then(|cap| cap.get(1))
            .ok_or_else(|| Error::Other("Could not find sentry asset file"))?
            .as_str();

        // Fetch the sentry asset file
        let sentry_url = format!("https://discord.com/assets/{sentry_file}.js");
        let sentry_content = client
            .get(&sentry_url)
            .send()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?
            .text()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?;

        // Extract build number
        let build_regex = Regex::new(r#"buildNumber"\s*:\s*"(\d+)""#)
            .map_err(|_| Error::Other("Invalid build regex"))?;

        let build_number = build_regex
            .captures(&sentry_content)
            .and_then(|cap| cap.get(1))
            .ok_or_else(|| Error::Other("Could not find build number"))?
            .as_str()
            .parse::<u32>()
            .map_err(|_| Error::Other("Invalid build number format"))?;

        debug!("Found Discord build number: {build_number}");
        let _ = BUILD_NUMBER_CACHE.set(build_number);

        Ok(build_number)
    }

    /// Fetches the latest stable Chrome version from Google's API
    pub async fn get_browser_version(client: &Client) -> Result<u32> {
        if let Some(cached) = BROWSER_VERSION_CACHE.get() {
            return Ok(*cached);
        }

        debug!("Fetching Chrome browser version...");

        #[derive(Deserialize)]
        struct VersionInfo {
            version: String,
        }

        #[derive(Deserialize)]
        struct ChromeVersions {
            versions: Vec<VersionInfo>,
        }

        let response: ChromeVersions = client
            .get("https://versionhistory.googleapis.com/v1/chrome/platforms/win/channels/stable/versions")
            .send()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?
            .json::<ChromeVersions>()
            .await
            .map_err(|e| Error::Http(HttpError::Request(e)))?;

        let version = response
            .versions
            .first()
            .ok_or_else(|| Error::Other("No Chrome versions found"))?
            .version
            .split('.')
            .next()
            .ok_or_else(|| Error::Other("Invalid Chrome version format"))?
            .parse::<u32>()
            .map_err(|_| Error::Other("Invalid Chrome version number"))?;

        debug!("Found Chrome version: {version}");
        let _ = BROWSER_VERSION_CACHE.set(version);

        Ok(version)
    }

    /// Generates a Chrome user agent string for the given browser version
    #[must_use]
    pub fn generate_user_agent(version: u32) -> String {
        format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36"
        )
    }

    /// Encodes the super properties as a base64 JSON string for the X-Super-Properties header
    #[must_use]
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("Failed to serialize SuperProperties");
        base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
    }

    /// Decodes super properties from a base64 JSON string
    pub fn decode(encoded: &str) -> Result<Self> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(|_| Error::Other("Failed to decode base64"))?;

        let json_str = String::from_utf8(bytes)
            .map_err(|_| Error::Other("Invalid UTF-8 in super properties"))?;

        serde_json::from_str(&json_str)
            .map_err(|e| Error::Json(e))
    }

    /// Returns the user agent string
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.browser_user_agent
    }
}

impl Default for SuperProperties {
    fn default() -> Self {
        Self::new(FALLBACK_BUILD_NUMBER, FALLBACK_BROWSER_VERSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let props = SuperProperties::default();
        let encoded = props.encode();
        let decoded = SuperProperties::decode(&encoded).unwrap();

        assert_eq!(props.os, decoded.os);
        assert_eq!(props.browser, decoded.browser);
        assert_eq!(props.client_build_number, decoded.client_build_number);
    }

    #[test]
    fn test_user_agent_generation() {
        let ua = SuperProperties::generate_user_agent(136);
        assert!(ua.contains("Chrome/136.0.0.0"));
        assert!(ua.contains("Windows NT 10.0"));
    }
}
