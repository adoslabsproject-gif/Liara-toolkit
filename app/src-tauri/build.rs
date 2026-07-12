fn main() {
    // ANDROID + OpenCL: llama.cpp accelera sulla GPU Adreno via OpenCL → libapp_lib.so referenzia
    // clGetPlatformIDs & co. Va linkata la stub libOpenCL.so (vendor/opencl-lib) così il .so ottiene
    // `NEEDED libOpenCL.so` e a RUNTIME il loader risolve i simboli dalla libOpenCL VERA del telefono
    // (/vendor/lib64). Senza → java.lang.UnsatisfiedLinkError "cannot locate symbol clGetPlatformIDs"
    // → CRASH all'avvio (regressione v0.2.3: il -lOpenCL fu perso quando il .cargo/config venne riscritto
    // solo per il 16KB page-size). build.rs è ADDITIVO: NON viene sovrascritto dai RUSTFLAGS di Tauri.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        println!("cargo:rustc-link-search=native={dir}/vendor/opencl-lib");
        println!("cargo:rustc-link-lib=OpenCL");
        // 16 KB page-size (avviso Samsung/Android 15+ "non compatibile con pagine da 16 kB"): allinea i
        // segmenti PT_LOAD di libapp_lib.so a 16 KB. In .cargo/config.toml NON basta — i RUSTFLAGS di
        // Tauri lo sovrascrivono (STESSO motivo per cui -lOpenCL fu spostato qui). Additivo → funziona.
        // Retrocompatibile: l'APK gira sia sui telefoni a 4 KB sia su quelli a 16 KB.
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg=-Wl,-z,common-page-size=16384");
    }
    // WINDOWS: sherpa-onnx-c-api linka `cargs` ma il pacchetto -shared non include cargs.lib → LNK1181.
    // Il workflow compila cargs.lib in cargs-lib/ PRIMA del build; qui aggiungiamo solo il search path
    // (il -lcargs lo emette già sherpa-rs-sys). cargs è una libreria C open-source di un solo file.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        println!("cargo:rustc-link-search=native={dir}/cargs-lib");
    }
    tauri_build::build()
}
