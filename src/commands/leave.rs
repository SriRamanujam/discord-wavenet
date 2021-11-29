use std::sync::Arc;


use serenity::{
    async_trait,
    builder::CreateApplicationCommandOption,
    client::Context,
    model::{
        guild::Guild,
        interactions::application_command::{
            ApplicationCommandInteractionDataOption, ApplicationCommandOptionType,
        },
    },
};
use songbird::{
    error::JoinResult,
    id::{ChannelId, GuildId},
    Songbird,
};

use crate::commands::{
    get_songbird_from_ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE,
};

pub(super) async fn do_leave(manager: Arc<Songbird>, guild_id: GuildId) -> JoinResult<()> {
    manager.remove(guild_id).await
}

pub struct LeaveCommand;

#[async_trait]
impl super::TugboatCommand for LeaveCommand {
    async fn execute(
        &self,
        ctx: &Context,
        _options: &[ApplicationCommandInteractionDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        let manager = get_songbird_from_ctx(ctx).await;
        match manager.get(guild.id) {
            Some(call) => {
                // we are in a call right now, check to make sure the user is in the same channel.
                let call_channel = {
                    let r = call.lock().await;
                    r.current_channel().expect("There should be a channel here")
                };

                if channel_id != call_channel {
                    return Ok(NOT_IN_SAME_VOICE_CHANNEL_MESSAGE.into());
                }
            }
            None => {
                return Ok("I'm not in a voice channel.".into());
            }
        }

        // if we've made it past all the checks, we are clear to remove ourselves from the channel.
        do_leave(manager, songbird::id::GuildId::from(guild.id)).await?;

        Ok("Left voice channel".into())
    }

    fn create_command(&self) -> CreateApplicationCommandOption {
        CreateApplicationCommandOption::default()
            .name("leave")
            .description("Leave the currently-joined voice channel")
            .kind(ApplicationCommandOptionType::SubCommand)
            .clone()
    }

    fn get_name(&self) -> String {
        String::from("leave")
    }
}
