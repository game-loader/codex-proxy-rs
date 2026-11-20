//! 启动插件进程并绑定管理会话

mod archive;
mod calendar;
mod management;
mod ranking;

use chrono_tz::Tz;
use gateway_plugin_sdk::{
    Manifest,
    client::{PluginSession, SessionConfig, SessionError},
};

const AUTHOR_MANIFEST: &[u8] = include_bytes!("../../plugin.json");

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::from_author_slice(AUTHOR_MANIFEST)?;
    let session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig::default(),
    )
    .await?;
    let handshake = session.handshake();
    if handshake.plugin_id != manifest.plugin_id()? || handshake.contributes != manifest.contributes
    {
        return Err(SessionError::Handshake.into());
    }
    let plugin = management::plugin(timezone(&handshake.configuration)?)?;
    session.run(plugin).await?;
    Ok(())
}

fn timezone(configuration: &serde_json::Value) -> Result<Tz, &'static str> {
    let configuration = configuration.as_object().ok_or("插件配置必须是对象")?;
    match configuration.get("timezone") {
        None => Ok(chrono_tz::Asia::Shanghai),
        Some(value) => value
            .as_str()
            .and_then(|name| name.parse().ok())
            .ok_or("timezone 必须是有效的 IANA 时区"),
    }
}
