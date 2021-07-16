use serenity::{async_trait, model::id::ChannelId, prelude::TypeMapKey};
use songbird::{events::EventHandler as VoiceEventHandler, Event, EventContext};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub mod join;
pub mod say;

const NOT_IN_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in a voice channel!";
const NOT_IN_SAME_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in the same voice channel!";

#[derive(Default)]
struct ChannelDurationNotifier {
    count: Arc<AtomicUsize>,
}

#[async_trait]
impl VoiceEventHandler for ChannelDurationNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        // TODO: automatically leave channel after some amount of time spent idle
        let count_before = self.count.fetch_add(1, Ordering::Relaxed);
        tracing::info!("Been in voice channel for {} minutes!", count_before);
        None
    }
}

struct CurrentVoiceChannel;

impl TypeMapKey for CurrentVoiceChannel {
    type Value = ChannelId;
}
