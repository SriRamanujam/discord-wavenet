use std::{
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::anyhow;
use anyhow::Context as anyhowContext;
use googapis::google::cloud::texttospeech::v1::{
    synthesis_input::InputSource, text_to_speech_client::TextToSpeechClient, AudioConfig,
    AudioEncoding, ListVoicesRequest, SsmlVoiceGender, SynthesisInput, SynthesizeSpeechRequest,
    VoiceSelectionParams,
};
use gouth::Builder;
use serenity::{
    async_trait,
    client::{Context, EventHandler},
    framework::{
        standard::{
            macros::{command, group},
            Args, CommandResult,
        },
        StandardFramework,
    },
    model::{
        channel::Message,
        id::{ChannelId, GuildId},
        prelude::Ready,
    },
    prelude::TypeMapKey,
    Client,
};
use songbird::{
    create_player, Event, EventContext, EventHandler as VoiceEventHandler, SerenityInit, TrackEvent,
};
use tonic::{
    metadata::MetadataValue,
    transport::{Certificate, Channel, ClientTlsConfig},
    Request,
};

const NOT_IN_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in a voice channel!";
const NOT_IN_SAME_VOICE_CHANNEL_MESSAGE: &str =
    "Can't tell me what to do if you're not in the same voice channel!";

#[tracing::instrument(skip(api_path), err)]
async fn create_google_api_client<C: AsRef<Path> + std::fmt::Debug>(
    api_path: C,
) -> anyhow::Result<TextToSpeechClient<Channel>> {
    tracing::debug!("Loading Google API credentials from {:?}", api_path);

    let token = Builder::new()
        .file(api_path.as_ref())
        .build()
        .context("Could not load Google API credentials")?;

    let tls_config = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(googapis::CERTIFICATES))
        .domain_name("texttospeech.googleapis.com");

    let channel = Channel::from_static("https://texttospeech.googleapis.com")
        .tls_config(tls_config)?
        .connect()
        .await
        .context("Could not connect to Google TTS API")?;

    // TODO: I don't understand how these autogenerated clients work. Is the interceptor really necessary to inject the bearer token?
    let service = TextToSpeechClient::with_interceptor(channel, move |mut req: Request<()>| {
        let token = &*token.header_value().map_err(|_| {
            tonic::Status::new(
                tonic::Code::Internal,
                "Could not get token authorization header value",
            )
        })?;
        let meta = MetadataValue::from_str(token)
            .map_err(|_| tonic::Status::new(tonic::Code::Internal, "Invalid metadata value"))?;
        req.metadata_mut().insert("authorization", meta);
        Ok(req)
    });

    Ok(service)
}

#[tracing::instrument(skip(svc))]
async fn get_voices(svc: &mut TextToSpeechClient<Channel>) -> anyhow::Result<Vec<String>> {
    tracing::debug!("Fetching list of Wavenet voices from Google...");
    let req = ListVoicesRequest {
        language_code: "en-US".to_string(),
    };

    let res = svc.list_voices(req).await?.into_inner();

    let voices = res
        .voices
        .into_iter()
        .filter_map(|v| {
            if v.name.contains("Wavenet") {
                Some(v.name)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    tracing::info!("Loading bot with {} voices", voices.len());

    Ok(voices)
}

#[derive(Default)]
struct ChannelDurationNotifier {
    count: Arc<AtomicUsize>,
}

#[async_trait]
impl VoiceEventHandler for ChannelDurationNotifier {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        // TODO: automatically leave channel after some amount of time spent idle
        let count_before = self.count.fetch_add(1, Ordering::Relaxed);
        tracing::info!("Been in voice channel for {} minutes!", count_before);
        None
    }
}

#[command]
#[only_in(guilds)]
async fn join(ctx: &Context, msg: &Message) -> CommandResult {
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
async fn do_join(
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
async fn leave(ctx: &Context, msg: &Message) -> CommandResult {
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
async fn say(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
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
            do_join(ctx, msg, channel_id, guild_id).await?;
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
async fn skip(ctx: &Context, msg: &Message) -> CommandResult {
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

// TODO: what is this
struct ReadyNotifier;

#[async_trait]
impl EventHandler for ReadyNotifier {
    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("{} is connected!", ready.user.name);
    }
}

struct CurrentVoiceChannel;

impl TypeMapKey for CurrentVoiceChannel {
    type Value = ChannelId;
}

struct TtsService;

impl TypeMapKey for TtsService {
    type Value = TextToSpeechClient<Channel>;
}

struct Voices;
impl TypeMapKey for Voices {
    type Value = Vec<String>;
}

#[group]
#[commands(say, join, leave, skip)]
struct General;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let discord_token =
        std::env::var("DISCORD_TOKEN").context("Could not find env var DISCORD_TOKEN")?;
    let api_path = std::env::var("GOOGLE_API_CREDENTIALS")
        .context("Could not find env var GOOGLE_API_CREDENTIALS")?;

    let mut service = create_google_api_client(api_path).await?;

    let framework = StandardFramework::new()
        .configure(|c| c.prefix("::"))
        .group(&GENERAL_GROUP);

    let mut client = Client::builder(&discord_token)
        .event_handler(ReadyNotifier)
        .framework(framework)
        .register_songbird()
        .await
        .context("Could not initialize Discord client")?;

    let voices = get_voices(&mut service).await?;

    {
        let mut data = client.data.write().await;
        data.insert::<TtsService>(service);
        data.insert::<Voices>(voices);
    }

    let _ = client.start().await.map_err(|why| {
        tracing::info!("Client ended: {:?}", why);
    });

    Ok(())
}
