//! Tool "telefono": Liara PREPARA una chiamata o un SMS e li PASSA all'app di sistema (hand-off via
//! Intent Android). Nessun permesso pericoloso, nessun invio silenzioso: l'utente conferma nell'app
//! telefono/SMS. Su desktop questi tool non sono operativi (rispondono con una nota).
//!
//! ⚠️ CONTRATTO congelato per il dataset (come i peer): nomi/argomenti stabili. Entrambi SENSIBILI
//! → consenso prima di agire.
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("manca '{key}'"))
}

/// Avvia il dialer col numero già inserito (ACTION_DIAL). L'utente preme "chiama" nell'app telefono.
pub struct PhoneCall;
impl Tool for PhoneCall {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "phone_call".into(),
            description: "Apre l'app telefono con un numero già composto, pronto da chiamare. Usalo quando \
l'utente vuole chiamare qualcuno: prepari la chiamata e passi il controllo all'app telefono (l'utente conferma)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "number": { "type": "string", "description": "Numero di telefono da comporre (con prefisso se serve)" }
                },
                "required": ["number"]
            }),
        }
    }
    fn sensitive(&self) -> bool { true }
    fn consent_action(&self, args: &Value) -> String {
        format!("aprire il telefono per chiamare {}", arg_str(args, "number").unwrap_or("?"))
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let number = arg_str(args, "number")?;
        #[cfg(target_os = "android")]
        {
            android::dial(number)?;
            Ok(format!("Ho aperto l'app telefono con il numero {number}: premi chiama per avviare la chiamata."))
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = number;
            Ok("Le chiamate sono disponibili solo sull'app Android di Liara.".into())
        }
    }
}

/// Apre l'app SMS con destinatario e testo già compilati (ACTION_SENDTO). L'utente preme "invia".
pub struct SmsSend;
impl Tool for SmsSend {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "sms_send".into(),
            description: "Apre l'app SMS con destinatario e messaggio già scritti, pronti da inviare. Usalo \
quando l'utente vuole mandare un SMS: prepari il messaggio e passi il controllo all'app (l'utente conferma l'invio)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "number": { "type": "string", "description": "Numero del destinatario" },
                    "text": { "type": "string", "description": "Testo del messaggio" }
                },
                "required": ["number", "text"]
            }),
        }
    }
    fn sensitive(&self) -> bool { true }
    fn consent_action(&self, args: &Value) -> String {
        format!("aprire l'app SMS per scrivere a {}", arg_str(args, "number").unwrap_or("?"))
    }
    fn execute(&self, args: &Value) -> Result<String> {
        let number = arg_str(args, "number")?;
        let text = arg_str(args, "text")?;
        #[cfg(target_os = "android")]
        {
            android::sms(number, text)?;
            Ok(format!("Ho aperto l'app SMS con il messaggio per {number}: premi invia per spedirlo."))
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = (number, text);
            Ok("L'invio di SMS è disponibile solo sull'app Android di Liara.".into())
        }
    }
}

/// JNI verso `com.liara.app.PhoneBridge` (stesso pattern di core/crypto/android_keystore.rs).
#[cfg(target_os = "android")]
mod android {
    use anyhow::{anyhow, Context, Result};
    use jni::objects::{JObject, JString, JValue};

    const BRIDGE_CLASS: &str = "com.liara.app.PhoneBridge";

    pub(super) fn dial(number: &str) -> Result<()> {
        call(
            "dial",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[Arg::Str(number)],
        )
    }

    pub(super) fn sms(number: &str, body: &str) -> Result<()> {
        call(
            "sms",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
            &[Arg::Str(number), Arg::Str(body)],
        )
    }

    enum Arg<'a> {
        Str(&'a str),
    }

    fn call(method: &str, sig: &str, extra: &[Arg]) -> Result<()> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.context("JavaVM")?;
        let mut env = vm.attach_current_thread().context("attach thread JNI")?;

        // class loader dell'app (FindClass diretto da thread nativo non vede le classi dell'app)
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
            .and_then(|v| v.l())
            .context("getClassLoader")?;
        let class_name: JString = env.new_string(BRIDGE_CLASS).context("class name")?;
        let bridge = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&class_name)],
            )
            .and_then(|v| v.l())
            .map_err(|e| anyhow!("loadClass {BRIDGE_CLASS}: {e}"))?;

        // costruisci gli argomenti JString e il vettore di JValue (context per primo)
        let jstrings: Vec<JString> = extra
            .iter()
            .map(|a| match a {
                Arg::Str(s) => env.new_string(s).context("arg string"),
            })
            .collect::<Result<_>>()?;
        let mut jvalues: Vec<JValue> = Vec::with_capacity(1 + jstrings.len());
        jvalues.push(JValue::Object(&context));
        for js in &jstrings {
            jvalues.push(JValue::Object(js));
        }

        let res = env.call_static_method(jni::objects::JClass::from(bridge), method, sig, &jvalues);
        // un'eccezione Java pendente va PULITA o le chiamate JNI successive abortiscono
        if env.exception_check().unwrap_or(false) {
            let exc = env.exception_occurred().ok();
            let _ = env.exception_clear();
            let desc = exc
                .and_then(|e| env.call_method(&e, "toString", "()Ljava/lang/String;", &[]).ok())
                .and_then(|v| v.l().ok())
                .and_then(|o| {
                    let js = JString::from(o);
                    env.get_string(&js).ok().map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "eccezione Java non descrivibile".into());
            return Err(anyhow!("PhoneBridge.{method}: {desc}"));
        }
        res.map(|_| ()).map_err(|e| anyhow!("PhoneBridge.{method}: {e}"))
    }
}
