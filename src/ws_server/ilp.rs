use std::collections::BTreeMap;
use reqwest::Client;
use serde_json::{Map, Value};
use tracing::{debug, error, info};
use urlencoding;

#[derive(Debug)]
pub struct IlpHttp {
    client: Client,
    pub url: String,
    pub table: String,
}

impl IlpHttp {
    pub fn new(base_url: String, table: &str) -> Self {
        let mut url = base_url.trim_end_matches('/').to_string();

        // Asegura /write una sola vez
        if !(url.ends_with("/write") || url.contains("/write?") || url.contains("/write&")) {
            url.push_str("/write");
        }

        // Asegura precision=ns
        if !url.contains("precision=") {
            if url.contains('?') { url.push_str("&precision=ns"); }
            else { url.push_str("?precision=ns"); }
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("reqwest client");

        Self { client, url, table: table.to_string() }
    }

    fn esc_tag(s: &str) -> String {
        s.replace(',', "\\,").replace(' ', "\\ ").replace('=', "\\=")
    }

    fn esc_str(s: &str) -> String {
        s.replace('"', "\\\"")
    }

    /// Convierte objeto JSON (numéricos/strings/bools) a 1 línea ILP
    pub fn json_to_line(
        &self,
        tags: &BTreeMap<String, String>,
        fields: &Map<String, Value>,
        ts_ns: Option<i64>,
    ) -> Option<String> {
        // measurement y tags
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

        // fields
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

    /// Envía líneas a /imp como text/plain; loguea el body
    pub async fn write_lines(&self, lines: &[String]) -> anyhow::Result<()> {
        let body = lines.join("\n");
        debug!("ilp_http -> POST {} (lines={}, bytes={})", self.url, lines.len(), body.len());
    
        let resp = self.client
            .post(&self.url)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(body.clone())
            .send()
            .await?;
    
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!("ILP write failed: {}", text);
        }
    
        info!("ilp_write_ok: url={} lines={} body={}", self.url, lines.len(), text);
        
        let sql = "SELECT count(), to_str(min(timestamp)), to_str(max(timestamp)) \
                   FROM flight_telemetry \
                   WHERE timestamp > dateadd('m', -10, now())";
    
        // Construir base_url a partir de .../write?... -> .../
        let base_url = match self.url.find("/write") {
            Some(idx) => &self.url[..idx],
            None => &self.url, // fallback
        };
        let check_url = format!("{base}/exec?query={q}",
            base = base_url.trim_end_matches('/'),
            q = urlencoding::encode(sql)
        );
    
        // Usa el mismo client (respeta timeout)
        match self.client.get(&check_url).send().await {
            Ok(check_resp) => {
                let s = check_resp.status();
                let body = check_resp.text().await.unwrap_or_else(|_| "Failed to read response".into());
                info!("exec_verify flight_telemetry [{}]: {}", s, body);
            }
            Err(e) => {
                error!("Failed to verify write: {}", e);
            }
        }    
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
