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
    api::{AudioConfig, SynthesisInput, SynthesizeSpeechRequest, Voice, VoiceSelectionParams},
    Texttospeech,
};
use serde_json::Value;
use serenity::model::guild::Guild;
use serenity::{
    async_trait,
    builder::CreateApplicationCommandOption,
    client::Context,
    model::interactions::application_command::{
        ApplicationCommandInteractionDataOption, ApplicationCommandOptionType,
    },
    prelude::TypeMapKey,
};
use songbird::{create_player, id::ChannelId, Event, EventContext, TrackEvent};
use songbird::{events::EventHandler as VoiceEventHandler, id::GuildId};

use crate::commands::{get_songbird_from_ctx, IdleDurations, NOT_IN_SAME_VOICE_CHANNEL_MESSAGE};

use super::{CommandsMap, TugboatCommand};

pub struct TtsService;
impl TypeMapKey for TtsService {
    type Value = Texttospeech;
}

pub struct Voices;
pub type VoiceValues = HashMap<String, Vec<Voice>>;
impl TypeMapKey for Voices {
    type Value = VoiceValues;
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

pub struct SayCommand;

#[async_trait]
impl TugboatCommand for SayCommand {
    fn get_name(&self) -> String {
        String::from("say")
    }

    async fn execute(
        &self,
        ctx: &Context,
        options: &[ApplicationCommandInteractionDataOption],
        guild: Guild,
        channel_id: ChannelId,
    ) -> anyhow::Result<String> {
        let manager = get_songbird_from_ctx(ctx).await;
        // if we're not in a voice channel for this guild, join the channel.
        // if we're in another voice channel in the same guild, deny the say with a message.
        match manager.get(guild.id) {
            Some(s) => {
                let r = s.lock().await;
                let c = r.current_channel().expect("there should be a channel here");
                if c != channel_id {
                    return Ok(NOT_IN_SAME_VOICE_CHANNEL_MESSAGE.into());
                }
            }
            None => {
                // we are not in a voice channel for this guild, join one.
                let join_command = {
                    let data = ctx.data.read().await;
                    data.get::<CommandsMap>()
                        .expect("Should have been commands here")
                        .get("join")
                        .expect("There should always be a join command")
                        .clone()
                };

                join_command
                    .execute(ctx, options, guild.clone(), channel_id)
                    .await?;
            }
        }

        let (message, language, gender) = {
            let mut m = None;
            let mut l = None;
            let mut g = None;
            for option in options {
                match option.name.as_str() {
                    "message" => {
                        m = option
                            .value
                            .as_ref()
                            .map(|v| match v {
                                Value::String(s) => Some(s.to_owned()),
                                _ => None,
                            })
                            .flatten();
                    }
                    "language" => {
                        l = option
                            .value
                            .as_ref()
                            .map(|v| match v {
                                Value::String(s) => Some(s.to_owned()),
                                _ => None,
                            })
                            .flatten();
                    }
                    "gender" => {
                        g = option
                            .value
                            .as_ref()
                            .map(|v| match v {
                                Value::String(s) => Some(s.to_owned()),
                                _ => None,
                            })
                            .flatten();
                    }
                    _ => continue,
                }
            }

            (m, l, g)
        };

        let message = match message {
            Some(m) => {
                if m.is_empty() {
                    return Ok("Must supply a string with at least one character".into());
                } else {
                    m
                }
            }
            None => return Ok("Must supply a string with at least one character".into()),
        };

        let language_code = language.unwrap_or_else(|| "en-US".to_owned());

        let res = {
            let data = ctx.data.read().await;
            let voices = data
                .get::<Voices>()
                .expect("There should have been voices here.")
                .get(&language_code)
                .context("No voices found for this language code!")?
                .into_iter()
                .filter_map(|v| match gender {
                    // if the gender is present, only filter out voices that
                    // have that same gender. otherwise, return all voices.
                    Some(ref g) => {
                        if g == v
                            .ssml_gender
                            .as_ref()
                            .expect("Should have been a gender here")
                            .as_str()
                        {
                            Some(
                                v.name
                                    .as_ref()
                                    .expect("Should have been a name here")
                                    .clone(),
                            )
                        } else {
                            None
                        }
                    }
                    None => Some(
                        v.name
                            .as_ref()
                            .expect("Should have been a name here")
                            .clone(),
                    ),
                })
                .collect::<Vec<_>>();

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
                    ssml: Some(format!("<speak>{}</speak>", message)),
                    text: None,
                }),
                voice: Some(VoiceSelectionParams {
                    language_code: Some(language_code),
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
                None => return Err(anyhow!("No audio content returned from API!")),
            }
        };

        if let Some(handler_lock) = manager.get(guild.id) {
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
                    .get(&GuildId::from(guild.id))
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
                    return Err(anyhow!(
                    "Unexpected error. Please contact bot admin and tell them \"Blue Rhinoceros\""
                        ));
                }
            }

            {
                let mut handler = handler_lock.lock().await;
                handler.enqueue(track);
            }
        } else {
            return Ok("Not in a voice channel right now.".into());
        }

        Ok(message)
    }

    fn create_command(&self) -> CreateApplicationCommandOption {
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
        .create_sub_option(|o| {
            o.name("gender")
                .description("The gender of the generated speech. By default will pick randomly.")
                .kind(ApplicationCommandOptionType::String)
                .required(false)
                .add_string_choice("Male", "MALE")
                .add_string_choice("Female", "FEMALE")
        })
        .clone()
    }
}
