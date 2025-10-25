//! Guild subscription management for user accounts.
//!
//! User accounts must subscribe to guilds to receive message content and events in larger servers.
//! This module handles automatic subscription on Ready/GuildCreate events and caches the
//! subscription state.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use parking_lot::RwLock;
use tracing::{debug, warn};

use crate::gateway::{GuildSubscribeOptions, WsClient};
use crate::json::to_string;
use crate::model::gateway::ShardInfo;
use crate::model::id::{ChannelId, GuildId, UserId};
use crate::Result;

/// Maximum payload size for bulk guild subscribe (15 KiB)
const MAX_PAYLOAD_SIZE: usize = 15360;

/// Manages guild subscriptions within the cache.
///
/// This handles automatic subscription to guilds when they become available,
/// tracking subscription state, and firing cache_ready when all guilds are loaded.
#[derive(Debug)]
pub struct GuildSubscriptions {
    /// Pending subscriptions to be sent
    pending: Arc<RwLock<HashMap<String, GuildSubscribeOptions>>>,
    /// Guilds that are currently subscribed
    subscribed: Arc<RwLock<HashSet<GuildId>>>,
    /// Guilds subscribed to typing events
    typing: Arc<RwLock<HashSet<GuildId>>>,
    /// Guilds subscribed to thread events
    threads: Arc<RwLock<HashSet<GuildId>>>,
    /// Guilds subscribed to activity events
    activities: Arc<RwLock<HashSet<GuildId>>>,
    /// Guilds subscribed to member update events
    member_updates: Arc<RwLock<HashSet<GuildId>>>,
    /// Per-guild member subscriptions
    members: Arc<RwLock<HashMap<GuildId, HashSet<UserId>>>>,
    /// Per-guild thread member list subscriptions
    thread_member_lists: Arc<RwLock<HashMap<GuildId, HashSet<ChannelId>>>>,
    /// Per-guild channel range subscriptions
    channels: Arc<RwLock<HashMap<GuildId, HashMap<ChannelId, Vec<Vec<u32>>>>>>,
    /// Whether the manager is blocked from sending
    blocked: Arc<RwLock<bool>>,
}

impl GuildSubscriptions {
    /// Creates a new guild subscription manager.
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            subscribed: Arc::new(RwLock::new(HashSet::new())),
            typing: Arc::new(RwLock::new(HashSet::new())),
            threads: Arc::new(RwLock::new(HashSet::new())),
            activities: Arc::new(RwLock::new(HashSet::new())),
            member_updates: Arc::new(RwLock::new(HashSet::new())),
            members: Arc::new(RwLock::new(HashMap::new())),
            thread_member_lists: Arc::new(RwLock::new(HashMap::new())),
            channels: Arc::new(RwLock::new(HashMap::new())),
            blocked: Arc::new(RwLock::new(false)),
        }
    }

    /// Marks a guild as initially subscribed (typically from has_threads_subscription flag).
    pub fn initial_subscription(&self, guild_id: GuildId) {
        let mut subscribed = self.subscribed.write();
        let mut typing = self.typing.write();
        let mut threads = self.threads.write();
        let mut activities = self.activities.write();

        subscribed.insert(guild_id);
        typing.insert(guild_id);
        threads.insert(guild_id);
        activities.insert(guild_id);
    }

    /// Clears all subscription state.
    pub fn clear(&self) {
        self.subscribed.write().clear();
        self.typing.write().clear();
        self.threads.write().clear();
        self.activities.write().clear();
        self.member_updates.write().clear();
        self.members.write().clear();
        self.thread_member_lists.write().clear();
        self.channels.write().clear();
        self.pending.write().clear();
    }

    /// Checks if subscriptions are empty.
    pub fn is_empty(&self) -> bool {
        self.subscribed.read().is_empty() && self.pending.read().is_empty()
    }

    /// Sets the blocked state.
    pub fn set_blocked(&self, blocked: bool) {
        *self.blocked.write() = blocked;
    }

    /// Checks if the manager is currently blocked.
    pub fn is_blocked(&self) -> bool {
        *self.blocked.read()
    }

    /// Checks if a guild is subscribed or pending subscription.
    pub fn is_pending_subscribe(&self, guild_id: GuildId) -> bool {
        if self.subscribed.read().contains(&guild_id) {
            return true;
        }

        let pending = self.pending.read();
        if let Some(opts) = pending.get(&guild_id.to_string()) {
            opts.typing == Some(true)
        } else {
            false
        }
    }

    /// Subscribes to a guild with the specified options.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload is too large or if not subscribed to typing.
    pub fn subscribe_to(
        &self,
        guild_id: GuildId,
        typing: Option<bool>,
        threads: Option<bool>,
        activities: Option<bool>,
        member_updates: Option<bool>,
    ) -> Result<()> {
        // Sanity check: must subscribe to typing if not already subscribed
        let is_pending = self.is_pending_subscribe(guild_id);
        let typing = if !is_pending {
            if typing == Some(false) {
                return Err(crate::Error::Other(
                    "Cannot subscribe to guild without subscribing to typing",
                ));
            }
            Some(typing.unwrap_or(true))
        } else {
            typing
        };

        let mut options = GuildSubscribeOptions::default();
        options.typing = typing;
        options.threads = threads;
        options.activities = activities;
        options.member_updates = member_updates;

        let mut changes = HashMap::new();
        changes.insert(guild_id.to_string(), options);

        self.checked_add(changes)
    }

    /// Subscribes to specific members within a guild.
    pub fn subscribe_to_members(
        &self,
        guild_id: GuildId,
        user_ids: Vec<UserId>,
        replace: bool,
    ) -> Result<()> {
        if !replace && user_ids.is_empty() {
            return Ok(());
        }

        // Must be subscribed to guild first
        if !self.is_pending_subscribe(guild_id) {
            return Err(crate::Error::Other(
                "Cannot subscribe to members without first subscribing to guild",
            ));
        }

        let mut user_set: HashSet<UserId> = user_ids.into_iter().collect();

        if !replace {
            let members = self.members.read();
            if let Some(existing) = members.get(&guild_id) {
                user_set.extend(existing.iter().copied());
            }
        }

        let mut options = GuildSubscribeOptions::default();
        options.members = Some(user_set.into_iter().collect());

        let mut changes = HashMap::new();
        changes.insert(guild_id.to_string(), options);

        self.checked_add(changes)
    }

    /// Subscribes to thread member lists within a guild.
    pub fn subscribe_to_threads(
        &self,
        guild_id: GuildId,
        thread_ids: Vec<ChannelId>,
        replace: bool,
    ) -> Result<()> {
        if !replace && thread_ids.is_empty() {
            return Ok(());
        }

        // Must be subscribed to guild first
        if !self.is_pending_subscribe(guild_id) {
            return Err(crate::Error::Other(
                "Cannot subscribe to threads without first subscribing to guild",
            ));
        }

        let mut thread_set: HashSet<ChannelId> = thread_ids.into_iter().collect();

        if !replace {
            let threads = self.thread_member_lists.read();
            if let Some(existing) = threads.get(&guild_id) {
                thread_set.extend(existing.iter().copied());
            }
        }

        let mut options = GuildSubscribeOptions::default();
        options.thread_member_lists = Some(thread_set.into_iter().collect());

        let mut changes = HashMap::new();
        changes.insert(guild_id.to_string(), options);

        self.checked_add(changes)
    }

    /// Adds subscription changes to the pending queue, checking payload size.
    fn checked_add(&self, changes: HashMap<String, GuildSubscribeOptions>) -> Result<()> {
        // Merge new changes with existing pending
        let mut new_payload = {
            let pending = self.pending.read();
            pending.clone()
        };

        for (guild_id, new_opts) in changes.iter() {
            if let Some(existing) = new_payload.get_mut(guild_id) {
                // Merge options
                if new_opts.typing.is_some() {
                    existing.typing = new_opts.typing;
                }
                if new_opts.threads.is_some() {
                    existing.threads = new_opts.threads;
                }
                if new_opts.activities.is_some() {
                    existing.activities = new_opts.activities;
                }
                if new_opts.member_updates.is_some() {
                    existing.member_updates = new_opts.member_updates;
                }
                if new_opts.members.is_some() {
                    existing.members.clone_from(&new_opts.members);
                }
                if new_opts.channels.is_some() {
                    existing.channels.clone_from(&new_opts.channels);
                }
                if new_opts.thread_member_lists.is_some() {
                    existing.thread_member_lists.clone_from(&new_opts.thread_member_lists);
                }
            } else {
                new_payload.insert(guild_id.clone(), new_opts.clone());
            }
        }

        // Check payload size
        let payload_json = to_string(&new_payload)?;
        if payload_json.len() > MAX_PAYLOAD_SIZE {
            // Check if the changes alone are too large
            let changes_json = to_string(&changes)?;
            if changes_json.len() > MAX_PAYLOAD_SIZE {
                return Err(crate::Error::Other(
                    "Guild subscription payload too large to send",
                ));
            }

            // Need to flush, but we can't do it here synchronously
            // Just log a warning and add to pending anyway
            warn!("Guild subscription payload exceeds max size, may need immediate flush");
        }

        *self.pending.write() = new_payload;
        Ok(())
    }

    /// Sends pending subscriptions immediately via the websocket.
    ///
    /// # Errors
    ///
    /// Returns an error if sending fails.
    pub async fn flush(&self, ws_client: &mut WsClient, shard_info: &ShardInfo) -> Result<()> {
        let payload = {
            let pending = self.pending.read();
            if pending.is_empty() {
                debug!("No pending subscriptions to flush");
                return Ok(());
            }
            if self.is_blocked() {
                warn!("Subscription manager is blocked, cannot flush");
                return Ok(());
            }
            pending.clone()
        };

        debug!("Flushing {} guild subscriptions: {:?}", payload.len(), payload.keys().collect::<Vec<_>>());

        // Send the subscription request
        ws_client.send_bulk_guild_subscribe(shard_info, &payload).await?;

        // Update internal state after successful send
        for (guild_id_str, opts) in &payload {
            if let Ok(guild_id_num) = guild_id_str.parse::<u64>() {
                let guild_id = GuildId::new(guild_id_num);

                if opts.typing == Some(true) {
                    self.subscribed.write().insert(guild_id);
                }

                // Update feature subscriptions
                Self::update_feature_subscription(&self.typing, guild_id, opts.typing);
                Self::update_feature_subscription(&self.threads, guild_id, opts.threads);
                Self::update_feature_subscription(&self.activities, guild_id, opts.activities);
                Self::update_feature_subscription(&self.member_updates, guild_id, opts.member_updates);

                // Update member subscriptions
                if let Some(ref user_ids) = opts.members {
                    let mut members = self.members.write();
                    if user_ids.is_empty() {
                        members.remove(&guild_id);
                    } else {
                        members.insert(guild_id, user_ids.iter().copied().collect());
                    }
                }

                // Update thread subscriptions
                if let Some(ref thread_ids) = opts.thread_member_lists {
                    let mut threads = self.thread_member_lists.write();
                    if thread_ids.is_empty() {
                        threads.remove(&guild_id);
                    } else {
                        threads.insert(guild_id, thread_ids.iter().copied().collect());
                    }
                }

                // Update channel subscriptions
                if let Some(ref channel_map) = opts.channels {
                    let mut channels = self.channels.write();
                    if channel_map.is_empty() {
                        channels.remove(&guild_id);
                    } else {
                        channels.insert(guild_id, channel_map.clone());
                    }
                }
            }
        }

        self.pending.write().clear();
        Ok(())
    }

    /// Helper to update feature subscription state.
    fn update_feature_subscription(
        set: &RwLock<HashSet<GuildId>>,
        guild_id: GuildId,
        value: Option<bool>,
    ) {
        if let Some(val) = value {
            let mut lock = set.write();
            if val {
                lock.insert(guild_id);
            } else {
                lock.remove(&guild_id);
            }
        }
    }

    /// Requeues all subscriptions (used after reconnect).
    pub async fn requeue_subscriptions(&self, guild_ids: &[GuildId]) -> Result<()> {
        let (subscribed, typing, threads, activities, member_updates, members, thread_member_lists, channels) = {
            let subscribed = self.subscribed.read();
            let typing = self.typing.read();
            let threads = self.threads.read();
            let activities = self.activities.read();
            let member_updates = self.member_updates.read();
            let members = self.members.read();
            let thread_member_lists = self.thread_member_lists.read();
            let channels = self.channels.read();

            (
                subscribed.clone(),
                typing.clone(),
                threads.clone(),
                activities.clone(),
                member_updates.clone(),
                members.clone(),
                thread_member_lists.clone(),
                channels.clone(),
            )
        };

        for &guild_id in guild_ids {
            if !subscribed.contains(&guild_id) {
                continue;
            }

            let key = guild_id.to_string();
            let payload = GuildSubscribeOptions {
                typing: Some(typing.contains(&guild_id) || subscribed.contains(&guild_id)),
                threads: Some(threads.contains(&guild_id)),
                activities: Some(activities.contains(&guild_id)),
                member_updates: Some(member_updates.contains(&guild_id)),
                members: members.get(&guild_id).map(|s| s.iter().copied().collect()),
                thread_member_lists: thread_member_lists
                    .get(&guild_id)
                    .map(|s| s.iter().copied().collect()),
                channels: channels.get(&guild_id).cloned(),
            };

            let mut changes = HashMap::new();
            changes.insert(key, payload);
            self.checked_add(changes)?;
        }

        Ok(())
    }

    /// Checks if a guild is subscribed.
    pub fn is_subscribed(&self, guild_id: GuildId) -> bool {
        self.subscribed.read().contains(&guild_id)
    }
}

impl Default for GuildSubscriptions {
    fn default() -> Self {
        Self::new()
    }
}
