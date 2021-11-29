use anyhow::anyhow;
use serenity::{
    async_trait,
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
use songbird::id::{ChannelId, GuildId};

use crate::commands::{
    get_songbird_from_ctx, get_voice_channel_id, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE,
    NOT_IN_VOICE_CHANNEL_MESSAGE,
};

#[command]
#[only_in(guilds)]
pub async fn skip(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not fetch guild"))?;
    let guild_id = GuildId::from(guild.id);

    let manager = get_songbird_from_ctx(ctx).await;

    let channel_id = match get_voice_channel_id(&guild, msg) {
        Some(c) => c,
        None => {
            msg.reply(ctx, NOT_IN_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    };

    if let Some(call_lock) = manager.get(guild_id) {
        // don't allow the action if the user is not in the same channel.
        let r = call_lock.lock().await;
        let c = r.current_channel().expect("there should be a channel here");
        if c != channel_id {
            msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }

        r.queue().skip()?;
    } else {
        msg.reply(ctx, "Not in a voice channel right now.").await?;
    }

    Ok(())
}

pub struct SkipCommand;

#[async_trait]
impl super::TugboatCommand for SkipCommand {
    async fn execute(
        &self,
        ctx: &Context,
        _options: &[ApplicationCommandInteractionDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        let manager = get_songbird_from_ctx(ctx).await;

        if let Some(call_lock) = manager.get(guild.id) {
            // don't allow the action if the user is not in the same channel.
            let r = call_lock.lock().await;
            let c = r.current_channel().expect("there should be a channel here");
            if c != channel_id {
                return Ok(NOT_IN_SAME_VOICE_CHANNEL_MESSAGE.into());
            }

            r.queue().skip()?;
        } else {
            return Ok("Not in a voice channel right now.".into());
        }

        Ok("Skipped.".into())
    }

    fn create_command(&self) -> CreateApplicationCommandOption {
        CreateApplicationCommandOption::default()
            .name("skip")
            .description("Skip the currently-playing track")
            .kind(ApplicationCommandOptionType::SubCommand)
            .clone()
    }

    fn get_name(&self) -> String {
        String::from("skip")
    }
}
