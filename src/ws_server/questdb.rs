use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
// Remove unused import
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};
use tokio_postgres::types::ToSql;
use tracing::{debug, error, info, trace};

use crate::ws_server::ilp::IlpHttp;

#[derive(Clone)]
pub struct QuestDb {
    inner: Arc<Mutex<Option<Client>>>,
    ilp: Arc<IlpHttp>,
    table_name: Arc<String>,
    time_col: Arc<String>,
}

#[derive(Clone, Deserialize)]
pub struct QuestDbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub table_name: Option<String>,     // p.ej. "flight_telemetry"
    pub time_col: Option<String>,       // p.ej. "timestamp"
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
        let table_name = cfg.table_name.clone().unwrap_or_else(|| "flight_telemetry".to_string());
        let time_col = cfg.time_col.clone().unwrap_or_else(|| "timestamp".to_string());
    
        let questdb = Self {
            inner: Arc::new(Mutex::new(Some(client))),
            ilp: Arc::new(IlpHttp::new(
                std::env::var("QDB_ILP_URL").unwrap_or_else(|_| "http://localhost:9000/imp".into()),
                &table_name,
            )),
            table_name: Arc::new(table_name),
            time_col: Arc::new(time_col),
        };
    
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
        for (i, rec) in records.iter().enumerate() {
            let obj = rec.as_object().ok_or_else(|| "record is not a JSON object".to_string())?;
            
            // Convert JSON object to fields map
            use serde_json::{Map, Value};
            
            let mut fields = Map::new();
            let mut ts_ns = None;
            
            for (k, v) in obj {
                if let Some(ts_field) = ts_field && k == ts_field {
                    if let Some(ts) = v.as_i64() {
                        ts_ns = Some(ts);
                    }
                } else {
                    // Clone the value directly since we need to maintain the original Value type
                    fields.insert(k.clone(), v.clone());
                }
            }
            
            if fields.is_empty() {
                tracing::warn!("ingest_skip: record {} had 0 numeric fields (only tags?) keys={:?}", i, obj.keys().collect::<Vec<_>>());
                continue;
            }
            
            if let Some(line) = self.ilp.json_to_line(&tags, &fields, ts_ns) {
                lines.push(line);
            } else {
                tracing::warn!("ingest_skip: json_to_line returned None for record {}", i);
            }
        }
        tracing::debug!("ingest: table={}, lines={}, example={:?}", self.table_name, lines.len(), lines.get(0));
        self.ilp.write_lines(&lines).await.map_err(|e| e.to_string())?;
        Ok(lines.len())
    }

    async fn ensure_schema(&self) -> Result<()> {
        let tbl = &*self.table_name;
        let tsc = &*self.time_col;

        let ddl = format!(r#"
    CREATE TABLE IF NOT EXISTS "{tbl}" (
        "{tsc}" TIMESTAMP,
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
    ) TIMESTAMP("{tsc}") PARTITION BY DAY;

    CREATE TABLE IF NOT EXISTS flight_logs (
        ts TIMESTAMP,
        flight_id SYMBOL,
        payload STRING
    ) TIMESTAMP(ts) PARTITION BY DAY;

    CREATE TABLE IF NOT EXISTS logger_configs (
        ts TIMESTAMP,
        config_json STRING
    ) TIMESTAMP(ts) PARTITION BY DAY;
    "#);

        let guard = self.inner.lock().await;
        let client = guard.as_ref().ok_or_else(|| anyhow!("Not connected to QuestDB"))?;
        client.batch_execute(&ddl).await?;
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
    pub async fn insert_logger_config_legacy(&self, _config_json: &str) -> Result<()> {
        let q = r#"
            INSERT INTO flight_logs (
                timestamp, flight_id, schema_version, mode, 
                AngleRoll, AnglePitch, Yaw, RateRoll, RatePitch, RateYaw,
                GyroXdps, GyroYdps, GyroZdps, InputThrottle, InputRoll, InputPitch, InputYaw,
                MotorInput1, MotorInput2, MotorInput3, MotorInput4, error_phi, error_theta, ErrorYaw,
                Altura, tau_x, tau_y, tau_z, Kc, Ki
            ) VALUES (
                now(), $1, '1.0', 'config', 
                0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0
            )"#;
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
        client.execute(q, &[&"__config__"]).await?;
        Ok(())
    }

    pub async fn list_flights(&self, limit: i64) -> Result<Vec<(String, DateTime<Utc>)>> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow!("Not connected to QuestDB"))?;
        let q = format!(r#"
            SELECT flight_id, max("{ts}") AS last_ts
            FROM "{tbl}"
            GROUP BY flight_id
            ORDER BY last_ts DESC
            LIMIT $1
        "#, tbl = self.table_name, ts = self.time_col);
    
        let rows = client.query(&q, &[&limit]).await?;
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
        let client = client_guard.as_ref().ok_or_else(|| anyhow!("Not connected to QuestDB"))?;
    
        // Construye la base de la consulta según filtros
        let mut base = format!(r#"
            SELECT "{ts}",
                   flight_id, schema_version, mode,
                   AngleRoll, AnglePitch, Yaw,
                   RateRoll, RatePitch, RateYaw,
                   GyroXdps, GyroYdps, GyroZdps,
                   InputThrottle, InputRoll, InputPitch, InputYaw,
                   MotorInput1, MotorInput2, MotorInput3, MotorInput4,
                   error_phi, error_theta, ErrorYaw,
                   Altura, tau_x, tau_y, tau_z,
                   Kc, Ki
            FROM "{tbl}"
            WHERE flight_id = $1
        "#, tbl = self.table_name, ts = self.time_col);
    
        // Use a tuple to store parameters to ensure Send
        let mut param_idx = 2;
        let mut params: Vec<Box<dyn ToSql + Send + Sync>> = vec![Box::new(flight_id.to_string())];
        
        if let Some(f) = from {
            base.push_str(&format!(r#" AND "{ts}" >= ${}"#, param_idx, ts = self.time_col));
            params.push(Box::new(f) as Box<dyn ToSql + Send + Sync>);
            param_idx += 1;
        }
        if let Some(t) = to {
            base.push_str(&format!(r#" AND "{ts}" <= ${}"#, param_idx, ts = self.time_col));
            params.push(Box::new(t) as Box<dyn ToSql + Send + Sync>);
            param_idx += 1;
        }
        base.push_str(&format!(r#" ORDER BY "{ts}" LIMIT ${}"#, param_idx, ts = self.time_col));
        params.push(Box::new(limit) as Box<dyn ToSql + Send + Sync>);
        
        // Convert to references for the query
        let param_refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| &**p as &(dyn ToSql + Sync)).collect();
        let rows = client.query(&base, &param_refs[..]).await?;
    
        // Convierte a tu FlightPoint (arma JSON en Rust)
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let ts: DateTime<Utc> = r.get::<_, DateTime<Utc>>(self.time_col.as_str());
            let mut payload = serde_json::Map::new();
    
            macro_rules! put {
                ($name:expr, $ty:ty) => {
                    if let Ok(v) = r.try_get::<_, $ty>($name) {
                        payload.insert($name.to_string(), serde_json::json!(v));
                    }
                };
            }
    
            put!("flight_id", String);
            put!("schema_version", String);
            put!("mode", String);
            put!("AngleRoll", f64);
            put!("AnglePitch", f64);
            put!("Yaw", f64);
            put!("RateRoll", f64);
            put!("RatePitch", f64);
            put!("RateYaw", f64);
            put!("GyroXdps", f64);
            put!("GyroYdps", f64);
            put!("GyroZdps", f64);
            put!("InputThrottle", i64);
            put!("InputRoll", i64);
            put!("InputPitch", i64);
            put!("InputYaw", i64);
            put!("MotorInput1", i64);
            put!("MotorInput2", i64);
            put!("MotorInput3", i64);
            put!("MotorInput4", i64);
            put!("error_phi", f64);
            put!("error_theta", f64);
            put!("ErrorYaw", f64);
            put!("Altura", f64);
            put!("tau_x", f64);
            put!("tau_y", f64);
            put!("tau_z", f64);
            put!("Kc", f64);
            put!("Ki", f64);
    
            out.push(FlightPoint { ts, payload: serde_json::Value::Object(payload) });
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
    
    pub async fn ingest_telemetry_batch(
        &self,
        flight_id: &str,
        schema_version: &str,
        mode: Option<&str>,
        records: &[serde_json::Value],
        ts_field: Option<&str>,
    ) -> Result<usize, String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref()
            .unwrap()
            .ingest_telemetry_batch(flight_id, schema_version, mode, records, ts_field)
            .await
            .map_err(|e| e.to_string())
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
