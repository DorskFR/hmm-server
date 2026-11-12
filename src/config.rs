use std::sync::Arc;

use clap::Parser;
use object_store::ObjectStore;
use object_store::aws::AmazonS3Builder;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Config {
    #[arg(long, env = "HMM_BIND", default_value = "0.0.0.0:8080")]
    pub bind: String,
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,
    #[arg(long, env = "HMM_DB_CONNECTIONS", default_value_t = 5)]
    pub db_connections: u32,
    #[arg(long, env = "HMM_S3_BUCKET")]
    pub s3_bucket: String,
    /// Set for an S3-compatible store; requests then use path-style addressing.
    #[arg(long, env = "HMM_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,
    #[arg(long, env = "HMM_S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,
    #[arg(long, env = "AWS_ACCESS_KEY_ID")]
    pub s3_access_key_id: String,
    #[arg(long, env = "AWS_SECRET_ACCESS_KEY", hide_env_values = true)]
    pub s3_secret_access_key: String,
    /// JSON policy; see the README. Unset accepts every report.
    #[arg(long, env = "HMM_POLICY")]
    pub policy: Option<String>,
    #[arg(long, env = "HMM_MAX_BODY_BYTES", default_value_t = 32 * 1024 * 1024)]
    pub max_body_bytes: usize,
}

impl Config {
    pub fn blobs(&self) -> object_store::Result<Arc<dyn ObjectStore>> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&self.s3_bucket)
            .with_region(&self.s3_region)
            .with_access_key_id(&self.s3_access_key_id)
            .with_secret_access_key(&self.s3_secret_access_key);
        if let Some(endpoint) = &self.s3_endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(true)
                .with_virtual_hosted_style_request(false);
        }
        Ok(Arc::new(builder.build()?))
    }
}
