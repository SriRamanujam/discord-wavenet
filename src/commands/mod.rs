use anyhow::anyhow;
use serenity::{
    async_trait,
    builder::CreateApplicationCommandOption,
    client::{Context, EventHandler},
    http::Http,
    model::{
        application::{
            command::Command,
            interaction::{
                application_command::{ApplicationCommandInteraction, CommandDataOption},
                Interaction,
            },
        },
        guild::Guild,
        id::GuildId as SerenityGuildId,
        prelude::{interaction::InteractionResponseType, Ready, User},
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
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    vec,
};

use crate::commands::leave::do_leave;

pub mod join;
pub(crate) mod languages;
pub(crate) mod leave;
pub mod say;
pub(crate) mod skip;

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

fn get_voice_channel_by_user(guild: &Guild, user: &User) -> Option<ChannelId> {
    guild
        .voice_states
        .get(&user.id)
        .and_then(|vs| vs.channel_id)
        .map(ChannelId::from)
}

pub struct CommandsMap;
pub type Commands = HashMap<String, Arc<dyn TugboatCommand + Send + Sync + 'static>>;
impl TypeMapKey for CommandsMap {
    type Value = Commands;
}

pub enum CommandScope {
    Guild,
    Global,
}

impl FromStr for CommandScope {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "guild" => Ok(Self::Guild),
            "global" => Ok(Self::Global),
            _ => Err(anyhow!(
                "Invalid value of command scope, can only be one of either 'guild' or 'global'"
            )),
        }
    }
}

/// Static registration of all new commands. Yes this is rather inconvenient,
/// but it'll work for now.
pub fn register_commands() -> Commands {
    let v: Vec<Arc<dyn TugboatCommand + Send + Sync>> = vec![
        Arc::new(say::SayCommand),
        Arc::new(join::JoinCommand),
        Arc::new(leave::LeaveCommand),
        Arc::new(skip::SkipCommand),
        Arc::new(languages::LanguagesCommand),
    ];

    v.into_iter()
        .map(|c| (c.get_name(), c))
        .collect::<Commands>()
}

#[async_trait]
pub trait TugboatCommand {
    async fn execute(
        &self,
        ctx: &Context,
        options: &[CommandDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String>;
    fn create_command(&self) -> CreateApplicationCommandOption;
    fn get_name(&self) -> String;
}

pub struct ApplicationCommandHandler {
    pub prefix: String,
    pub scope: CommandScope,
}

impl ApplicationCommandHandler {
    async fn set_commands_global(&self, ctx: &Context) {
        let options = ctx
            .data
            .read()
            .await
            .get::<CommandsMap>()
            .expect("Should have been commands here")
            .values()
            .map(|comm| comm.create_command())
            .collect::<Vec<_>>();

        if let Err(err) = Command::create_global_application_command(&ctx.http, |c| {
            c.name(&self.prefix)
                .description("Commands")
                .set_options(options)
        })
        .await
        {
            tracing::error!(?err, "Could not set global application commands")
        } else {
            tracing::info!("Registered global application commands");
        }
    }

    async fn set_commands_guild(&self, guild_id: SerenityGuildId, ctx: &Context) {
        let options = ctx
            .data
            .read()
            .await
            .get::<CommandsMap>()
            .expect("Should have been commands here")
            .values()
            .map(|comm| comm.create_command())
            .collect::<Vec<_>>();

        if let Err(e) = guild_id
            .set_application_commands(&ctx.http, |c| {
                c.create_application_command(|a| {
                    a.name(&self.prefix) // TODO: replace this with something configurable
                        .description("Commands")
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

    async fn send_interaction_response(
        &self,
        http: &impl AsRef<Http>,
        command: &ApplicationCommandInteraction,
        content: &str,
    ) {
        tracing::debug!(content, "Sending interaction response");
        if let Err(e) = command
            .create_interaction_response(http, |r| {
                r.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|message| message.content(content.to_owned()))
            })
            .await
        {
            tracing::error!(?e, "Could not respond to slash command");
        } else {
            tracing::debug!("Application command response sent successfully!");
        }
    }
}

#[async_trait]
impl EventHandler for ApplicationCommandHandler {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::ApplicationCommand(command) = interaction {
            tracing::info!(data=?command.data, "got command interaction!");

            // since our top level command is always tugboat, we are interested in the first child of the options.
            let incoming = &command.data.options[0];

            // gather up guild and channel info
            let guild = match command.guild_id {
                Some(g) => match g.to_guild_cached(&ctx.cache) {
                    Some(gu) => gu,
                    None => {
                        tracing::error!(guild_id=?g, "Could not find guild in cache!");
                        return;
                    }
                },
                None => {
                    return self
                        .send_interaction_response(
                            &ctx.http,
                            &command,
                            "Can't call this from a non-guild context",
                        )
                        .await
                }
            };

            let channel_id = match get_voice_channel_by_user(&guild, &command.user) {
                Some(c) => c,
                None => {
                    return self
                        .send_interaction_response(
                            &ctx.http,
                            &command,
                            NOT_IN_VOICE_CHANNEL_MESSAGE,
                        )
                        .await
                }
            };

            let dispatched_command = {
                let data = ctx.data.read().await;
                let commands = data
                    .get::<CommandsMap>()
                    .expect("Should have been commands here");
                commands.get(&incoming.name).cloned()
            };

            let response = {
                match dispatched_command {
                    Some(c) => {
                        tracing::debug!(
                            requested_comm = incoming.name.as_str(),
                            "Dispatching command"
                        );
                        let r = c.execute(&ctx, &incoming.options, guild, channel_id).await;
                        tracing::debug!(result=?r, "We have received a result from our command!");
                        r
                    }
                    None => Err(anyhow!("Unknown command {}", &incoming.name)),
                }
            };

            // dispatch to the relevant command in our command struct.
            match response {
                Ok(s) => {
                    tracing::trace!("We received a successful response, sending back result");
                    self.send_interaction_response(&ctx.http, &command, &s)
                        .await
                }
                Err(e) => {
                    tracing::error!(?e, guild_id=?command.guild_id, "Error completing interaction");
                    self.send_interaction_response(
                        &ctx.http,
                        &command,
                        "Error completing interaction.",
                    )
                    .await
                }
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        match self.scope {
            CommandScope::Guild => {
                for g in &ready.guilds {
                    if !g.unavailable {
                        tracing::info!("Registering command for guild {:?}", g.id);
                        self.set_commands_guild(g.id, &ctx).await;
                    }
                }
            }
            CommandScope::Global => {
                tracing::info!("Registering global commands");
                self.set_commands_global(&ctx).await;
            }
        }

        tracing::info!("Bot is ready!");
    }
}
