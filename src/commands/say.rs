use std::io::Write;

use anyhow::anyhow;
use googapis::google::cloud::texttospeech::v1::{
    synthesis_input::InputSource, text_to_speech_client::TextToSpeechClient, AudioConfig,
    AudioEncoding, SsmlVoiceGender, SynthesisInput, SynthesizeSpeechRequest, VoiceSelectionParams,
};
use serenity::{
    async_trait,
    client::Context,
    framework::standard::{macros::command, Args, CommandResult},
    model::channel::Message,
    prelude::TypeMapKey,
};
use songbird::events::EventHandler as VoiceEventHandler;
use songbird::{create_player, Event, EventContext, TrackEvent};
use tonic::transport::Channel;

use crate::commands::{
    CurrentVoiceChannel, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE, NOT_IN_VOICE_CHANNEL_MESSAGE,
};

pub struct TtsService;

impl TypeMapKey for TtsService {
    type Value = TextToSpeechClient<Channel>;
}

pub struct Voices;
impl TypeMapKey for Voices {
    type Value = Vec<String>;
}

struct TrackCleanup(tempfile::NamedTempFile);

#[async_trait]
impl VoiceEventHandler for TrackCleanup {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        // This event is just around so that the tempfile will get destructed
        // after the track has been played and not while it's in the queue.
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

    let in_voice_channel = { ctx.data.read().await.get::<CurrentVoiceChannel>().copied() };

    // If we're not in a voice channel currently, try to join the one
    // the user's in. If we are in one, check to make sure it's the same one
    // as the user. Bail if it's not.
    match in_voice_channel {
        Some(c) => {
            if c != channel_id {
                msg.reply(ctx, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE).await?;
                return Ok(());
            } else {
                tracing::debug!(
                    ?guild_id,
                    "Already in same voice channel as user, continuing..."
                );
            }
        }
        None => {
            crate::commands::join::do_join(ctx, msg, channel_id, guild_id).await?;
        }
    }

    let manager = songbird::get(ctx)
        .await
        .expect("Songbird context should be there")
        .clone();

    let res = {
        let mut data = ctx.data.write().await;
        let voices = data
            .get::<Voices>()
            .expect("There should have been voices here.");
        let voice = voices[fastrand::usize(..voices.len())].clone();

        let tts_service = data
            .get_mut::<TtsService>()
            .expect("There should have been a TTS service here.");
        let req = SynthesizeSpeechRequest {
            input: Some(SynthesisInput {
                input_source: Some(InputSource::Ssml(format!(
                    "<speak>{}</speak>",
                    args.message().to_string()
                ))),
            }),
            voice: Some(VoiceSelectionParams {
                language_code: "en-US".to_string(),
                name: voice,
                ssml_gender: SsmlVoiceGender::Unspecified as i32,
            }),
            audio_config: Some(AudioConfig {
                audio_encoding: AudioEncoding::Linear16 as i32,
                speaking_rate: 0.0,
                pitch: 0.0,
                volume_gain_db: 0.0,
                sample_rate_hertz: 0,
                effects_profile_id: vec![],
            }),
        };

        let res = tts_service
            .synthesize_speech(req)
            .await
            .map_err(|s| anyhow!("Could not make TTS API call: {}", s.message()))?;

        res
    };

    if let Some(handler_lock) = manager.get(guild_id) {
        let response = res.into_inner();

        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(&response.audio_content)?;

        let input = songbird::ffmpeg(file.path())
            .await
            .map_err(|_| anyhow!("Could not create ffmpeg player"))?;

        let (track, track_handle) = create_player(input);

        track_handle.add_event(Event::Track(TrackEvent::End), TrackCleanup(file))?;

        {
            let mut handler = handler_lock.lock().await;
            handler.enqueue(track);
        }
    } else {
        msg.reply(ctx, "Not in a voice channel right now.").await?;
    }

    Ok(())
}

#[command]
#[only_in(guilds)]
pub async fn skip(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg
        .guild(&ctx.cache)
        .await
        .ok_or_else(|| anyhow!("Could not fetch guild"))?;

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
        .expect("Songbird context should be in there")
        .clone();
    if let Some(handler_lock) = manager.get(guild.id) {
        handler_lock.lock().await.queue().skip()?;
    } else {
        msg.reply(ctx, "Not in a voice channel right now.").await?;
    }

    Ok(())
}
