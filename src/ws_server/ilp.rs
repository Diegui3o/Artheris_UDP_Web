use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Client;
use serde_json::{Map, Value};
use tracing::{debug, error, info};

#[derive(Debug)]
pub struct IlpHttp {
    client: Client,
    pub url: String,
    pub table: String,
}

impl IlpHttp {
    /// Normaliza `base_url` para que SIEMPRE use /imp y precision=ns.
    /// Si llega a venir "/exec", lo corrige.
    pub fn new(base_url: String, table: &str) -> Self {
        let mut url = base_url;

        // si viene .../exec, lo quitamos
        if url.ends_with("/exec") {
            url.truncate(url.len() - "/exec".len());
        }

        // aseguramos /imp
        if !url.ends_with("/imp") {
            if url.ends_with('/') {
                url.push_str("imp");
            } else {
                url.push_str("/imp");
            }
        }

        // aseguramos precision=ns
        if !url.contains("precision=") {
            if url.contains('?') {
                url.push_str("&precision=ns");
            } else {
                url.push_str("?precision=ns");
            }
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");

        info!("🔭 ILP HTTP endpoint = {}", url);

        Self { client, url, table: table.to_string() }
    }

    fn esc_tag(s: &str) -> String {
        s.replace(',', "\\,").replace(' ', "\\ ").replace('=', "\\=")
    }

    fn esc_str(s: &str) -> String {
        s.replace('"', "\\\"")
    }

    /// Convierte un objeto JSON (solo numéricos/strings/bools) a 1 línea ILP
    pub fn json_to_line(
        &self,
        tags: &BTreeMap<String, String>,
        fields: &Map<String, Value>,
        ts_ns: Option<i64>,
    ) -> Option<String> {
        let mut head = IlpHttp::esc_tag(&self.table);
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

        let mut field_pairs = Vec::new();
        for (k, v) in fields {
            match v {
                Value::Null => {}
                Value::Bool(b) => field_pairs.push(format!("{}={}", k, if *b { "t" } else { "f" })),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        field_pairs.push(format!("{}={}i", k, i));
                    } else if let Some(f) = n.as_f64() {
                        if f.is_finite() {
                            field_pairs.push(format!("{}={}", k, f));
                        }
                    }
                }
                Value::String(s) => field_pairs.push(format!("{}=\"{}\"", k, IlpHttp::esc_str(s))),
                _ => field_pairs.push(format!("{}=\"{}\"", k, IlpHttp::esc_str(&v.to_string()))),
            }
        }

        if field_pairs.is_empty() {
            return None;
        }

        let mut line = format!("{} {}", head, field_pairs.join(","));
        if let Some(ns) = ts_ns {
            line.push(' ');
            line.push_str(&ns.to_string());
        }
        Some(line)
    }

    /// Envía las líneas a /imp
    pub async fn write_lines(&self, lines: &[String]) -> anyhow::Result<()> {
        let body = lines.join("\n");
        debug!("ilp_http -> POST {} (lines={}, bytes={})", self.url, lines.len(), body.len());

        // Sanity: si por alguna razón no contiene /imp, lo vemos en logs
        if !self.url.contains("/imp") {
            error!("❌ ILP URL NO contiene /imp: {}", self.url);
        }

        let resp = self.client
            .post(&self.url)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            error!("ilp_write_fail: url={} status={} body={}", self.url, status, text);
            anyhow::bail!("ILP write failed: url={} status={} body={}", self.url, status, text);
        }

        info!("ilp_write_ok: url={} lines={}", self.url, lines.len());
        Ok(())
    }
}

/// ISO8601 → ns, o now() → ns si None
pub fn choose_timestamp_ns(iso_or_none: Option<&str>) -> i64 {
    use chrono::{DateTime, Utc};
    match iso_or_none {
        Some(s) => {
            let dt = DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            dt.timestamp_nanos_opt().unwrap_or_else(|| dt.timestamp_micros() * 1000)
        }
        None => {
            let now = Utc::now();
            now.timestamp_nanos_opt().unwrap_or_else(|| now.timestamp_micros() * 1000)
        }
    }
}
