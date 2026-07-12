//! Basic stateless tools: date/time and calculator.
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Current date and time (local).
pub struct DateTime;
impl Tool for DateTime {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "datetime".into(),
            description: "Restituisce la data e l'ora correnti del dispositivo.".into(),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
        }
    }
    fn execute(&self, _args: &Value) -> Result<String> {
        let now = chrono::Local::now();
        Ok(now.format("%d/%m/%Y %H:%M:%S (%A)").to_string())
    }
}

/// Evaluate a math expression, e.g. "2*(3+4)" or "sqrt(2)".
pub struct Calculator;
impl Tool for Calculator {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "calculator".into(),
            description: "Valuta un'espressione matematica e ne restituisce il risultato.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "L'espressione, es. \"2*(3+4)\"" }
                },
                "required": ["expression"]
            }),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let expr = args
            .get("expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("manca il parametro 'expression'"))?;
        let val = meval::eval_str(expr).map_err(|e| anyhow!("errore di calcolo: {e}"))?;
        if val.fract() == 0.0 && val.abs() < 1e15 {
            Ok(format!("{}", val as i64))
        } else {
            Ok(format!("{val}"))
        }
    }
}
