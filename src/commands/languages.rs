use serenity::{
    async_trait,
    builder::CreateApplicationCommandOption,
    client::Context,
    model::{
        application::command::CommandOptionType, guild::Guild,
        prelude::interaction::application_command::CommandDataOption,
    },
};
use songbird::id::ChannelId;

use super::say::Voices;

pub struct LanguagesCommand;

#[async_trait]
impl super::TugboatCommand for LanguagesCommand {
    async fn execute(
        &self,
        ctx: &Context,
        _options: &[CommandDataOption],
        _guild: Guild,
        _channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        let data = ctx.data.read().await;

        let voices = data
            .get::<Voices>()
            .expect("Should have been voices here")
            .keys()
            .map(|v| v.as_str())
            .collect::<Vec<_>>();

        let mut res = format!("{} languages available:\n", voices.len());
        res.push_str(&voices.join(", "));

        Ok(res)
    }

    fn create_command(&self) -> CreateApplicationCommandOption {
        CreateApplicationCommandOption::default()
            .name("languages")
            .description("Show all the languages currently supported by the bot")
            .kind(CommandOptionType::SubCommand)
            .clone()
    }

    fn get_name(&self) -> String {
        String::from("languages")
    }
}
