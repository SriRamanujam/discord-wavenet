use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{
        channel::Message,
        guild::{Guild, GuildStatus},
        id::GuildId as SerenityGuildId,
        prelude::Ready,
    },
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
    vec,
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
        tracing::debug!("Idle in voice channel for {} minutes!", idle_minutes);

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
}

fn get_voice_channel_id(guild: &Guild, msg: &Message) -> Option<ChannelId> {
    guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
        .map(ChannelId::from)
}

pub struct CommandHandler;

impl CommandHandler {
    async fn set_application_commands_on_guild(&self, guild_id: SerenityGuildId, ctx: &Context) {
        let options = vec![join::create_command(), say::create_command()];

        /*
        // TODO:
        // this does not scale, but I don't know of a better way. Just delete all the commands before
        // adding the new ones.
        // NB: don't actually commit this logic, or if you do, leave it commented-out or something.
        {
            let commands = guild_id.get_application_commands(&ctx.http).await.unwrap();
            for c in &commands {
                let x = guild_id.delete_application_command(&ctx.http, c.id).await;
                tracing::info!(?guild_id, ?c, "Deleted application command");
            }
        }
        */

        if let Err(e) = guild_id
            .set_application_commands(&ctx.http, |c| {
                c.create_application_command(|a| {
                    a.name("tugboat") // TODO: replace this with something configurable
                        .description("Tugboat commands")
                        .set_options(options)
                })
            })
            .await
        {
            tracing::error!(?guild_id, ?e, "Could not register application commands!");
        } else {
            tracing::info!(?guild_id, "Registered application commands");
        }
    }
}

#[async_trait]
impl EventHandler for CommandHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("ready callback has fired");
        for x in &ready.guilds {
            match x {
                GuildStatus::OnlinePartialGuild(g) => {
                    tracing::info!("Registering command for partial guild {:?}", g);
                    self.set_application_commands_on_guild(g.id, &ctx).await
                }
                GuildStatus::OnlineGuild(g) => {
                    tracing::info!("Registering command for guild {:?}", g);
                    self.set_application_commands_on_guild(g.id, &ctx).await
                }
                GuildStatus::Offline(g) => {
                    tracing::info!("Guild unavailable: {:?}", g);
                    self.set_application_commands_on_guild(g.id, &ctx).await
                }
                _ => {
                    tracing::info!("Uknown message");
                    continue;
                }
            }
        }
    }
}
