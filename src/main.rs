use std::{collections::HashMap, str::FromStr};

use anyhow::Context as anyhowContext;

use commands::CommandScope;
use google_texttospeech1::Texttospeech;
use serenity::{
    async_trait,
    client::{Context, EventHandler},
    framework::{standard::macros::group, StandardFramework},
    model::prelude::Ready,
    Client,
};
use songbird::SerenityInit;
use tracing_subscriber::EnvFilter;

mod commands;

use commands::{join::*, leave::*, say::*, skip::*, ApplicationCommandHandler, IdleDurations};

use crate::commands::CommandsMap;

#[tracing::instrument(skip(hub))]
async fn get_voices(hub: &Texttospeech) -> anyhow::Result<HashMap<String, Vec<String>>> {
    let (_, response) = hub
        .voices()
        .list()
        .doit()
        .await
        .context("Could not make list voices request!")?;

    let mut x = HashMap::new();
    let mut counter = 0;

    for v in response.voices.into_iter().flatten() {
        let name = v.name.expect("Should have been a name here");
        if name.contains("Wavenet") {
            let language_codes = v
                .language_codes
                .expect("Should have been a language code here");
            for code in language_codes {
                x.entry(code).or_insert_with(Vec::new).push(name.clone());
                counter += 1;
            }
        }
    }

    tracing::info!("Loaded {} Wavenet voices", counter);

    Ok(x)
}

struct ReadyNotifier;
#[async_trait]
impl EventHandler for ReadyNotifier {
    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("{} is connected!", ready.user.name);
    }
}

#[group]
#[commands(say, join, leave, skip)]
struct General;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to start the logger");

    let discord_token =
        std::env::var("DISCORD_TOKEN").context("Could not find env var DISCORD_TOKEN")?;
    let application_id = std::env::var("DISCORD_APPLICATION_ID")
        .context("Could not find env var DISCORD_APPLICATION_ID")?
        .parse::<u64>()
        .context("Invalid application id")?;
    let api_path = std::env::var("GOOGLE_API_CREDENTIALS")
        .context("Could not find env var GOOGLE_API_CREDENTIALS")?;
    let prefix = std::env::var("PREFIX").unwrap_or_else(|_| "::".to_owned());

    let app_command_prefix = std::env::var("APPLICATION_COMMAND_PREFIX")
        .context("Must provide an application command prefix for slash commands.")?;
    let app_command_scope = CommandScope::from_str(
        &std::env::var("APPLICATION_COMMAND_SCOPE").unwrap_or_else(|_| "global".into()),
    )?;

    let secret = yup_oauth2::read_service_account_key(&api_path)
        .await
        .context("Could not read application secret from file!")?;

    let auth = yup_oauth2::ServiceAccountAuthenticator::builder(secret)
        .build()
        .await
        .context("Could not create authenticator!")?;

    let hub = google_texttospeech1::Texttospeech::new(
        hyper::Client::builder().build(hyper_rustls::HttpsConnector::with_native_roots()),
        auth,
    );

    let voices = get_voices(&hub).await?;

    let framework = StandardFramework::new()
        .configure(|c| c.prefix(&prefix))
        .group(&GENERAL_GROUP);

    let mut client = Client::builder(&discord_token)
        .event_handler(ReadyNotifier)
        .event_handler(ApplicationCommandHandler {
            prefix: app_command_prefix,
            scope: app_command_scope,
        })
        .framework(framework)
        .application_id(application_id)
        .register_songbird()
        .await
        .context("Could not initialize Discord client")?;

    {
        let mut data = client.data.write().await;
        data.insert::<TtsService>(hub);
        data.insert::<Voices>(voices);
        data.insert::<IdleDurations>(HashMap::new());
        data.insert::<CommandsMap>(commands::register_commands());
    }

    let _ = client.start().await.map_err(|why| {
        tracing::info!("Client ended: {:?}", why);
    });

    Ok(())
}
