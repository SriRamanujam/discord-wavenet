use std::{sync::atomic::Ordering, time::Duration};

use anyhow::anyhow;
use serenity::async_trait;
use serenity::{
    builder::CreateApplicationCommandOption,
    client::Context,
    framework::standard::{macros::command, CommandResult},
    model::{
        channel::Message,
        guild::Guild,
        interactions::application_command::{
            ApplicationCommandInteractionDataOption, ApplicationCommandOptionType,
        },
    },
};
use songbird::{
    id::{ChannelId, GuildId as SongbirdGuildId},
    Event,
};

use super::TugboatCommand;
use crate::commands::{
    get_songbird_from_ctx, get_voice_channel_id, IdleDurationTracker, IdleDurations,
    NOT_IN_VOICE_CHANNEL_MESSAGE,
};

#[command]
#[only_in(guilds)]
pub async fn join(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not retrieve server info"))?;
    let guild_id = SongbirdGuildId::from(guild.id);

    // if the user is not in a voice channel, don't allow the join (what would we join into?)
    let channel_id = match get_voice_channel_id(&guild, msg) {
        Some(c) => c,
        None => {
            msg.reply(ctx, NOT_IN_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    };

    // if we're already in a call for this guild, don't allow the join.
    let manager = get_songbird_from_ctx(ctx).await;
    if manager.get(guild_id).is_some() {
        msg.reply(ctx, "I'm already in a voice channel").await?;
        return Ok(());
    }

    do_join(ctx, msg, channel_id, guild_id).await
}

/// Inner function for joining mostly so that I can have the say command
/// automatically try to join a channel if it's not already in one.
#[tracing::instrument(skip(ctx))]
pub(super) async fn do_join(
    ctx: &Context,
    msg: &Message,
    channel_id: ChannelId,
    guild_id: SongbirdGuildId,
) -> CommandResult {
    tracing::debug!(initiator = ?msg.author, ?channel_id, "Attempting to join voice channel");

    let manager = get_songbird_from_ctx(ctx).await;
    let (handle_lock, success) = manager.join(guild_id, channel_id).await;

    if success.is_ok() {
        // create the duration tracking atomic usize
        let duration_tracking = {
            let mut data = ctx.data.write().await;
            let durations = data
                .get_mut::<IdleDurations>()
                .expect("join durations hashmap should be here");
            let d = durations.entry(guild_id).or_default();
            d.store(0, Ordering::SeqCst);
            d.clone()
        };

        {
            let mut handle = handle_lock.lock().await;

            handle.add_global_event(
                Event::Periodic(Duration::from_secs(60), None),
                IdleDurationTracker::new(duration_tracking, manager.clone(), guild_id),
            );
        }
    } else {
        msg.channel_id
            .say(&ctx.http, "Could not join voice channel.")
            .await?;
    }

    Ok(())
}

pub struct JoinCommand;

#[async_trait]
impl TugboatCommand for JoinCommand {
    async fn execute(
        &self,
        ctx: &Context,
        options: &[ApplicationCommandInteractionDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        tracing::debug!(?guild, ?channel_id, "Attempting to join voice channel");

        let manager = get_songbird_from_ctx(ctx).await;
        let (handle_lock, success) = manager.join(guild.id, channel_id).await;

        if success.is_ok() {
            // create the duration tracking atomic usize
            let duration_tracking = {
                let mut data = ctx.data.write().await;
                let durations = data
                    .get_mut::<IdleDurations>()
                    .expect("join durations hashmap should be here");
                let d = durations
                    .entry(songbird::id::GuildId::from(guild.id))
                    .or_default();
                d.store(0, Ordering::SeqCst);
                d.clone()
            };

            {
                let mut handle = handle_lock.lock().await;

                handle.add_global_event(
                    Event::Periodic(Duration::from_secs(60), None),
                    IdleDurationTracker::new(
                        duration_tracking,
                        manager.clone(),
                        songbird::id::GuildId::from(guild.id),
                    ),
                );
            }
        } else {
            return Err(anyhow!("Could not join voice channel."));
        }

        Ok("Joined voice channel".into())
    }

    fn create_command(&self) -> CreateApplicationCommandOption {
        CreateApplicationCommandOption::default()
            .name("join")
            .description("Join your current voice channel")
            .kind(ApplicationCommandOptionType::SubCommand)
            .clone()
    }

    fn get_name(&self) -> String {
        String::from("join")
    }
}
