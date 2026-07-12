//! Weather tool (Open-Meteo, no API key) and current-location setter.
use super::web::{http_agent, urlencode};
use crate::core::memory::Memory;
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

fn wmo_desc(code: i64) -> &'static str {
    match code {
        0 => "sereno",
        1 => "prevalentemente sereno",
        2 => "parzialmente nuvoloso",
        3 => "nuvoloso",
        45 | 48 => "nebbia",
        51 | 53 | 55 => "pioggerella",
        56 | 57 => "pioggerella gelata",
        61 | 63 | 65 => "pioggia",
        66 | 67 => "pioggia gelata",
        71 | 73 | 75 => "neve",
        77 => "nevischio",
        80 | 81 | 82 => "rovesci",
        85 | 86 => "rovesci di neve",
        95 => "temporale",
        96 | 99 => "temporale con grandine",
        _ => "condizioni variabili",
    }
}

fn get_json(url: &str) -> Result<Value> {
    // Agent coi root CA inclusi (webpki-roots), NON ureq::get nudo: su Android lo store CA di sistema
    // è inaffidabile → l'HTTPS di meteo/geocoding falliva. Stesso agent del web tool.
    let body = http_agent()
        .get(url)
        .set("User-Agent", "Liara/1.0")
        .timeout(std::time::Duration::from_secs(8))
        .call()
        .map_err(|e| anyhow!("richiesta meteo fallita: {e}"))?
        .into_string()
        .map_err(|e| anyhow!("lettura fallita: {e}"))?;
    serde_json::from_str(&body).map_err(|e| anyhow!("risposta non valida: {e}"))
}

/// Resolve a place name to (lat, lon, label) via Open-Meteo geocoding.
fn geocode(city: &str) -> Result<(f64, f64, String)> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=it&format=json",
        urlencode(city)
    );
    let j = get_json(&url)?;
    let r = j.get("results").and_then(|v| v.as_array()).and_then(|a| a.first())
        .ok_or_else(|| anyhow!("località \"{city}\" non trovata"))?;
    let lat = r.get("latitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("geocoding incompleto"))?;
    let lon = r.get("longitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("geocoding incompleto"))?;
    let name = r.get("name").and_then(|v| v.as_str()).unwrap_or(city);
    let country = r.get("country").and_then(|v| v.as_str()).unwrap_or("");
    Ok((lat, lon, format!("{name}{}", if country.is_empty() { String::new() } else { format!(", {country}") })))
}

/// Approximate device location from the public IP (city-level; GPS-free fallback).
/// PRIVACY: usa ipwho.is via HTTPS. Prima era `http://ip-api.com` in CHIARO → la geolocalizzazione
/// dell'utente viaggiava in cleartext, incoerente con un'app che si presenta "privata e locale"
/// (il free tier di ip-api NON supporta HTTPS; ipwho.is sì, senza chiave).
fn ip_locate() -> Result<(f64, f64, String)> {
    let j = get_json("https://ipwho.is/?fields=success,latitude,longitude,city,country")?;
    if j.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return Err(anyhow!("posizione non determinabile"));
    }
    let lat = j.get("latitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("posizione incompleta"))?;
    let lon = j.get("longitude").and_then(|v| v.as_f64()).ok_or_else(|| anyhow!("posizione incompleta"))?;
    let city = j.get("city").and_then(|v| v.as_str()).unwrap_or("la tua zona");
    let country = j.get("country").and_then(|v| v.as_str()).unwrap_or("");
    Ok((lat, lon, format!("{city}{}", if country.is_empty() { String::new() } else { format!(", {country}") })))
}

pub struct Weather {
    pub mem: Arc<Memory>,
}
impl Tool for Weather {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "weather".into(),
            description: "Meteo attuale. Se non indichi una località, usa la posizione corrente del dispositivo.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": { "type": "string", "description": "Città o luogo (opzionale)" },
                    "latitude": { "type": "number", "description": "Latitudine GPS (opzionale)" },
                    "longitude": { "type": "number", "description": "Longitudine GPS (opzionale)" }
                },
                "required": []
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let (lat, lon, place) = match (
            args.get("latitude").and_then(|v| v.as_f64()),
            args.get("longitude").and_then(|v| v.as_f64()),
        ) {
            (Some(la), Some(lo)) => (la, lo, "la tua posizione".to_string()),
            _ => match args.get("location").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
                Some(loc) => geocode(loc)?,
                // no explicit place → stored location (GPS/manual) first, IP as last resort
                None => match self.mem.location() {
                    Some((la, lo, label)) => (la, lo, label),
                    None => ip_locate()?,
                },
            },
        };
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
&current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&timezone=auto"
        );
        let j = get_json(&url)?;
        let c = j.get("current").ok_or_else(|| anyhow!("dati meteo assenti"))?;
        let temp = c.get("temperature_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let feels = c.get("apparent_temperature").and_then(|v| v.as_f64()).unwrap_or(temp);
        let hum = c.get("relative_humidity_2m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let wind = c.get("wind_speed_10m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let code = c.get("weather_code").and_then(|v| v.as_i64()).unwrap_or(-1);
        Ok(format!(
            "Meteo a {place}: {desc}, {temp:.0}°C (percepiti {feels:.0}°C), umidità {hum:.0}%, vento {wind:.0} km/h.",
            desc = wmo_desc(code)
        ))
    }
}

/// Lets Liara set the user's current location when they state it (e.g. "sono a Bologna").
/// Persists until a fresh GPS fix overrides it. Used by the weather/location tools.
pub struct SetLocation {
    pub mem: Arc<Memory>,
}
impl Tool for SetLocation {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "set_location".into(),
            description: "Imposta la posizione corrente dell'utente quando te la comunica (es. \"sono a Bologna\", \"la mia città è Modena\"). Resta finché non arriva un GPS aggiornato.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "location": { "type": "string", "description": "Città o luogo indicato dall'utente" } },
                "required": ["location"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let loc = args.get("location").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("manca 'location'"))?;
        let (lat, lon, label) = geocode(loc)?;
        self.mem.set_location(lat, lon, &label, "manual").map_err(|e| anyhow!("{e}"))?;
        Ok(format!("Posizione impostata su {label}. La userò finché non cambia."))
    }
}
