use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use bson::{DateTime as BsonDateTime, doc};
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use mongodb::{Client, Collection};
use serde::Deserialize;
use srm_common::config::{ApiConfig, env_or_manifest_path, load_toml_file};
use srm_common::models::{MongoTelemetryRecord, TelemetrySample, ensure_telemetry_indexes};

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_ENV_VAR: &str = "SRM_DATA_API_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config/api.toml";

#[derive(Clone)]
struct AppState {
    collection: Collection<MongoTelemetryRecord>,
}

#[derive(Deserialize)]
struct TelemetryQuery {
    start: String,
    end: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error=fatal details={}", error);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    println!("{} v{}", APP_NAME, APP_VERSION);
    let config_path = env_or_manifest_path(
        CONFIG_ENV_VAR,
        DEFAULT_CONFIG_PATH,
        env!("CARGO_MANIFEST_DIR"),
    );
    let config: ApiConfig = load_toml_file(&config_path)?;

    let client = Client::with_uri_str(&config.mongodb.url).await?;
    let collection = client
        .database(&config.mongodb.database)
        .collection::<MongoTelemetryRecord>(&config.mongodb.collection);
    ensure_telemetry_indexes(&collection).await?;

    let state = AppState { collection };

    let app = Router::new()
        .route("/telemetry", get(fetch_telemetry))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind_address).await?;
    println!("listening=http://{}", config.server.bind_address);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn fetch_telemetry(
    State(state): State<AppState>,
    Query(query): Query<TelemetryQuery>,
) -> Result<Json<Vec<TelemetrySample>>, ApiError> {
    let start = parse_rfc3339(&query.start)?;
    let end = parse_rfc3339(&query.end)?;

    if start > end {
        return Err(ApiError::bad_request("start must be before end"));
    }

    let filter = doc! {
        "timestamp_utc": {
            "$gte": BsonDateTime::from_millis(start.timestamp_millis()),
            "$lte": BsonDateTime::from_millis(end.timestamp_millis()),
        }
    };

    let documents: Vec<MongoTelemetryRecord> = state
        .collection
        .find(filter)
        .sort(doc! { "timestamp_utc": 1 })
        .await
        .map_err(ApiError::internal)?
        .try_collect()
        .await
        .map_err(ApiError::internal)?;

    let mut samples = Vec::with_capacity(documents.len());
    for document in documents {
        samples.push(TelemetrySample::try_from(document).map_err(ApiError::internal)?);
    }

    Ok(Json(samples))
}

fn parse_rfc3339(value: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| ApiError::bad_request(&format!("invalid RFC3339 timestamp: {}", error)))
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}
