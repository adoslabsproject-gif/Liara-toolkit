//! JNI verso `com.liara.app.KeystoreBridge` (review 2026-07-02 #3): wrapping
//! hardware-backed della chiave master via AndroidKeyStore.
//!
//! ⚠️ Trappola JNI nota: `FindClass` da un thread nativo usa il class loader di
//! sistema, che NON vede le classi dell'app → si passa dal class loader del
//! Context Android (ndk_context) con `loadClass`.
#![cfg(target_os = "android")]

use anyhow::{anyhow, Context, Result};
use jni::objects::{JByteArray, JObject, JString, JValue};

const BRIDGE_CLASS: &str = "com.liara.app.KeystoreBridge";

/// master key in chiaro → blob avvolto dal keystore hardware.
pub(super) fn wrap(plain: &[u8]) -> Result<Vec<u8>> {
    call_bridge("wrapKey", plain)
}

/// blob avvolto → master key in chiaro (fallisce se il blob non autentica
/// o se la chiave di wrapping non esiste più nel keystore).
pub(super) fn unwrap(blob: &[u8]) -> Result<Vec<u8>> {
    call_bridge("unwrapKey", blob)
}

fn call_bridge(method: &str, arg: &[u8]) -> Result<Vec<u8>> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.context("JavaVM")?;
    let mut env = vm.attach_current_thread().context("attach thread JNI")?;

    // class loader del Context dell'app (FindClass diretto qui fallirebbe)
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

    let jarr = env.byte_array_from_slice(arg).context("byte array in")?;
    let result = env.call_static_method(
        jni::objects::JClass::from(bridge),
        method,
        "([B)[B",
        &[JValue::Object(&jarr)],
    );
    // un'eccezione Java pendente va PULITA o ogni chiamata JNI successiva abortisce;
    // prima però ne catturiamo il toString() — "eccezione generica" non si debugga
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
        return Err(anyhow!("KeystoreBridge.{method}: {desc}"));
    }
    let out = result
        .and_then(|v| v.l())
        .map_err(|e| anyhow!("KeystoreBridge.{method}: {e}"))?;
    let bytes = env
        .convert_byte_array(JByteArray::from(out))
        .context("byte array out")?;
    Ok(bytes)
}
