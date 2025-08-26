use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct IlpHttp {
    endpoint: String,
    measurement: String,
    http: Client,
}

impl IlpHttp {
    pub fn new(endpoint: impl Into<String>, measurement: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            measurement: measurement.into(),
            http: Client::new(),
        }
    }

    pub fn esc_tag(s: &str) -> String {
        s.replace(',', "\\,").replace(' ', "\\ ").replace('=', "\\=")
    }

    pub fn esc_str(s: &str) -> String {
        s.replace('"', "\\\"")
    }

    /// Convierte tags + fields a una línea ILP. `ts_ns` en nanosegundos (i64).
    pub fn json_to_line(
        &self,
        tags: &BTreeMap<String, String>,
        fields: &Map<String, Value>,
        ts_ns: Option<i64>,
    ) -> Option<String> {
        // measurement y tags
        let mut head = IlpHttp::esc_tag(&self.measurement);
        if !tags.is_empty() {
            let joined = tags
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{}={}", IlpHttp::esc_tag(k), IlpHttp::esc_tag(v)))
                .collect::<Vec<_>>()
                .join(",");
            head.push(',');
            head.push_str(&joined);
        }

        // fields
        let mut field_pairs = Vec::new();
        for (k, v) in fields {
            match v {
                Value::Null => {}
                Value::Bool(b) => field_pairs.push(format!("{k}={}", if *b { "t" } else { "f" })),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        // enteros con sufijo 'i'
                        field_pairs.push(format!("{k}={}i", i));
                    } else if let Some(f) = n.as_f64() {
                        if f.is_finite() {
                            field_pairs.push(format!("{k}={}", f));
                        }
                    }
                }
                Value::String(s) => field_pairs.push(format!("{k}=\"{}\"", IlpHttp::esc_str(s))),
                _ => field_pairs.push(format!("{k}=\"{}\"", IlpHttp::esc_str(&v.to_string()))),
            }
        }
        if field_pairs.is_empty() {
            return None;
        }

        // línea final
        let mut line = format!("{head} {}", field_pairs.join(","));
        if let Some(ns) = ts_ns {
            line.push(' ');
            line.push_str(&ns.to_string());
        }
        Some(line)
    }

    /// Envía un batch de líneas a /imp
    pub async fn write_lines(&self, lines: &[String]) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let body = lines.join("\n");
        let resp = self
            .http
            .post(&self.endpoint)
            .header("Content-Type", "text/plain")
            .body(body)
            .send()
            .await
            .context("POST /imp failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("QuestDB /imp error: {} - {}", status, text);
        }
        Ok(())
    }
}

/// Devuelve un timestamp en **nanosegundos (i64)**.
/// Si `maybe_iso` viene con ISO-8601, lo usa; si no, usa `Utc::now()`.
pub fn choose_timestamp_ns(maybe_iso: Option<&str>) -> i64 {
    if let Some(s) = maybe_iso {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            // Ambos caminos producen i64
            return dt
                .timestamp_nanos_opt()
                .unwrap_or_else(|| dt.timestamp_micros() * 1000);
        }
    }
    let now = Utc::now();
    now.timestamp_nanos_opt()
        .unwrap_or_else(|| now.timestamp_micros() * 1000)
}
