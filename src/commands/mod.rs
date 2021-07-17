use serenity::{
    async_trait,
    client::Context,
    model::{channel::Message, guild::Guild},
    prelude::TypeMapKey,
};
use songbird::{
    events::EventHandler as VoiceEventHandler,
    id::{ChannelId, GuildId},
    Event, EventContext, Songbird,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use crate::commands::join::do_leave;

pub mod join;
pub mod say;

const NOT_IN_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in a voice channel!";
const NOT_IN_SAME_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in the same voice channel!";

pub struct IdleDurations;
impl TypeMapKey for IdleDurations {
    type Value = HashMap<GuildId, Arc<AtomicUsize>>;
}

struct IdleDurationTracker {
    idle_count_minutes: Arc<AtomicUsize>,
    manager: Arc<Songbird>,
    guild_id: GuildId,
}

impl IdleDurationTracker {
    pub fn new(u: Arc<AtomicUsize>, manager: Arc<Songbird>, guild_id: GuildId) -> Self {
        Self {
            idle_count_minutes: u,
            manager,
            guild_id,
        }
    }
}

#[async_trait]
impl VoiceEventHandler for IdleDurationTracker {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let idle_minutes = self.idle_count_minutes.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!("Idle in voice channel for {} minutes!", idle_minutes);

        // if we've been idle in the channel for 10 minutes, leave.
        if idle_minutes >= 10 {
            tracing::info!(
                "Idle for 10+ minutes in guild {:?}, leaving!",
                self.guild_id
            );
            let _ = do_leave(self.manager.clone(), self.guild_id).await;
        }

        None
    }
}

async fn get_songbird_from_ctx(ctx: &Context) -> Arc<Songbird> {
    songbird::get(ctx)
        .await
        .expect("Songbird context should be present")
        .clone()
}

fn get_voice_channel_id(guild: &Guild, msg: &Message) -> Option<ChannelId> {
    guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
        .map(|c| ChannelId::from(c))
}
