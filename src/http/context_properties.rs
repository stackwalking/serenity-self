use base64::Engine;

use serde_json::json;

use crate::model::channel::ChannelType;
use crate::model::id::{ChannelId, GuildId, MessageId};

/// Represents the Discord X-Context-Properties header.
///
/// This header is essential for certain user actions (e.g. creating invites, joining guilds,
/// friend requesting) to avoid detection/blocking.
#[derive(Debug, Clone)]
pub enum ContextProperties {
    /// Empty context properties: {}
    Empty,
    /// Guild Header location
    GuildHeader,
    /// Context Menu location
    ContextMenu,
    /// Group DM Invite Create location
    GroupDmInviteCreate,
    /// Chat input location
    ChatInput,
    /// Accept Invite Page location (with optional guild/channel context)
    AcceptInvitePage {
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        channel_type: Option<ChannelType>,
    },
    /// Join Guild location (with optional guild/channel context)
    JoinGuild {
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        channel_type: Option<ChannelType>,
    },
    /// Invite Button Embed location (from a message with invite button)
    InviteButtonEmbed {
        guild_id: Option<GuildId>,
        channel_id: ChannelId,
        message_id: MessageId,
        channel_type: Option<ChannelType>,
    },
}

impl ContextProperties {
    /// Returns an empty context properties object
    #[must_use]
    pub const fn empty() -> Self {
        Self::Empty
    }

    /// Returns context properties for guild header
    #[must_use]
    pub const fn guild_header() -> Self {
        Self::GuildHeader
    }

    /// Returns context properties for context menu
    #[must_use]
    pub const fn context_menu() -> Self {
        Self::ContextMenu
    }

    /// Returns context properties for group DM invite creation
    #[must_use]
    pub const fn group_dm_invite_create() -> Self {
        Self::GroupDmInviteCreate
    }

    /// Returns context properties for chat input
    #[must_use]
    pub const fn chat_input() -> Self {
        Self::ChatInput
    }

    /// Randomly selects between GuildHeader and ContextMenu
    /// This mimics the Python implementation's random selection for create_invite
    #[must_use]
    pub fn random_invite_context() -> Self {
        // Use a simple alternating pattern since we don't have rand dependency
        // In practice, this provides similar behavior to random selection
        use std::sync::atomic::{AtomicBool, Ordering};
        static TOGGLE: AtomicBool = AtomicBool::new(false);

        if TOGGLE.fetch_xor(true, Ordering::Relaxed) {
            Self::GuildHeader
        } else {
            Self::ContextMenu
        }
    }

    /// Encodes the context properties as a base64-encoded JSON string
    /// for use in the X-Context-Properties header
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            Self::Empty => "e30=".to_string(),
            Self::GuildHeader => "eyJsb2NhdGlvbiI6Ikd1aWxkIEhlYWRlciJ9".to_string(),
            Self::ContextMenu => "eyJsb2NhdGlvbiI6IkNvbnRleHRNZW51In0=".to_string(),
            Self::GroupDmInviteCreate => {
                "eyJsb2NhdGlvbiI6Ikdyb3VwIERNIEludml0ZSBDcmVhdGUifQ==".to_string()
            },
            Self::ChatInput => "eyJsb2NhdGlvbiI6ImNoYXRfaW5wdXQifQ==".to_string(),
            Self::AcceptInvitePage {
                guild_id,
                channel_id,
                channel_type,
            } => {
                let mut data = json!({
                    "location": "Accept Invite Page",
                });
                if let Some(gid) = guild_id {
                    data["location_guild_id"] = json!(gid.to_string());
                }
                if let Some(cid) = channel_id {
                    data["location_channel_id"] = json!(cid.to_string());
                }
                if let Some(ct) = channel_type {
                    data["location_channel_type"] = json!(u8::from(*ct));
                }
                Self::custom(data)
            },
            Self::JoinGuild {
                guild_id,
                channel_id,
                channel_type,
            } => {
                let mut data = json!({
                    "location": "Join Guild",
                });
                if let Some(gid) = guild_id {
                    data["location_guild_id"] = json!(gid.to_string());
                }
                if let Some(cid) = channel_id {
                    data["location_channel_id"] = json!(cid.to_string());
                }
                if let Some(ct) = channel_type {
                    data["location_channel_type"] = json!(u8::from(*ct));
                }
                Self::custom(data)
            },
            Self::InviteButtonEmbed {
                guild_id,
                channel_id,
                message_id,
                channel_type,
            } => {
                let mut data = json!({
                    "location": "Invite Button Embed",
                    "location_channel_id": channel_id.to_string(),
                    "location_message_id": message_id.to_string(),
                });
                if let Some(gid) = guild_id {
                    data["location_guild_id"] = json!(gid.to_string());
                }
                if let Some(ct) = channel_type {
                    data["location_channel_type"] = json!(u8::from(*ct));
                }
                Self::custom(data)
            },
        }
    }

    /// Creates a custom context properties with arbitrary JSON data
    #[must_use]
    pub fn custom(data: serde_json::Value) -> String {
        let json_str = data.to_string();
        base64::engine::general_purpose::STANDARD.encode(json_str.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_encoding() {
        assert_eq!(ContextProperties::empty().encode(), "e30=");
    }

    #[test]
    fn test_guild_header_encoding() {
        let encoded = ContextProperties::guild_header().encode();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .unwrap();
        let json_str = String::from_utf8(decoded).unwrap();
        assert_eq!(json_str, r#"{"location":"Guild Header"}"#);
    }

    #[test]
    fn test_group_dm_invite_create() {
        let encoded = ContextProperties::group_dm_invite_create().encode();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .unwrap();
        let json_str = String::from_utf8(decoded).unwrap();
        assert_eq!(json_str, r#"{"location":"Group DM Invite Create"}"#);
    }
}
