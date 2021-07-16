use std::time::Duration;

use anyhow::anyhow;
use serenity::{
    client::Context,
    framework::standard::{macros::command, CommandResult},
    model::{
        channel::Message,
        id::{ChannelId, GuildId},
    },
    prelude::TypeMapKey,
};
use songbird::Event;

use crate::commands::{
    ChannelDurationNotifier, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE, NOT_IN_VOICE_CHANNEL_MESSAGE,
};

struct CurrentVoiceChannel;

impl TypeMapKey for CurrentVoiceChannel {
    type Value = ChannelId;
}

#[command]
#[only_in(guilds)]
pub async fn join(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not retrieve server info"))?;
    let guild_id = guild.id;

    let channel_id = match guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
    {
        Some(c) => c,
        None => {
            msg.reply(ctx, NOT_IN_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    };

    {
        if ctx.data.read().await.get::<CurrentVoiceChannel>().is_some() {
            // we are already in a voice channel. We require the bot to explicitly
            // leave a voice channel before it can join another one.
            msg.reply(ctx, "I'm already in a voice channel.").await?;
            return Ok(());
        }
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
    guild_id: GuildId,
) -> CommandResult {
    tracing::debug!(initiator = ?msg.author, ?channel_id, "Attempting to join voice channel");

    let manager = songbird::get(ctx)
        .await
        .expect("Songbird context should be there")
        .clone();

    let (handle_lock, success) = manager.join(guild_id, channel_id).await;

    if success.is_ok() {
        let mut handle = handle_lock.lock().await;

        handle.add_global_event(
            Event::Periodic(Duration::from_secs(60), None),
            ChannelDurationNotifier::default(),
        );

        ctx.data
            .write()
            .await
            .insert::<CurrentVoiceChannel>(channel_id);
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
    let guild_id = guild.id;

    // in general, can't ask the bot to do something unless you're in that channel.
    {
        if let Some(channel_id) = guild
            .voice_states
            .get(&msg.author.id)
            .and_then(|vs| vs.channel_id)
        {
            if let Some(c) = ctx.data.read().await.get::<CurrentVoiceChannel>() {
                if *c != channel_id {
                    msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
                    return Ok(());
                }
            } else {
                msg.reply(ctx, "I'm not in a voice channel.").await?;
                return Ok(());
            }
        } else {
            msg.reply(ctx, NOT_IN_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    }

    let manager = songbird::get(ctx)
        .await
        .expect("Songbird Voice client placed in at initialisation.")
        .clone();
    let has_handler = manager.get(guild_id).is_some();

    if has_handler {
        let mut data = ctx.data.write().await;

        if let Err(e) = manager.remove(guild_id).await {
            msg.channel_id
                .say(&ctx.http, format!("Failed: {:?}", e))
                .await?;
        }

        data.remove::<CurrentVoiceChannel>();

        msg.channel_id.say(&ctx.http, "Left voice channel").await?;
    } else {
        msg.reply(ctx, "Not in a voice channel").await?;
    }

    Ok(())
}
