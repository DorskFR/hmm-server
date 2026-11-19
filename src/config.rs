use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[command(flatten)]
    pub config: Config,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server; the default without a subcommand.
    Serve,
    /// Manage API keys; needs only DATABASE_URL.
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Create a key and print its token once.
    Create {
        #[arg(long)]
        name: String,
        /// Comma-separated; `*` for every project.
        #[arg(long, value_delimiter = ',', required = true)]
        projects: Vec<String>,
        /// Comma-separated: reports:read, attachments:read, reports:triage, admin.
        #[arg(long, value_delimiter = ',', required = true)]
        scopes: Vec<String>,
        #[arg(long)]
        expires_in_days: Option<u32>,
    },
    Ls,
    Revoke {
        prefix: String,
    },
}

#[derive(Debug, Args)]
pub struct Config {
    #[arg(long, env = "HMM_BIND", default_value = "0.0.0.0:8080", global = true)]
    pub bind: String,
    #[arg(long, env = "HMM_ADMIN_BIND", default_value = "0.0.0.0:8081", global = true)]
    pub admin_bind: String,
    /// Directory of the built web UI, served on the admin listener.
    #[arg(long, env = "HMM_UI_DIR", default_value = "/usr/share/hmm-server/ui", global = true)]
    pub ui_dir: PathBuf,
    #[arg(long, env = "DATABASE_URL", global = true)]
    pub database_url: Option<String>,
    #[arg(long, env = "HMM_DB_CONNECTIONS", default_value_t = 5, global = true)]
    pub db_connections: u32,
    #[arg(long, env = "HMM_S3_BUCKET", global = true)]
    pub s3_bucket: Option<String>,
    /// Set for an S3-compatible store; requests then use path-style addressing.
    #[arg(long, env = "HMM_S3_ENDPOINT", global = true)]
    pub s3_endpoint: Option<String>,
    #[arg(long, env = "HMM_S3_REGION", default_value = "us-east-1", global = true)]
    pub s3_region: String,
    #[arg(long, env = "AWS_ACCESS_KEY_ID", global = true)]
    pub s3_access_key_id: Option<String>,
    #[arg(long, env = "AWS_SECRET_ACCESS_KEY", hide_env_values = true, global = true)]
    pub s3_secret_access_key: Option<String>,
    /// JSON policy; see the README. Unset accepts every report.
    #[arg(long, env = "HMM_POLICY", global = true)]
    pub policy: Option<String>,
    #[arg(long, env = "HMM_MAX_BODY_BYTES", default_value_t = 32 * 1024 * 1024, global = true)]
    pub max_body_bytes: usize,
    /// Comma-separated webhook hosts that may use http and resolve to private addresses.
    #[arg(long, env = "HMM_WEBHOOK_ALLOW_HOSTS", value_delimiter = ',', global = true)]
    pub webhook_allow_hosts: Vec<String>,
}

impl Config {
    pub fn database_url(&self) -> anyhow::Result<&str> {
        self.database_url.as_deref().context("DATABASE_URL is required")
    }

    pub fn blobs(&self) -> anyhow::Result<Arc<dyn ObjectStore>> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(self.s3_bucket.as_deref().context("HMM_S3_BUCKET is required")?)
            .with_region(&self.s3_region)
            .with_access_key_id(
                self.s3_access_key_id.as_deref().context("AWS_ACCESS_KEY_ID is required")?,
            )
            .with_secret_access_key(
                self.s3_secret_access_key
                    .as_deref()
                    .context("AWS_SECRET_ACCESS_KEY is required")?,
            );
        if let Some(endpoint) = &self.s3_endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(true)
                .with_virtual_hosted_style_request(false);
        }
        Ok(Arc::new(builder.build()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_subcommand_serves_and_key_commands_need_no_storage_settings() {
        let cli = Cli::try_parse_from(["hmm-server"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(
            (cli.config.bind.as_str(), cli.config.admin_bind.as_str()),
            ("0.0.0.0:8080", "0.0.0.0:8081")
        );
        assert_eq!(cli.config.ui_dir, PathBuf::from("/usr/share/hmm-server/ui"));

        let cli =
            Cli::try_parse_from(["hmm-server", "serve", "--webhook-allow-hosts", "a.svc,b.svc"])
                .unwrap();
        assert!(matches!(cli.command, Some(Command::Serve)));
        assert_eq!(cli.config.webhook_allow_hosts, ["a.svc", "b.svc"]);

        let cli = Cli::try_parse_from([
            "hmm-server",
            "key",
            "create",
            "--name",
            "cctui-agent",
            "--projects",
            "cctui",
            "--scopes",
            "reports:read,attachments:read",
            "--expires-in-days",
            "90",
            "--database-url",
            "postgres://db/hmm",
        ])
        .unwrap();
        assert_eq!(cli.config.database_url().unwrap(), "postgres://db/hmm");
        let Some(Command::Key {
            command: KeyCommand::Create { name, projects, scopes, expires_in_days },
        }) = cli.command
        else {
            panic!("not a key create command");
        };
        assert_eq!((name.as_str(), expires_in_days), ("cctui-agent", Some(90)));
        assert_eq!(projects, ["cctui"]);
        assert_eq!(scopes, ["reports:read", "attachments:read"]);

        assert!(matches!(
            Cli::try_parse_from(["hmm-server", "key", "revoke", "k7Qp2xAb"]).unwrap().command,
            Some(Command::Key { command: KeyCommand::Revoke { .. } })
        ));
        assert!(Cli::try_parse_from(["hmm-server", "key", "create", "--name", "x"]).is_err());
    }
}
