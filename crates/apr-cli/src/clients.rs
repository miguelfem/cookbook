use apr_runtime::{Runtime, RuntimeError, oauth_clients::ClientIdentity};
use clap::Subcommand;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum ClientCommand {
    /// Prepare public static hosting files for review; does not publish anything.
    PrepareHosting {
        #[arg(long)]
        client_id_url: String,
        #[arg(long)]
        directory: PathBuf,
    },
    /// Inspect public client identity only; never reads account credentials.
    Show,
    /// Install reviewed public identity metadata. Does not authenticate or grant access.
    Configure {
        #[arg(long)]
        file: PathBuf,
    },
}

pub fn execute(state: &Path, command: ClientCommand) -> apr_runtime::Result<Value> {
    if let ClientCommand::PrepareHosting {
        client_id_url,
        directory,
    } = &command
    {
        return prepare_hosting(client_id_url, directory);
    }
    let runtime = Runtime::open(state)?;
    match command {
        ClientCommand::PrepareHosting { .. } => unreachable!(),
        ClientCommand::Show => Ok(serde_json::to_value(runtime.oauth_client_identity()?)?),
        ClientCommand::Configure { file } => {
            let mut bytes = Vec::new();
            std::fs::File::open(file)?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 16 * 1024 {
                return Err(RuntimeError::InvalidInput(
                    "client identity exceeds limit".into(),
                ));
            }
            let identity: ClientIdentity = serde_json::from_slice(&bytes)?;
            runtime.set_oauth_client_identity(&identity)?;
            Ok(
                json!({"configured":true,"identity":identity,"account_connected":false,"grant_created":false}),
            )
        }
    }
}

fn prepare_hosting(client_id_url: &str, directory: &Path) -> apr_runtime::Result<Value> {
    let identity = ClientIdentity {
        client_id_metadata_url: Some(client_id_url.into()),
        pre_registered_clients: Vec::new(),
    };
    identity.validate()?;
    let url = apr_connectors::oauth::public_url(client_id_url, false)
        .map_err(|_| RuntimeError::InvalidInput("public HTTPS client URL required".into()))?;
    if !directory.is_absolute() || directory.exists() || url.path() != "/client.json" {
        return Err(RuntimeError::InvalidInput(
            "use a new absolute directory and a /client.json HTTPS URL".into(),
        ));
    }
    let metadata = json!({
        "client_id":client_id_url,"client_name":"CookBook APR",
        "redirect_uris":[
            "http://127.0.0.1:48557/oauth/callback",
            "http://127.0.0.1:48558/oauth/callback",
            "http://127.0.0.1:48559/oauth/callback",
            "http://127.0.0.1:48560/oauth/callback"
        ],
        "grant_types":["authorization_code","refresh_token"],
        "response_types":["code"],"token_endpoint_auth_method":"none","application_type":"native"
    });
    std::fs::create_dir(directory)?;
    let public = directory.join("public");
    std::fs::create_dir(&public)?;
    std::fs::write(
        public.join("client.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    std::fs::write(
        directory.join("client-identity.json"),
        serde_json::to_vec_pretty(&identity)?,
    )?;
    std::fs::write(
        directory.join("README.md"),
        "# CookBook public identity\n\nPublish only the public directory at the reviewed HTTPS host. Serve client.json as application/json, with no authentication or redirects. Keep its address stable. It contains public application metadata, no credentials. Verify the live document before configuring CookBook with the sibling client-identity.json. Hosting publication and DNS changes require owner approval.\n",
    )?;
    Ok(
        json!({"prepared":true,"directory":directory,"public_directory":public,"client_id":client_id_url,"published":false,"configured":false,"grant_created":false}),
    )
}
