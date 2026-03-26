use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use axum::{extract::State, Json, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};
use tracing::{error, info, warn, trace};
use tokio_postgres::types::ToSql;
use crate::models::experiment_metadata::ExperimentMetadata;

use crate::ws_server::http_server::AppState;
use crate::ws_server::ilp::{IlpHttp, choose_timestamp_ns};
use std::future::Future;
use std::pin::Pin;
#[derive(Clone, Debug)]
pub struct QuestDb {
    inner: Arc<Mutex<Option<Client>>>,
    ilp: Arc<IlpHttp>,
    table_name: Arc<String>,
    time_col: Arc<String>,
    config: Arc<Mutex<QuestDbConfig>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestDbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub table_name: Option<String>,
    pub time_col: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FlightPoint {
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

#[derive(serde::Serialize)]
struct IngestResp {
    status: String,
    inserted: usize,
    #[serde(rename = "flightId")]
    flight_id: String,
}

#[derive(serde::Serialize)]
pub struct ProbeResp { 
    pub ok: bool, 
    pub rows: i64 
}

pub async fn probe_sql_insert(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProbeResp>, (StatusCode, String)> {
    let ws_ctx = state.ws_ctx.lock().await;
    // 1) Get QuestDb and PG Client
    let (_table, _tcol, rows) = {
        // OptionalDb -> QuestDb
        let qdb_guard = ws_ctx.questdb.inner.lock().await;
        let qdb = qdb_guard.as_ref()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "QuestDB not connected".to_string()))?;

        let table = qdb.table_name.to_string();
        let tcol  = qdb.time_col.to_string();

        let client_guard = qdb.inner.lock().await;
        let client = client_guard.as_ref()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "No PostgreSQL client".to_string()))?;

        // 2) Insert de prueba
        let q = format!(r#"
            INSERT INTO "{table}" ("{tcol}", flight_id, schema_version, mode, AngleRoll, InputThrottle)
            VALUES (now(), 'probe_fid', '1', 'probe', 1.23, 1234)
        "#);
        client.batch_execute(&q).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("probe insert failed: {e}")))?;

        // 3) Conteo
        let count_q = format!(r#"SELECT count() FROM "{table}" WHERE flight_id='probe_fid'"#);
        let row = client.query_one(&count_q, &[]).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("probe count failed: {e}")))?;
        (table, tcol, row.get::<_, i64>(0))
    };

    //info!("probe_sql_insert ok: table={} time_col={} rows={}", table, tcol, rows);
    Ok(Json(ProbeResp { ok: true, rows }))
}

impl QuestDb {
    async fn create_connection(cfg: &QuestDbConfig) -> Result<Client> {
        let connection_string = format!(
            "host={} port={} user={} password={} dbname={}",
            cfg.host, cfg.port, cfg.user, cfg.password, cfg.database
        );
        
        let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
            .await
            .map_err(|e| {
                error!("❌ Failed to connect to QuestDB: {}", e);
                anyhow!("Failed to connect to QuestDB: {}", e)
            })?;

        // Spawn the connection task
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("❌ QuestDB connection error: {}", e);
            }
        });

        Ok(client)
    }

    pub async fn connect(cfg: QuestDbConfig) -> Result<Self> {
        info!("🔌 Connecting to QuestDB at {}:{}", cfg.host, cfg.port);

        // Clone the config values we need before moving cfg
        let table_name = cfg.table_name.clone().unwrap_or_else(|| "flight_telemetry".to_string());
        let time_col = cfg.time_col.clone().unwrap_or_else(|| "timestamp".to_string());
        
        // Create initial connection
        let client = match Self::create_connection(&cfg).await {
            Ok(client) => client,
            Err(e) => {
                error!("❌ Failed to establish initial connection to QuestDB: {}", e);
                return Err(e);
            }
        };

        let ilp = Arc::new(IlpHttp::new(
            std::env::var("QDB_ILP_URL").unwrap_or_else(|_| "http://127.0.0.1:9000".into()),
            &table_name,
        ));
        
        let questdb = Self {
            inner: Arc::new(Mutex::new(Some(client))),
            ilp,
            table_name: table_name.into(),
            time_col: time_col.into(),
            config: Arc::new(Mutex::new(cfg)),
        };

        // Verify connection and schema
        match questdb.ensure_schema().await {
            Ok(_) => {
                //info!("✅ Successfully connected to QuestDB and verified schema");
                Ok(questdb)
            }
            Err(e) => {
                error!("❌ Failed to verify schema: {}", e);
                Err(anyhow!("Failed to verify schema: {}", e))
            }
        }
    }

    async fn with_connection<T, F>(&self, mut f: F) -> Result<T>
    where
        T: Send + 'static,
        // La callback puede tomar &Client con cualquier 'c
        // y regresa un Future que vive al menos 'c.
        F: for<'c> FnMut(&'c Client) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'c>>
            + Send
            + 'static,
    {
        // 1) Saca el client del pool
        let mut client = {
            let mut guard = self.inner.lock().await;
            guard.take()
        };
    
        // 2) Si no hay cliente, reconecta
        if client.is_none() {
            tracing::error!("⚠️ No PG client in pool; reconnecting to QuestDB...");
            let cfg = self.config.lock().await.clone();
            client = Some(Self::create_connection(&cfg).await?);
        }
    
        let client = client.expect("client should exist here");
    
        // 3) Primer intento
        let first = f(&client).await;
    
        // 4) Devuelve SIEMPRE el client al pool
        {
            let mut guard = self.inner.lock().await;
            *guard = Some(client);
        }
    
        // 5) Si ok → listo
        if first.is_ok() {
            return first;
        }
    
        // 6) Reintento: reconecta y vuelve a invocar la callback
        tracing::error!(
            "❌ Query failed: {}. Reconnecting and retrying once...",
            first.as_ref().err().unwrap()
        );
    
        let cfg = self.config.lock().await.clone();
        let new_client = Self::create_connection(&cfg).await?;
        let second = f(&new_client).await;
    
        {
            let mut guard = self.inner.lock().await;
            *guard = Some(new_client);
        }
    
        second
    }
    
    pub async fn ingest_telemetry_batch(
        &self,
        flight_id: &str,
        schema_version: &str,
        mode: Option<&str>,
        records: &[serde_json::Value],
        ts_field: Option<&str>,
    ) -> Result<usize, String> {
        // valida conexión PG (no por ILP, pero nos sirve para saber que está vivo)
        {
            let guard = self.inner.lock().await;
            if guard.is_none() {
                return Err("Not connected to QuestDB".to_string());
            }
        }
        fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }
        // tags
        let mut tags = BTreeMap::new();
        tags.insert("flight_id".to_string(), flight_id.to_string());
        tags.insert("schema_version".to_string(), schema_version.to_string());
        if let Some(m) = mode {
            tags.insert("mode".to_string(), m.to_string());
        }

        let mut lines = Vec::with_capacity(records.len());

        for (i, rec) in records.iter().enumerate() {
            let obj = rec
                .as_object()
                .ok_or_else(|| "record is not a JSON object".to_string())?;

            let mut ts_ns: Option<i64> = None;
            let mut fields = serde_json::Map::new();

            for (k, v) in obj {
                // timestamp
                if let Some(tf) = ts_field {
                    if k == tf {
                        if let Some(s) = v.as_str() {
                            // ISO8601 → ns
                            ts_ns = Some(choose_timestamp_ns(Some(s)));
                        } else if let Some(n) = v.as_i64() {
                            // epoch ns
                            ts_ns = Some(n);
                        }
                        continue;
                    }
                }

                // evita duplicar tags
                if matches!(k.as_str(), "flight_id" | "schema_version" | "mode") {
                    continue;
                }

                let key = k.as_str();

                if v.is_number() {
                    let (key_norm, val_norm) = match key {
                        // mapeos previos que ya tenías
                        "AngleYaw" => ("Yaw", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "gyroRatePitch" => ("RatePitch", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "gyroRateRoll"  => ("RateRoll",  serde_json::json!(v.as_f64().unwrap_or(0.0))),
                    
                        // motores → float
                        "MotorInput1" | "MotorInput2" | "MotorInput3" | "MotorInput4" => {
                            (key, serde_json::json!(v.as_f64().unwrap_or(0.0)))
                        }
                    
                        // NUEVO: variables con decimales (DOUBLE) redondeadas a 2dp
                        "m" | "g" | "k" | "m1" | "m2" | "m3" | "m4" => {
                            let f = v.as_f64().unwrap_or(0.0);
                            (key, serde_json::json!(round2(f))) // DOUBLE (con 2 decimales)
                        }

                        "phi_ref" => ("phi_ref", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "theta_ref" => ("theta_ref", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "KalmanAngleRoll" => ("KalmanAngleRoll", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "KalmanAnglePitch" => ("KalmanAnglePitch", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "Motor1" => ("Motor1", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "Motor2" => ("Motor2", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "Motor3" => ("Motor3", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "Motor4" => ("Motor4", serde_json::json!(v.as_f64().unwrap_or(0.0))),

                        // NUEVO (si tienes variantes tipo 'm_kg' u otras): ejemplo de prefijo 'm' + dígitos
                        _ if key.starts_with('m') && key[1..].chars().all(|c| c.is_ascii_digit()) => {
                            let f = v.as_f64().unwrap_or(0.0);
                            (key, serde_json::json!(round2(f)))
                        }
                    
                        // errores → float
                        "error_phi" | "error_theta" => {
                            (key, serde_json::json!(v.as_f64().unwrap_or(0.0)))
                        }
                    
                        // inputs discretos → enteros (LONG)
                        "InputThrottle" | "InputRoll" | "InputPitch" | "InputYaw" => {
                            let ival = if let Some(i) = v.as_i64() {
                                i
                            } else {
                                v.as_f64().unwrap_or(0.0).round() as i64
                            };
                            (key, serde_json::json!(ival))
                        }
                    
                        // por defecto: conserva tipo
                        _ => (key, v.clone()),
                    };
                    
                    // evita duplicar tags
                    if matches!(key_norm, "flight_id" | "schema_version" | "mode") {
                        continue;
                    }
                
                    fields.insert(key_norm.to_string(), val_norm);
                }
            }

            if fields.is_empty() {
                tracing::warn!(
                    "ingest_skip[{}]: no había campos numéricos; keys={:?}",
                    i,
                    obj.keys().collect::<Vec<_>>()
                );
                continue;
            }

            let ts = ts_ns.or_else(|| Some(choose_timestamp_ns(None)));

            if let Some(line) = self.ilp.json_to_line(&tags, &fields, ts) {
                if i == 0 {
                }
                lines.push(line);
            } else {
                tracing::warn!("ingest_skip: json_to_line devolvió None para record {}", i);
            }
        }

        if lines.is_empty() {
            return Ok(0);
        }

        // 👉 usa ILP /imp
        self.write_lines(&lines).await.map_err(|e| e.to_string())?;
        Ok(lines.len())
    }

    /// Wrapper para ILP (usa self.ilp → /imp)
    pub async fn write_lines(&self, lines: &[String]) -> anyhow::Result<()> {
        tracing::debug!("ilp_write(wrapper): lines={}", lines.len());
        self.ilp.write_lines(lines).await
    }
    
    async fn ensure_schema(&self) -> Result<()> {
        let tbl = &*self.table_name;
        let tsc = &*self.time_col;
    
        let ddl = format!(r#"
        CREATE TABLE IF NOT EXISTS "{tbl}" (
            "{tsc}" TIMESTAMP,
            rx_timestamp_ns LONG,
            flight_id SYMBOL,
            schema_version SYMBOL,
            mode SYMBOL,
            
            AngleRoll DOUBLE, AnglePitch DOUBLE, Yaw DOUBLE,
            RateRoll DOUBLE, RatePitch DOUBLE, RateYaw DOUBLE,
            
            GyroXdps DOUBLE, GyroYdps DOUBLE, GyroZdps DOUBLE,
            
            InputThrottle LONG, InputRoll LONG, InputPitch LONG, InputYaw LONG,
            
            MotorInput1 DOUBLE, MotorInput2 DOUBLE, MotorInput3 DOUBLE, MotorInput4 DOUBLE,
            
            error_phi DOUBLE, error_theta DOUBLE, ErrorYaw DOUBLE,
            Altura DOUBLE, tau_x DOUBLE, tau_y DOUBLE, tau_z DOUBLE,
            Kc DOUBLE, Ki DOUBLE,
            
            m DOUBLE, g DOUBLE, k DOUBLE,
            m1 DOUBLE, m2 DOUBLE, m3 DOUBLE, m4 DOUBLE
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
    CREATE TABLE IF NOT EXISTS experiment_metadata (
        flight_id SYMBOL,
        experiment_id SYMBOL,
        start_time TIMESTAMP,
        end_time TIMESTAMP,
        duration_seconds DOUBLE,
        sampling_rate_hz LONG,
        esp32_loop_hz LONG,
        filter_type SYMBOL,
        experiment_type SYMBOL,
        kalman_k1 DOUBLE,
        kalman_k2 DOUBLE,
        kalman_k3 DOUBLE,
        kalman_g1 DOUBLE,
        kalman_g2 DOUBLE,
        kalman_g3 DOUBLE,
        kalman_m1 DOUBLE,
        kalman_m2 DOUBLE,
        kalman_m3 DOUBLE,
        description STRING,
        location STRING,
        notes STRING,
        created_at TIMESTAMP
    ) TIMESTAMP(created_at) PARTITION BY DAY;
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

    async fn existing_columns(&self, client: &tokio_postgres::Client) -> anyhow::Result<HashSet<String>> {
        let rows = client.query(
            "SELECT column_name FROM information_schema.columns WHERE table_name = $1",
            &[&self.table_name.as_str()],
        ).await?;
        let mut set = HashSet::new();
        for r in rows {
            let name: String = r.get(0);
            set.insert(name);
        }
        Ok(set)
    }

    async fn detect_logger_configs_col(&self) -> Result<String> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;

        // 1) Read the actual schema of the table
        let rows = client.query(
            "SELECT column_name FROM information_schema.columns WHERE lower(table_name) = lower($1)",
            &[&self.table_name.as_str()],
        ).await?;

        // 2) Build a set of existing columns
        let mut cols = HashSet::new();
        for row in rows {
            let col_name: String = row.get(0);
            cols.insert(col_name);
        }

        // 3) Try these candidates in order
        for cand in ["payload", "config", "data", "json", "value"] {
            if cols.contains(cand) {
                return Ok(cand.to_string());
            }
        }

        // 4) If no candidate exists, use a default and log an error
        error!("No suitable column found in logger_configs, using 'payload' as fallback");
        Ok("payload".to_string())
    }

    /// Guarda la configuración/eventos (start/stop) en `logger_configs`
    pub async fn insert_logger_config(&self, config_json: &str) -> Result<()> {

        let client = self.inner.lock().await;
        let client = match client.as_ref() {
            Some(c) => c,
            None => {
                let error_msg = "❌ No se pudo guardar la configuración: No hay conexión a QuestDB";
                error!("{}", error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
        };

        // Use the fixed column name 'config_json' as defined in the table schema
        let query = "INSERT INTO logger_configs (ts, config_json) VALUES (now(), $1)";
        
        match client.execute(query, &[&config_json]).await {
            Ok(_rows) => {
                //info!("✅ Configuración guardada exitosamente en QuestDB (filas afectadas: {})", rows);
                info!("📋 Configuración guardada: {}", config_json);
                Ok(())
            },
            Err(e) => {
                let error_msg = format!("❌ Error guardando configuración: {}", e);
                error!("{}", error_msg);
                
                // Try the legacy method if the main method fails
                warn!("⚠️  Intentando método alternativo para guardar configuración...");
                match self.insert_logger_config_legacy(config_json).await {
                    Ok(_) => {
                        info!("✅ Configuración guardada usando método alternativo");
                        Ok(())
                    },
                    Err(legacy_err) => {
                        error!("❌ Error en método alternativo: {}", legacy_err);
                        Err(legacy_err)
                    }
                }
            }
        }
    }

    /// Alternativa: guarda configs dentro de `flight_logs` con flight_id='__config__'
    pub async fn insert_logger_config_legacy(&self, config_json: &str) -> Result<()> {
        //info!("🔄 Intentando guardar configuración usando método alternativo");
    
        let client = self.inner.lock().await;
        let client = match client.as_ref() {
            Some(c) => c,
            None => return Err(anyhow::anyhow!("❌ No hay conexión a QuestDB")),
        };
    
        // Guarda como “evento” en flight_logs
        let q = r#"
            INSERT INTO flight_logs (ts, flight_id, payload)
            VALUES (now(), $1, $2)
        "#;
    
        match client.execute(q, &[&"__config__", &config_json]).await {
            Ok(_rows) => {
                //info!("✅ Config (legacy) guardada en flight_logs (filas: {})", rows);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!(format!(
                "❌ Error legacy flight_logs: {}",
                e
            ))),
        }
    }    

    pub async fn list_flights(&self, limit: i64) -> Result<Vec<(String, DateTime<Utc>)>> {

        let tn = self.table_name.to_string();
        let tc = self.time_col.to_string();
        
        self.with_connection(move |client| {
            let tn = tn.clone();
            let tc = tc.clone();
        
            Box::pin(async move {
                // 1) ¿Existe la tabla?
                let table_exists = match client
                    .query(
                        "SELECT table_name FROM tables() WHERE lower(table_name) = lower($1)",
                        &[&tn],
                    )
                    .await
                {
                    Ok(rows) => !rows.is_empty(),
                    Err(e) => {
                        error!("Failed to check if table exists: {}", e);
                        return Err(anyhow!("Failed to verify table existence: {}", e));
                    }
                };
        
                if !table_exists {
                    error!("Table '{}' does not exist in the database", tn);
                    return Ok(Vec::new());
                }
        
                // 2) ¿Existe la columna de tiempo?
                let time_col_exists = match client
                    .query(
                        "SELECT column_name
                           FROM information_schema.columns
                          WHERE lower(table_name) = lower($1)
                            AND lower(column_name) = lower($2)",
                        &[&tn, &tc],
                    )
                    .await
                {
                    Ok(rows) => !rows.is_empty(),
                    Err(e) => {
                        error!("Failed to check if time column exists: {}", e);
                        return Err(anyhow!("Failed to verify time column existence: {}", e));
                    }
                };
        
                if !time_col_exists {
                    error!("Time column '{}' does not exist in table '{}'", tc, tn);
                    return Err(anyhow!("Time column '{}' not found in table '{}'", tc, tn));
                }
        
                // 3) Trae últimos vuelos
                let query = format!(
                    r#"
                    SELECT "flight_id", max("{tc}") AS last_ts
                      FROM "{tn}"
                     GROUP BY "flight_id"
                     ORDER BY last_ts DESC
                     LIMIT $1
                    "#,
                    tc = tc, tn = tn
                );
        
                let rows = client.query(&query, &[&limit]).await?;
                let mut flights = Vec::new();
                for row in rows {
                    let flight_id: String = match row.try_get(0) {
                        Ok(id) => id,
                        Err(e) => { error!("Failed to get flight_id: {}", e); continue; }
                    };
                    let last_ts: chrono::NaiveDateTime = match row.try_get(1) {
                        Ok(dt) => dt,
                        Err(e) => { error!("Failed to parse timestamp for flight {}: {}", &flight_id, e); continue; }
                    };
                    let last_ts_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(last_ts, Utc);
                    flights.push((flight_id, last_ts_utc));
                }
        
                Ok(flights)
            })
        }).await
    }
    
    pub async fn fetch_flight_points(
        &self,
        flight_id: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FlightPoint>> {
        let tn = self.table_name.to_string();
        let tc = self.time_col.to_string();
        let fid = flight_id.to_string();
        let from = from.map(|d| d.naive_utc());
        let to   = to.map(|d| d.naive_utc());
    
        self.with_connection(move |client| {
            let tn = tn.clone();
            let tc = tc.clone();
            let fid = fid.clone();
            Box::pin(async move {
                // 1) Descubre columnas presentes
                let present: std::collections::HashSet<String> = {
                    let rows = client.query(
                        "SELECT column_name FROM information_schema.columns WHERE table_name = $1",
                        &[&tn],
                    ).await?;
                    let mut set = std::collections::HashSet::new();
                    for r in rows {
                        let name: String = r.get(0);
                        set.insert(name);
                    }
                    set
                };
    
                // 2) Construye SELECT dinámico
                let mut sel: Vec<String> = vec![format!(r#""{}""#, tc)];
                let push_if = |sel: &mut Vec<String>, col: &str, present: &std::collections::HashSet<String>| {
                    if present.contains(col) { sel.push(format!(r#""{}""#, col)); }
                };
    
                for col in [
                    "flight_id","schema_version","mode",
                    "AngleRoll","AnglePitch","Yaw",
                    "RateRoll","RatePitch","RateYaw",
                    "DesiredAngleRoll","DesiredAnglePitch","DesiredRateYaw",
                    "AccX","AccY","AccZ",
                    "g1","g2","k1","k2","m1","m2",
                    "InputThrottle","InputRoll","InputPitch","InputYaw",
                    "MotorInput1","MotorInput2","MotorInput3","MotorInput4",
                    "error_phi","error_theta","ErrorYaw",
                    "Altura","tau_x","tau_y","tau_z","Kc","Ki",
                    "phi_ref","theta_ref","KalmanAngleRoll","KalmanAnglePitch",
                    "Motor1","Motor2","Motor3","Motor4",
                ] {
                    push_if(&mut sel, col, &present);
                }
    
                // Gyros con alias si no existen *dps*
                if present.contains("GyroXdps")      { sel.push(r#""GyroXdps""#.into()); }
                else if present.contains("GyroX")    { sel.push(r#""GyroX" AS "GyroXdps""#.into()); }
                if present.contains("GyroYdps")      { sel.push(r#""GyroYdps""#.into()); }
                else if present.contains("GyroY")    { sel.push(r#""GyroY" AS "GyroYdps""#.into()); }
                if present.contains("GyroZdps")      { sel.push(r#""GyroZdps""#.into()); }
                else if present.contains("GyroZ")    { sel.push(r#""GyroZ" AS "GyroZdps""#.into()); }
    
                let mut q = format!(r#"SELECT {} FROM "{}" WHERE flight_id = $1"#, sel.join(", "), tn);
    
                let mut params: Vec<Box<dyn ToSql + Send + Sync>> = vec![Box::new(fid)];
                let mut idx = 2;
                if let Some(f) = from { q.push_str(&format!(r#" AND "{}" >= ${}"#, tc, idx)); params.push(Box::new(f)); idx+=1; }
                if let Some(t) = to   { q.push_str(&format!(r#" AND "{}" <= ${}"#, tc, idx)); params.push(Box::new(t)); idx+=1; }
    
                q.push_str(&format!(r#" ORDER BY "{}" LIMIT ${}"#, tc, idx));
                params.push(Box::new(limit));
    
                let param_refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| &**p as _).collect();
                let rows = client.query(&q, &param_refs[..]).await?;
    
                let mut out = Vec::with_capacity(rows.len());
                for r in rows {
                    let ts_naive: chrono::NaiveDateTime = r.try_get(tc.as_str())?;
                    let ts = chrono::DateTime::<Utc>::from_naive_utc_and_offset(ts_naive, Utc);
    
                    let mut payload = serde_json::Map::new();
                    macro_rules! put { ($name:expr, $ty:ty) => {
                        if let Ok(v) = r.try_get::<_, $ty>($name) {
                            payload.insert($name.to_string(), serde_json::json!(v));
                        }
                    }}
                    put!("flight_id", String); put!("schema_version", String); put!("mode", String);
                    put!("AngleRoll", f64); put!("AnglePitch", f64); put!("Yaw", f64);
                    put!("DesiredAngleRoll", f64); put!("DesiredAnglePitch", f64); put!("DesiredRateYaw", f64);
                    put!("AccX", f64); put!("AccY", f64); put!("AccZ", f64);
                    put!("g1", f64); put!("g2", f64); put!("k1", f64); put!("k2", f64);
                    put!("m1", f64); put!("m2", f64);
                    put!("RateRoll", f64); put!("RatePitch", f64); put!("RateYaw", f64);
                    put!("GyroXdps", f64); put!("GyroYdps", f64); put!("GyroZdps", f64);
                    put!("MotorInput1", f64); put!("MotorInput2", f64); put!("MotorInput3", f64); put!("MotorInput4", f64);
                    put!("InputThrottle", i64); put!("InputRoll", i64); put!("InputPitch", i64); put!("InputYaw", i64);
                    put!("error_phi", f64); put!("error_theta", f64); put!("ErrorYaw", f64);
                    put!("Altura", f64); put!("tau_x", f64); put!("tau_y", f64); put!("tau_z", f64);
                    put!("Kc", f64); put!("Ki", f64);
                    put!("phi_ref", f64); put!("theta_ref", f64);
                    put!("KalmanAngleRoll", f64); put!("KalmanAnglePitch", f64);
                    put!("Motor1", f64); put!("Motor2", f64); put!("Motor3", f64); put!("Motor4", f64);
    
                    out.push(FlightPoint { ts, payload: serde_json::Value::Object(payload) });
                }
                Ok(out)
            })
        }).await
    } 

}
    
#[derive(Clone, Debug)]
pub struct OptionalDb {
    pub inner: Arc<Mutex<Option<QuestDb>>>,
    pub config: QuestDbConfig,
}

impl QuestDb {

    pub async fn save_experiment_metadata(&self, metadata: &ExperimentMetadata) -> Result<()> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
        
        // Convertir correctamente los tipos para QuestDB
        let start_time = metadata.start_time.naive_utc();
        
        // end_time: si es None, usar NULL en SQL (pasamos un Option)
        let end_time = metadata.end_time.map(|t| t.naive_utc());
        
        // duration_seconds: si es None, usar NULL
        let duration_seconds = metadata.duration_seconds.map(|d| d as f64);
        
        let sampling_rate_hz = metadata.sampling_rate_hz as i64;
        let esp32_loop_hz = metadata.esp32_loop_hz as i64;
        let experiment_type = metadata.experiment_type.to_string();
        
        // Kalman gains: convertir a Option<f64>
        let k1 = metadata.kalman_gains.as_ref().map(|g| g.k1 as f64);
        let k2 = metadata.kalman_gains.as_ref().map(|g| g.k2 as f64);
        let k3 = metadata.kalman_gains.as_ref().map(|g| g.k3 as f64);
        let g1 = metadata.kalman_gains.as_ref().map(|g| g.g1 as f64);
        let g2 = metadata.kalman_gains.as_ref().map(|g| g.g2 as f64);
        let g3 = metadata.kalman_gains.as_ref().map(|g| g.g3 as f64);
        let m1 = metadata.kalman_gains.as_ref().map(|g| g.m1 as f64);
        let m2 = metadata.kalman_gains.as_ref().map(|g| g.m2 as f64);
        let m3 = metadata.kalman_gains.as_ref().map(|g| g.m3 as f64);
        
        let query = r#"
            INSERT INTO experiment_metadata (
                flight_id, experiment_id, start_time, end_time, duration_seconds,
                sampling_rate_hz, esp32_loop_hz, filter_type, experiment_type,
                kalman_k1, kalman_k2, kalman_k3, kalman_g1, kalman_g2, kalman_g3,
                kalman_m1, kalman_m2, kalman_m3, description, location, notes, created_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9,
                $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, now()
            )
        "#;
        
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
            &metadata.flight_id,
            &metadata.experiment_id,
            &start_time,
            &end_time,              // Option<NaiveDateTime> funciona con NULL
            &duration_seconds,      // Option<f64> funciona con NULL
            &sampling_rate_hz,
            &esp32_loop_hz,
            &metadata.filter_type,
            &experiment_type,
            &k1, &k2, &k3,
            &g1, &g2, &g3,
            &m1, &m2, &m3,
            &metadata.description,
            &metadata.location,
            &metadata.notes,
        ];
        
        client.execute(query, params).await?;
        
        info!("✅ Metadatos guardados para flight_id: {}", metadata.flight_id);
        Ok(())
    }
    
    /// Actualiza end_time y duración al finalizar vuelo
    pub async fn end_experiment(&self, flight_id: &str, end_time: DateTime<Utc>) -> Result<()> {
        let client = self.inner.lock().await;
        let client = client.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected to QuestDB"))?;
        
        let end_time_naive = end_time.naive_utc();
        
        let query = r#"
            UPDATE experiment_metadata 
            SET end_time = $2, duration_seconds = extract(epoch from ($2 - start_time))
            WHERE flight_id = $1 AND end_time IS NULL
        "#;
        
        let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[&flight_id, &end_time_naive];
        client.execute(query, params).await?;
        Ok(())
    }
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
    
    pub async fn save_experiment_metadata(&self, metadata: &ExperimentMetadata) -> Result<(), String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref()
            .unwrap()
            .save_experiment_metadata(metadata)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn end_experiment(&self, flight_id: &str, end_time: DateTime<Utc>) -> Result<(), String> {
        self.ensure_connected().await?;
        let db = self.inner.lock().await;
        db.as_ref()
            .unwrap()
            .end_experiment(flight_id, end_time)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn list_available_fields(&self) -> Result<Vec<String>, String> {
        // For now, return a default set of fields
        // In a real implementation, this would query the database schema
        Ok(vec![
            "timestamp".to_string(),
            "latitude".to_string(),
            "longitude".to_string(),
            "altitude".to_string(),
            "speed".to_string(),
            "battery".to_string(),
            "rssi".to_string(),
            "voltage".to_string(),
            "current".to_string(),
            "temperature".to_string(),
        ])
    }
}
