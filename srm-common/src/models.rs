use anyhow::{Context, Result, anyhow};
use bson::{DateTime as BsonDateTime, oid::ObjectId};
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use mongodb::{Collection, IndexModel, options::IndexOptions};
use serde::{Deserialize, Serialize};

pub const TELEMETRY_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
const TELEMETRY_TIMESTAMP_INDEX_NAME: &str = "telemetry_timestamp_ttl";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TelemetrySample {
    pub timestamp_utc: DateTime<Utc>,
    pub band: String,
    pub signal_strength: i32,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

impl TelemetrySample {
    pub fn new(
        timestamp_utc: DateTime<Utc>,
        band: String,
        signal_strength: i32,
        rx_bps: u64,
        tx_bps: u64,
    ) -> Self {
        Self {
            timestamp_utc,
            band,
            signal_strength,
            rx_bps,
            tx_bps,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MongoTelemetryRecord {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub timestamp_utc: BsonDateTime,
    pub band: String,
    pub signal_strength: i32,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

impl From<&TelemetrySample> for MongoTelemetryRecord {
    fn from(sample: &TelemetrySample) -> Self {
        Self {
            id: None,
            timestamp_utc: BsonDateTime::from_millis(sample.timestamp_utc.timestamp_millis()),
            band: sample.band.clone(),
            signal_strength: sample.signal_strength,
            rx_bps: sample.rx_bps,
            tx_bps: sample.tx_bps,
        }
    }
}

impl TryFrom<MongoTelemetryRecord> for TelemetrySample {
    type Error = anyhow::Error;

    fn try_from(record: MongoTelemetryRecord) -> Result<Self> {
        let Some(timestamp_utc) =
            DateTime::from_timestamp_millis(record.timestamp_utc.timestamp_millis())
        else {
            return Err(anyhow!("invalid BSON timestamp in telemetry record"));
        };

        Ok(Self {
            timestamp_utc,
            band: record.band,
            signal_strength: record.signal_strength,
            rx_bps: record.rx_bps,
            tx_bps: record.tx_bps,
        })
    }
}

pub async fn ensure_telemetry_indexes(collection: &Collection<MongoTelemetryRecord>) -> Result<()> {
    let desired_key = bson::doc! { "timestamp_utc": 1 };
    let desired_expire_after = std::time::Duration::from_secs(TELEMETRY_RETENTION_SECS);
    let existing_indexes: Vec<IndexModel> = collection
        .list_indexes()
        .await
        .context("failed to list telemetry indexes")?
        .try_collect()
        .await
        .context("failed to read telemetry indexes")?;
    let mut has_desired_index = false;

    for index in existing_indexes {
        if index.keys != desired_key {
            continue;
        }

        let index_name = index
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref());
        let expire_after = index
            .options
            .as_ref()
            .and_then(|options| options.expire_after);

        if index_name == Some(TELEMETRY_TIMESTAMP_INDEX_NAME)
            && expire_after == Some(desired_expire_after)
        {
            has_desired_index = true;
            continue;
        }

        if let Some(index_name) = index_name {
            collection
                .drop_index(index_name)
                .await
                .with_context(|| format!("failed to drop legacy telemetry index {index_name}"))?;
        }
    }

    if has_desired_index {
        return Ok(());
    }

    let ttl_timestamp_index = IndexModel::builder()
        .keys(desired_key)
        .options(
            IndexOptions::builder()
                .name(Some(TELEMETRY_TIMESTAMP_INDEX_NAME.to_string()))
                .expire_after(Some(desired_expire_after))
                .build(),
        )
        .build();

    collection
        .create_index(ttl_timestamp_index)
        .await
        .context("failed to create telemetry TTL index")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::DateTime as BsonDateTime;

    #[test]
    fn telemetry_sample_round_trips_through_mongo_record() {
        let timestamp = DateTime::parse_from_rfc3339("2026-03-15T18:44:12+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let sample =
            TelemetrySample::new(timestamp, "5G-1".to_string(), 78, 800_000_000, 720_000_000);

        let record = MongoTelemetryRecord::from(&sample);
        let recovered = TelemetrySample::try_from(record).unwrap();

        assert_eq!(recovered, sample);
    }

    #[test]
    fn invalid_bson_timestamp_is_rejected() {
        let record = MongoTelemetryRecord {
            id: None,
            timestamp_utc: BsonDateTime::from_millis(i64::MAX),
            band: "5G-1".to_string(),
            signal_strength: 78,
            rx_bps: 800_000_000,
            tx_bps: 720_000_000,
        };

        let error = TelemetrySample::try_from(record).unwrap_err();

        assert!(error.to_string().contains("invalid BSON timestamp"));
    }

    #[test]
    fn telemetry_retention_constant_matches_one_week() {
        assert_eq!(TELEMETRY_RETENTION_SECS, 7 * 24 * 60 * 60);
    }
}
