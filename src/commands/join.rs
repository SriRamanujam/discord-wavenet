use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use anyhow::anyhow;
use serenity::{
    builder::CreateApplicationCommandOption,
    client::Context,
    framework::standard::{macros::command, CommandResult},
    model::{channel::Message, interactions::application_command::ApplicationCommandOptionType},
};
use songbird::{
    error::JoinResult,
    id::{ChannelId, GuildId},
    Event, Songbird,
};

use crate::commands::{
    get_songbird_from_ctx, get_voice_channel_id, IdleDurationTracker, IdleDurations,
    NOT_IN_SAME_VOICE_CHANNEL_MESSAGE, NOT_IN_VOICE_CHANNEL_MESSAGE,
};

#[command]
#[only_in(guilds)]
pub async fn join(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not retrieve server info"))?;
    let guild_id = GuildId::from(guild.id);

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

pub fn create_command() -> CreateApplicationCommandOption {
    CreateApplicationCommandOption::default()
        .name("join")
        .description("Join your current voice channel")
        .kind(ApplicationCommandOptionType::SubCommand)
        .clone()
}

/// Inner function for joining mostly so that I can have the say command
/// automatically try to join a channel if it's not already in one.
#[tracing::instrument(skip(ctx))]
pub(super) async fn do_join(
    ctx: &Context,
    msg: &Message,
    channel_id: ChannelId,
    guild_id: GuildId,
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

#[command]
#[only_in(guilds)]
pub async fn leave(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not retrieve server info"))?;
    let guild_id = GuildId::from(guild.id);

    let manager = get_songbird_from_ctx(ctx).await;
    let channel_id = match get_voice_channel_id(&guild, msg) {
        Some(c) => c,
        None => {
            msg.reply(ctx, NOT_IN_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    };

    match manager.get(guild_id) {
        Some(call) => {
            // we are in a call right now, check to make sure the user is in the same channel.
            let call_channel = {
                let r = call.lock().await;
                r.current_channel().expect("There should be a channel here")
            };

            if channel_id != call_channel {
                msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
                return Ok(());
            }
        }
        None => {
            msg.reply(ctx, "I'm not in a voice channel.").await?;
            return Ok(());
        }
    }

    // if we've made it past all the checks, we are clear to remove ourselves from the channel.
    if let Err(e) = do_leave(manager, guild_id).await {
        msg.channel_id
            .say(&ctx.http, format!("Failed: {:?}", e))
            .await?;
    }

    Ok(())
}

pub(super) async fn do_leave(manager: Arc<Songbird>, guild_id: GuildId) -> JoinResult<()> {
    manager.remove(guild_id).await
}
