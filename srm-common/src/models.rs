use anyhow::{Result, anyhow};
use bson::{DateTime as BsonDateTime, oid::ObjectId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
