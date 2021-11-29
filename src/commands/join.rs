use std::{sync::atomic::Ordering, time::Duration};

use anyhow::anyhow;
use serenity::async_trait;
use serenity::{
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
    id::ChannelId,
    Event,
};

use super::TugboatCommand;
use crate::commands::{
    get_songbird_from_ctx, IdleDurationTracker, IdleDurations,
};

pub struct JoinCommand;

#[async_trait]
impl TugboatCommand for JoinCommand {
    async fn execute(
        &self,
        ctx: &Context,
        _options: &[ApplicationCommandInteractionDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        tracing::debug!(guild=?guild.id, ?channel_id, "Attempting to join voice channel");

        let manager = get_songbird_from_ctx(ctx).await;
        let (handle_lock, success) = manager.join(guild.id, channel_id).await;

        if success.is_ok() {
            tracing::trace!("Join complete");
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
                tracing::trace!("Created durationg tracking atomic usize");
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
                tracing::trace!("Created timer to count down 10 minutes");
            }

            tracing::trace!("Returning success");
            Ok("Joined voice channel".into())
        } else {
            Err(anyhow!("Could not join voice channel."))
        }
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
