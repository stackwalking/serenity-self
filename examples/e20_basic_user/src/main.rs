use base64::Engine;
use {
    std::{
        env,
        sync::Arc,
    },
    serenity:: {
        async_trait, Error,
        prelude::*,
        all::GuildId,
        model:: {
            channel::Message,
            gateway:: {
                Ready,
                GatewayIntents
            }
        },
        http::CaptchaRequiredData,
    },
    twocaptcha::{
        TwoCaptcha, TwoCaptchaConfig
    }
};

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, _ctx: Context, guilds: Vec<GuildId>) {
        tracing::info!("{} guilds in cache!", guilds.len());
    }
    async fn shards_ready(&self, _ctx: Context, total_shards: u32) {
        tracing::info!("{} shards are ready!", total_shards);
    }
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.guild_id.is_none() {
            println!("{}: {}", msg.author.name, msg.content);
        } else {
            println!("[{:?}] {}: {}", msg.guild(&ctx.cache).map(|g| g.name.to_string()).unwrap_or_default(), msg.author.name, msg.content);
        }
    }
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("{} is connected!", base64::engine::general_purpose::STANDARD.encode(ready.user.id.get().to_le_bytes()));
    }
}

async fn solve_captcha(data: CaptchaRequiredData) -> Result<String, Error> {
    tracing::info!("CAPTCHA challenge received!");
    tracing::info!("  Service: {:?}", data.service());
    tracing::info!("  Site key: {}", data.sitekey());

    let api_key = env::var("TWO_CAPTCHA_KEY").expect("Expected a api key in the environment");
    let solver = TwoCaptcha::new(api_key, TwoCaptchaConfig::default());

    let result = solver.hcaptcha(
        data.sitekey(),
        "https://discord.com/api/v10/users/@me",
        None
    ).await.expect("Failed to solve captcha");

    tracing::info!("hCaptcha solved: {}", result.code.clone().unwrap_or_default());
    Ok(result.code.unwrap_or_default())
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().expect("Failed to load .env file");
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    /*let captcha_handler: CaptchaHandler = Arc::new(|data| {
        Box::pin(solve_captcha(data))
    });*/

    let mut client =
        Client::builder(&token, intents)
            // .captcha_handler(Arc::new(|data| {
            //     Box::pin(solve_captcha(data))
            // }))
            .event_handler(Handler).await.expect("Err creating client");

    if let Err(why) = client.start().await {
        tracing::error!("Client error: {why:?}");
    }
}
