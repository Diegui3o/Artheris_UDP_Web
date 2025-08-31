use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::{extract::State, Json, http::StatusCode};
use crate::ws_server::http_server::AppState;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};
use tokio_postgres::types::ToSql;
use tracing::{error, info, trace, warn};

use crate::ws_server::ilp::{IlpHttp, choose_timestamp_ns};

#[derive(Clone, Debug)]
pub struct QuestDb {
    inner: Arc<Mutex<Option<Client>>>,
    ilp: Arc<IlpHttp>,
    table_name: Arc<String>,
    time_col: Arc<String>,
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
    let (table, tcol, rows) = {
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

    info!("probe_sql_insert ok: table={} time_col={} rows={}", table, tcol, rows);
    Ok(Json(ProbeResp { ok: true, rows }))
}

impl QuestDb {
    pub async fn connect(cfg: QuestDbConfig) -> Result<Self> {
        info!("🔌 Conectando a QuestDB en {}:{}", cfg.host, cfg.port);

        let connection_string = format!(
            "host={} port={} user={} password={} dbname={}",
            cfg.host, cfg.port, cfg.user, cfg.password, cfg.database
        );
        // Spawn connection in the background
        let (client, connection) = tokio_postgres::connect(&connection_string, NoTls)
            .await
            .map_err(|e| {
                error!("❌ No se pudo conectar a QuestDB: {}", e);
                anyhow!("Failed to connect to QuestDB: {}", e)
            })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("❌ Error de conexión con QuestDB: {}", e);
            }
        });

        let table_name = Arc::new(cfg.table_name.unwrap_or_else(|| "flight_telemetry".to_string()));
        let time_col = Arc::new(cfg.time_col.unwrap_or_else(|| "timestamp".to_string()));

        let ilp = Arc::new(IlpHttp::new(
            std::env::var("QDB_ILP_URL").unwrap_or_else(|_| "http://127.0.0.1:9000".into()),
            &*table_name,
        ));
        info!("🔭 ILP apuntando a {}", ilp.url);

        let questdb = Self {
            inner: Arc::new(Mutex::new(Some(client))),
            ilp,
            table_name,
            time_col,
        };

        questdb.ensure_schema().await.map_err(|e| {
            error!("❌ Error al crear el esquema: {}", e);
            anyhow!("Failed to create database schema: {}", e)
        })?;

        info!("✅ Conexión a QuestDB (PG) lista");
        Ok(questdb)
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
                        // ya tenías:
                        "AngleYaw" => ("Yaw", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "gyroRatePitch" => ("RatePitch", serde_json::json!(v.as_f64().unwrap_or(0.0))),
                        "gyroRateRoll"  => ("RateRoll",  serde_json::json!(v.as_f64().unwrap_or(0.0))),
                    
                        // motores → float
                        "MotorInput1" | "MotorInput2" | "MotorInput3" | "MotorInput4" => {
                            (key, serde_json::json!(v.as_f64().unwrap_or(0.0)))
                        }
                    
                        // NUEVO: errores → float
                        "error_phi" | "error_theta" => {
                            (key, serde_json::json!(v.as_f64().unwrap_or(0.0)))
                        }
                    
                        // inputs discretos → entero
                        "InputThrottle" | "InputRoll" | "InputPitch" | "InputYaw" => {
                            let ival = if let Some(i) = v.as_i64() { i } else { v.as_f64().unwrap_or(0.0).round() as i64 };
                            (key, serde_json::json!(ival))
                        }
                    
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
        flight_id SYMBOL,
        schema_version SYMBOL,
        mode SYMBOL,
    
        AngleRoll DOUBLE, AnglePitch DOUBLE, Yaw DOUBLE,
        RateRoll DOUBLE, RatePitch DOUBLE, RateYaw DOUBLE,
    
        GyroXdps DOUBLE, GyroYdps DOUBLE, GyroZdps DOUBLE,
    
        -- Controles discretos (enteros)
        InputThrottle LONG, InputRoll LONG, InputPitch LONG, InputYaw LONG,
    
        -- Señales de motor en DOUBLE (pueden venir con decimales)
        MotorInput1 DOUBLE, MotorInput2 DOUBLE, MotorInput3 DOUBLE, MotorInput4 DOUBLE,
    
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
        // Log the configuration being saved
        info!("💾 Intentando guardar configuración: {}", config_json);
        
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
            Ok(rows) => {
                info!("✅ Configuración guardada exitosamente en QuestDB (filas afectadas: {})", rows);
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
        info!("🔄 Intentando guardar configuración usando método alternativo");
    
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
            Ok(rows) => {
                info!("✅ Config (legacy) guardada en flight_logs (filas: {})", rows);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!(format!(
                "❌ Error legacy flight_logs: {}",
                e
            ))),
        }
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
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            let fid: String = r.get(0);
            let last_naive: chrono::NaiveDateTime = r.get(1);              // ← TIMESTAMP -> NaiveDateTime
            let last_ts = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(last_naive, chrono::Utc);
            items.push((fid, last_ts));
        }
        Ok(items)
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
    
            // Descubre columnas presentes
            let present = self.existing_columns(client).await?;
    
            // Utilidad para añadir columna sólo si existe
            let mut sel: Vec<String> = Vec::new();
            let quoted_ts = format!(r#""{}""#, self.time_col);
            sel.push(quoted_ts.clone()); // siempre el timestamp
    
            let push_if = |sel: &mut Vec<String>, col: &str| {
                if present.contains(col) {
                    sel.push(format!(r#""{}""#, col));
                }
            };
    
            // Campos seguros/comunes
            for col in [
                "flight_id","schema_version","mode",
                "AngleRoll","AnglePitch","Yaw",
                "RateRoll","RatePitch","RateYaw",
                "InputThrottle","InputRoll","InputPitch","InputYaw",
                "MotorInput1","MotorInput2","MotorInput3","MotorInput4",
                "error_phi","error_theta","ErrorYaw",
                "Altura","tau_x","tau_y","tau_z","Kc","Ki",
            ] {
                push_if(&mut sel, col);
            }
    
            // Gyros: intenta dps; si no, alias desde GyroX/Y/Z
            let alias_or_push = |sel: &mut Vec<String>, want: &str, legacy: &str| {
                if present.contains(want) {
                    sel.push(format!(r#""{}""#, want));
                } else if present.contains(legacy) {
                    // alias con comillas para conservar el nombre de salida
                    sel.push(format!(r#""{}" AS "{}""#, legacy, want));
                }
            };
            alias_or_push(&mut sel, "GyroXdps", "GyroX");
            alias_or_push(&mut sel, "GyroYdps", "GyroY");
            alias_or_push(&mut sel, "GyroZdps", "GyroZ");
    
            // Monta el SELECT final
            let mut q = format!(
                r#"SELECT {} FROM "{}" WHERE flight_id = $1"#,
                sel.join(", "),
                self.table_name
            );
    
            // Parámetros (usa NaiveDateTime para timestamp)
            let mut params: Vec<Box<dyn ToSql + Send + Sync>> = vec![Box::new(flight_id.to_string())];
            let mut idx = 2;
            if let Some(f) = from {
                q.push_str(&format!(r#" AND "{}" >= ${}"#, self.time_col, idx));
                params.push(Box::new(f.naive_utc()) as _);
                idx += 1;
            }
            if let Some(t) = to {
                q.push_str(&format!(r#" AND "{}" <= ${}"#, self.time_col, idx));
                params.push(Box::new(t.naive_utc()) as _);
                idx += 1;
            }
            q.push_str(&format!(r#" ORDER BY "{}" LIMIT ${}"#, self.time_col, idx));
            params.push(Box::new(limit) as _);
    
            let param_refs: Vec<&(dyn ToSql + Sync + 'static)> = params.iter().map(|p| &**p as &(dyn ToSql + Sync)).collect();
            let rows = client.query(&q, &param_refs[..]).await?;
    
            // Arma la salida
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                // TIMESTAMP -> NaiveDateTime -> Utc
                let ts_naive: chrono::NaiveDateTime = r.try_get(self.time_col.as_str())?;
                let ts = chrono::DateTime::<Utc>::from_naive_utc_and_offset(ts_naive, Utc);
    
                let mut payload = serde_json::Map::new();
    
                macro_rules! put {
                    ($name:expr, $ty:ty) => {
                        if let Ok(v) = r.try_get::<_, $ty>($name) {
                            payload.insert($name.to_string(), serde_json::json!(v));
                        }
                    };
                }
    
                // Los que quizá estén
                put!("flight_id", String);
                put!("schema_version", String);
                put!("mode", String);
    
                put!("AngleRoll", f64);
                put!("AnglePitch", f64);
                put!("Yaw", f64);
    
                put!("RateRoll", f64);
                put!("RatePitch", f64);
                put!("RateYaw", f64);
    
                // Gyros normalizados bajo *el mismo nombre de salida*
                put!("GyroXdps", f64);
                put!("GyroYdps", f64);
                put!("GyroZdps", f64);
    
                // Tipos: tu DDL actual pone DOUBLE para motores → f64
                put!("MotorInput1", f64);
                put!("MotorInput2", f64);
                put!("MotorInput3", f64);
                put!("MotorInput4", f64);
    
                put!("InputThrottle", i64);
                put!("InputRoll", i64);
                put!("InputPitch", i64);
                put!("InputYaw", i64);
    
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

#[derive(Clone, Debug)]
pub struct OptionalDb {
    pub inner: Arc<Mutex<Option<QuestDb>>>,
    pub config: QuestDbConfig,
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
