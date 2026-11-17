use crate::config::Invitation;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Status {
    pub ready: bool,
    #[serde(default)]
    pub services: BTreeMap<String, bool>,
    #[serde(default, rename = "acceptedBuilds")]
    pub accepted_builds: Vec<String>,
}
pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("WoW-Friends-Launcher/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
pub async fn status(client: &reqwest::Client, i: &Invitation) -> Result<Status> {
    let r = client
        .get(format!("{}/status", i.origin()))
        .bearer_auth(&i.token)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Cannot reach the private server. Retrying automatically."))?;
    if r.status() == 401 {
        bail!("Invitation is invalid or revoked. Contact the server owner.");
    }
    if !r.status().is_success() {
        bail!(
            "Server status is unavailable (HTTP {}).",
            r.status().as_u16()
        );
    }
    let s: Status = r.json().await?;
    if !s
        .accepted_builds
        .iter()
        .all(|s| crate::builds::valid_build(s))
    {
        bail!("Server returned invalid compatibility information");
    }
    Ok(s)
}
