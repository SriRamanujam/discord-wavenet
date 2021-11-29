
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
use songbird::id::ChannelId;

use crate::commands::{
    get_songbird_from_ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE,
};

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
