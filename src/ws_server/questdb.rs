use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
// Remove unused import
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};
use tokio_postgres::types::ToSql;
use tracing::{debug, error, info, trace, warn};

use crate::ws_server::ilp::{choose_timestamp_ns, IlpHttp};

#[derive(Clone)]
pub struct QuestDb {
    inner: Arc<Mutex<Option<Client>>>,
    ilp: Arc<IlpHttp>,
}

#[derive(Clone, Deserialize)]
pub struct QuestDbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

#[derive(Clone, Debug)]
pub struct FlightPoint {
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl QuestDb {
    pub async fn connect(cfg: QuestDbConfig) -> Result<Self> {
        info!("🔌 Conectando a QuestDB en {}:{}", cfg.host, cfg.port);

        let connection_string = format!(
            "host={} port={} user={} password={} dbname={}",
            cfg.host, cfg.port, cfg.user, cfg.password, cfg.database
        );

        // First try to connect to the database
        let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
            .await
            .map_err(|e| {
                error!("❌ No se pudo conectar a QuestDB: {}", e);
                anyhow!("Failed to connect to QuestDB: {}", e)
            })?;

        // Start connection listener
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("❌ Error de conexión con QuestDB: {}", e);
            }
        });

        // Create QuestDb instance
        let questdb = Self {
            inner: Arc::new(Mutex::new(Some(client))),
            ilp: Arc::new(IlpHttp::new(
                std::env::var("QDB_ILP_URL").unwrap_or_else(|_| "http://localhost:9000/imp".into()),
                "telemetry"  // Measurement name matching the table
            )),
        };
        
        // Remove unused import
        use tracing::{debug, error, info, trace};

        // Ensure schema exists
        questdb.ensure_schema().await.map_err(|e| {
            error!("❌ Error al crear el esquema: {}", e);
            anyhow!("Failed to create database schema: {}", e)
        })?;

        info!("✅ Conexión a QuestDB establecida");
        Ok(questdb)
    }

    async fn ensure_connected(&self) -> Result<()> {
        let client = self.inner.lock().await;
        if client.is_none() {
            bail!("Not connected to QuestDB");
        }
        Ok(())
    }

    pub async fn ingest_telemetry_batch(
        &self,
        flight_id: &str,
        schema_version: &str,
        mode: Option<&str>,
        records: &[serde_json::Value],
        ts_field: Option<&str>,
    ) -> Result<usize, String> {
        self.ensure_connected().await.map_err(|e| e.to_string())?;
        
        // Create tags map
        let mut tags = BTreeMap::new();
        tags.insert("flight_id".to_string(), flight_id.to_string());
        tags.insert("schema_version".to_string(), schema_version.to_string());
        if let Some(m) = mode {
            tags.insert("mode".to_string(), m.to_string());
        }
        
        // Build ILP lines
        let mut lines = Vec::with_capacity(records.len());
        for rec in records {
            let obj = rec
                .as_object()
                .ok_or_else(|| "record is not a JSON object".to_string())?;
    
            // Get timestamp in nanoseconds (i64)
            let ts_ns = ts_field
                .and_then(|k| obj.get(k).and_then(|v| v.as_str()))
                .map(|iso| choose_timestamp_ns(Some(iso)))
                .unwrap_or_else(|| choose_timestamp_ns(None));
    
            // Extract fields (excluding tags that might be in the payload)
            let mut fields = serde_json::Map::with_capacity(obj.len());
            for (k, v) in obj {
                if matches!(k.as_str(), "flight_id" | "schema_version" | "mode") {
                    continue;
                }
                fields.insert(k.clone(), v.clone());
            }
    
            if let Some(line) = self.ilp.json_to_line(&tags, &fields, Some(ts_ns)) {
                lines.push(line);
            }
        }
    
        // Write to /imp (one transaction per batch)
        self.ilp.write_lines(&lines).await.map_err(|e| e.to_string())?;
        Ok(lines.len())
    }

    async fn ensure_schema(&self) -> Result<()> {
        let ddl = r#"
        -- Telemetry data table
        CREATE TABLE IF NOT EXISTS telemetry (
            ts TIMESTAMP,
            flight_id SYMBOL,
            schema_version SYMBOL,
            mode SYMBOL,
            AngleRoll DOUBLE, AnglePitch DOUBLE, Yaw DOUBLE,
            RateRoll DOUBLE, RatePitch DOUBLE, RateYaw DOUBLE,
            GyroXdps DOUBLE, GyroYdps DOUBLE, GyroZdps DOUBLE,
            InputThrottle LONG, InputRoll LONG, InputPitch LONG, InputYaw LONG,
            MotorInput1 LONG, MotorInput2 LONG, MotorInput3 LONG, MotorInput4 LONG,
            error_phi DOUBLE, error_theta DOUBLE, ErrorYaw DOUBLE,
            Altura DOUBLE, tau_x DOUBLE, tau_y DOUBLE, tau_z DOUBLE,
            Kc DOUBLE, Ki DOUBLE
        ) TIMESTAMP(ts) PARTITION BY DAY;

        -- Raw flight logs table
        CREATE TABLE IF NOT EXISTS flight_logs (
            ts TIMESTAMP,
            flight_id SYMBOL,
            payload STRING
        ) TIMESTAMP(ts) PARTITION BY DAY;

        -- Logger configurations table
        CREATE TABLE IF NOT EXISTS logger_configs (
            ts TIMESTAMP,
            config_json STRING
        ) TIMESTAMP(ts) PARTITION BY DAY;
        "#;

        let guard = self.inner.lock().await;
        let client = guard.as_ref().ok_or_else(|| anyhow!("Not connected to QuestDB"))?;
        client.batch_execute(ddl).await?;
        Ok(())
    }

    /// Inserta telemetría cruda asociada a un flight_id
    pub async fn insert_flight_log(&self, flight_id: &str, payload_json: &str) -> Result<()> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;

        match client.execute(
            "INSERT INTO flight_logs (ts, flight_id, payload) VALUES (now(), $1, $2)",
            &[&flight_id, &payload_json],
        ).await {
            Ok(_) => {
                trace!("📊 Log de vuelo insertado: {}", flight_id);
                Ok(())
            },
            Err(e) => {
                error!("❌ Error insertando log de vuelo: {}", e);
                Err(e.into())
            }
        }
    }

    /// Guarda la configuración/eventos (start/stop) en `logger_configs`
    pub async fn insert_logger_config(&self, config_json: &str) -> Result<()> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;

        match client.execute(
            "INSERT INTO logger_configs (ts, config_json) VALUES (now(), $1)",
            &[&config_json],
        ).await {
            Ok(_) => {
                debug!("⚙️  Configuración guardada en QuestDB");
                Ok(())
            },
            Err(e) => {
                error!("❌ Error guardando configuración: {}", e);
                Err(e.into())
            }
        }
    }

    /// Alternativa: guarda configs dentro de `flight_logs` con flight_id='__config__'
    pub async fn insert_logger_config_legacy(&self, config_json: &str) -> Result<()> {
        let q = "INSERT INTO flight_logs (ts, flight_id, payload) VALUES (now(), $1, $2)";
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
        client.execute(q, &[&"__config__", &config_json]).await?;
        Ok(())
    }

    pub async fn list_flights(&self, limit: i64) -> Result<Vec<(String, DateTime<Utc>)>> {
        let client_guard = self.inner.lock().await;
        let client = client_guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
        
        let params: &[&(dyn ToSql + Sync)] = &[&limit];
        let rows = client
            .query(
                "SELECT flight_id, max(ts) AS last_ts
                 FROM telemetry
                 GROUP BY flight_id
                 ORDER BY last_ts DESC
                 LIMIT $1",
                params,
            )
            .await?;
    
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    pub async fn fetch_flight_points(
        &self,
        flight_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FlightPoint>> {
        let client_guard = self.inner.lock().await;
        let client = client_guard.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
    
        let rows = match (from, to) {
            (None, None) => {
                let params: &[&(dyn ToSql + Sync)] = &[&flight_id, &limit];
                client.query(
                    "SELECT ts, row_to_json(t) AS payload
                     FROM (
                        SELECT *
                        FROM telemetry
                        WHERE flight_id = $1
                        ORDER BY ts
                        LIMIT $2
                     ) t",
                    params,
                ).await?
            }
            (Some(f), None) => {
                let params: &[&(dyn ToSql + Sync)] = &[&flight_id, &f, &limit];
                client.query(
                    "SELECT ts, row_to_json(t) AS payload
                     FROM (
                        SELECT *
                        FROM telemetry
                        WHERE flight_id = $1 AND ts >= $2
                        ORDER BY ts
                        LIMIT $3
                     ) t",
                    params,
                ).await?
            }
            (None, Some(t)) => {
                let params: &[&(dyn ToSql + Sync)] = &[&flight_id, &t, &limit];
                client.query(
                    "SELECT ts, row_to_json(t) AS payload
                     FROM (
                        SELECT *
                        FROM telemetry
                        WHERE flight_id = $1 AND ts <= $2
                        ORDER BY ts
                        LIMIT $3
                     ) t",
                    params,
                ).await?
            }
            (Some(f), Some(t)) => {
                let params: &[&(dyn ToSql + Sync)] = &[&flight_id, &f, &t, &limit];
                client.query(
                    "SELECT ts, row_to_json(t) AS payload
                     FROM (
                        SELECT *
                        FROM telemetry
                        WHERE flight_id = $1 AND ts >= $2 AND ts <= $3
                        ORDER BY ts
                        LIMIT $4
                     ) t",
                    params,
                ).await?
            }
        };
    
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let ts: DateTime<Utc> = r.get(0);
            let payload_str: String = r.get(1);
            let payload = serde_json::from_str::<serde_json::Value>(&payload_str)
                .unwrap_or_else(|_| serde_json::json!({ "raw": payload_str }));
            out.push(FlightPoint { ts, payload });
        }
        Ok(out)
    }
    
}

#[derive(Clone)]
pub struct OptionalDb {
    inner: Arc<Mutex<Option<QuestDb>>>,
    config: QuestDbConfig,
}

impl OptionalDb {
    pub fn new(config: QuestDbConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            config,
        }
    }

    async fn ensure_connected(&self) -> Result<(), String> {
        let mut db = self.inner.lock().await;
        if db.is_none() {
            match QuestDb::connect(self.config.clone()).await {
                Ok(new_db) => { *db = Some(new_db); Ok(()) }
                Err(e) => Err(e.to_string()),
            }
        } else {
            Ok(())
        }
    }

    pub async fn insert_flight_log(&self, flight_id: &str, payload: &str) -> Result<(), String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref()
            .unwrap()
            .insert_flight_log(flight_id, payload)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn insert_logger_config(&self, config: &str) -> Result<(), String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref()
            .unwrap()
            .insert_logger_config(config)
            .await
            .map_err(|e| e.to_string())
    }

    // Delegados que usa mod.rs
    pub async fn list_flights(&self, limit: i64) -> Result<Vec<(String, DateTime<Utc>)>, String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref().unwrap()
            .list_flights(limit).await
            .map_err(|e| e.to_string())
    }

    pub async fn fetch_flight_points(
        &self,
        flight_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FlightPoint>, String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref().unwrap()
            .fetch_flight_points(flight_id, from, to, limit)
            .await
            .map_err(|e| e.to_string())
    }
}
