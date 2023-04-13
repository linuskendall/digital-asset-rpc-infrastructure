use crate::{
    tasks::{FromTaskData, IntoTaskData},
    BgTask, IngesterError, TaskData,
    metric
};
use async_trait::async_trait;
use chrono::NaiveDateTime;
use digital_asset_types::dao::asset_data;
use reqwest::{Client, ClientBuilder};
use sea_orm::*;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Display, Formatter},
    time::Duration,
};
use url::Url;
use cadence_macros::is_global_default_set;
use cadence_macros::{set_global_default, statsd_count, statsd_gauge, statsd_time};

const TASK_NAME: &str = "DownloadMetadata";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadata {
    pub asset_data_id: Vec<u8>,
    pub uri: String,
    #[serde(skip_serializing)]
    pub created_at: Option<NaiveDateTime>,
}

impl DownloadMetadata {
    pub fn sanitize(&mut self) {
        self.uri = self.uri.trim().replace('\0', "");
    }
}

impl IntoTaskData for DownloadMetadata {
    fn into_task_data(self) -> Result<TaskData, IngesterError> {
        let ts = self.created_at;
        let data =
            serde_json::to_value(self).map_err(<serde_json::Error as Into<IngesterError>>::into)?;
        Ok(TaskData {
            name: TASK_NAME,
            data,
            created_at: ts,
        })
    }
}

impl FromTaskData<DownloadMetadata> for DownloadMetadata {
    fn from_task_data(data: TaskData) -> Result<Self, IngesterError> {
        serde_json::from_value(data.data).map_err(|e| e.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadataTask {}

impl DownloadMetadataTask {
    async fn request_metadata(uri: String) -> Result<serde_json::Value, IngesterError> {
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(3))
            .build()?;
        let response = Client::get(&client, uri) // Need to check for malicious sites ?
            .send()
            .await;

        if let Err(e) = response {
            metric! {
                statsd_count!("ingester.bgtask.fetch_error", 1, 
                    "type" => TASK_NAME);
            }
            Err(IngesterError::FetchError(e.to_string()))
        } else if resp.status() != reqwest::StatusCode::OK {
            metric! {
                statsd_count!("ingester.bgtask.http_error", 1, 
                    "status" => status.unwrap_or("".to_string()).as_str(),
                    "type" => TASK_NAME);
            }
            Err(IngesterError::HttpError(e.to_string()))
        } else {
            let val: serde_json::Value = response.unwrap().json().await?;
            Ok(val)
        }
    }
}

#[async_trait]
impl BgTask for DownloadMetadataTask {
    fn name(&self) -> &'static str {
        TASK_NAME
    }

    fn lock_duration(&self) -> i64 {
        5
    }

    fn max_attempts(&self) -> i16 {
        3
    }

    async fn task(
        &self,
        db: &DatabaseConnection,
        data: serde_json::Value,
    ) -> Result<(), IngesterError> {
        let download_metadata: DownloadMetadata = serde_json::from_value(data)?;
        let meta_url = Url::parse(&download_metadata.uri);
        let body = match meta_url {
            Ok(_) => DownloadMetadataTask::request_metadata(download_metadata.uri).await?,
            _ => serde_json::Value::String("Invalid Uri".to_string()), //TODO -> enumize this.
        };
        let model = asset_data::ActiveModel {
            id: Unchanged(download_metadata.asset_data_id.clone()),
            metadata: Set(body),
            ..Default::default()
        };
        println!(
            "download metadata for {:?}",
            bs58::encode(download_metadata.asset_data_id.clone()).into_string()
        );
        asset_data::Entity::update(model)
            .filter(asset_data::Column::Id.eq(download_metadata.asset_data_id.clone()))
            .exec(db)
            .await
            .map(|_| ())
            .map_err(|db| {
                IngesterError::TaskManagerError(format!(
                    "Database error with {}, error: {}",
                    self.name(),
                    db
                ))
            })?;
        if meta_url.is_err() {
            return Err(IngesterError::UnrecoverableTaskError);
        }
        Ok(())
    }
}

impl Display for DownloadMetadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DownloadMetadata from {} for {:?}",
            self.uri, self.asset_data_id
        )
    }
}
