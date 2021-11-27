use std::{
    collections::HashMap,
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, Context as anyhowContext};
use google_texttospeech1::{
    api::{AudioConfig, SynthesisInput, SynthesizeSpeechRequest, VoiceSelectionParams},
    Texttospeech,
};
use serenity::{
    async_trait,
    builder::CreateApplicationCommandOption,
    client::Context,
    framework::standard::{macros::command, Args, CommandResult},
    model::{channel::Message, interactions::application_command::ApplicationCommandOptionType},
    prelude::TypeMapKey,
};
use songbird::{create_player, Event, EventContext, TrackEvent};
use songbird::{events::EventHandler as VoiceEventHandler, id::GuildId};

use crate::commands::{
    get_songbird_from_ctx, get_voice_channel_id, IdleDurations, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE,
    NOT_IN_VOICE_CHANNEL_MESSAGE,
};

pub struct TtsService;
impl TypeMapKey for TtsService {
    type Value = Texttospeech;
}

pub struct Voices;
impl TypeMapKey for Voices {
    type Value = HashMap<String, Vec<String>>;
}

struct TrackCleanup {
    idle_tracking: Arc<AtomicUsize>,
    /// Unused, we want to tie the lifetimes together so that the temp file is cleaned up.
    _tmpfile: tempfile::NamedTempFile,
}

#[async_trait]
impl VoiceEventHandler for TrackCleanup {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        // reset the idle tracker for this guild.
        self.idle_tracking.store(0, Ordering::SeqCst);
        None
    }
}

#[command]
#[only_in(guilds)]
pub async fn say(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not fetch guild"))?;
    let guild_id = GuildId::from(guild.id);

    let channel_id = match get_voice_channel_id(&guild, msg) {
        Some(c) => c,
        None => {
            msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
            return Ok(());
        }
    };

    let manager = get_songbird_from_ctx(ctx).await;

    // if we're not in a voice channel for this guild, join the channel.
    // if we're in another voice channel in the same guild, deny the say with a message.
    match manager.get(guild_id) {
        Some(s) => {
            let r = s.lock().await;
            let c = r.current_channel().expect("there should be a channel here");
            if c != channel_id {
                msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
                return Ok(());
            }
        }
        None => {
            // we are not in a voice channel for this guild, join one.
            crate::commands::join::do_join(ctx, msg, channel_id, guild_id).await?;
        }
    }

    let res = {
        let data = ctx.data.read().await;
        let voices = data
            .get::<Voices>()
            .expect("There should have been voices here.")
            .get("en-US") // TODO: change this to be dynamic!
            .context("No voices found for this language code!")?;
        let voice = voices[fastrand::usize(..voices.len())].clone();

        let tts_service = data
            .get::<TtsService>()
            .expect("There should have been a TTS service here.");

        let req = SynthesizeSpeechRequest {
            audio_config: Some(AudioConfig {
                audio_encoding: Some("LINEAR16".to_string()),
                effects_profile_id: None,
                pitch: Some(0.0),
                sample_rate_hertz: None,
                speaking_rate: None,
                volume_gain_db: None,
            }),
            input: Some(SynthesisInput {
                ssml: Some(format!("<speak>{}</speak>", args.message())),
                text: None,
            }),
            voice: Some(VoiceSelectionParams {
                language_code: Some("en-US".to_string()),
                name: Some(voice),
                ssml_gender: None,
            }),
        };

        let (_, res) = tts_service
            .text()
            .synthesize(req)
            .doit()
            .await
            .context("Could not make TTS API call")?;

        match res.audio_content {
            Some(c) => base64::decode(c).context("Could not decode base64 audio content!")?,
            None => return Err(anyhow!("No audio content returned from API!").into()),
        }
    };

    if let Some(handler_lock) = manager.get(guild_id) {
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(&res)?;

        let input = songbird::ffmpeg(file.path())
            .await
            .map_err(|_| anyhow!("Could not create ffmpeg player"))?;

        let (track, track_handle) = create_player(input);

        let maybe_idle_tracking = {
            ctx.data
                .read()
                .await
                .get::<IdleDurations>()
                .expect("idle duration should be present")
                .get(&guild_id)
                .cloned()
        };

        match maybe_idle_tracking {
            Some(c) => {
                track_handle.add_event(
                    Event::Track(TrackEvent::End),
                    TrackCleanup {
                        _tmpfile: file,
                        idle_tracking: c,
                    },
                )?;
            }
            None => {
                file.close()?;
                msg.reply(
                    ctx,
                    "Unexpected error. Please contact bot admin and tell them \"Blue Rhinoceros\"",
                )
                .await?;
                return Ok(());
            }
        }

        {
            let mut handler = handler_lock.lock().await;
            handler.enqueue(track);
        }
    } else {
        msg.reply(ctx, "Not in a voice channel right now.").await?;
    }

    Ok(())
}

pub fn create_command() -> CreateApplicationCommandOption {
    CreateApplicationCommandOption::default()
        .name("say")
        .description("Say something into the voice channel you are currently in")
        .kind(ApplicationCommandOptionType::SubCommand)
        .create_sub_option(|o| {
            o.name("message")
                .description("What you want the bot to say")
                .kind(ApplicationCommandOptionType::String)
                .required(true)
        })
        .create_sub_option(|o| {
            o.name("language")
                .description("A language to use (default en-US). You can get the list of languages with `/tugboat languages`")
                .kind(ApplicationCommandOptionType::String)
                .required(false)
        })
        .clone()
}

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
