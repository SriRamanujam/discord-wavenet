use std::sync::Arc;

use anyhow::anyhow;
use serenity::{
    builder::CreateApplicationCommandOption,
    client::Context,
    framework::standard::{macros::command, CommandResult},
    model::{channel::Message, interactions::application_command::ApplicationCommandOptionType},
};
use songbird::{error::JoinResult, id::GuildId, Songbird};

use crate::commands::{
    get_songbird_from_ctx, get_voice_channel_id, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE,
    NOT_IN_VOICE_CHANNEL_MESSAGE,
};

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

pub fn create_command() -> CreateApplicationCommandOption {
    CreateApplicationCommandOption::default()
        .name("leave")
        .description("Leave the currently-joined voice channel")
        .kind(ApplicationCommandOptionType::SubCommand)
        .clone()
}
