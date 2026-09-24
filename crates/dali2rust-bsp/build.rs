fn main() {
    println!("cargo::rustc-check-cfg=cfg(esp_idf_spiram_xip_from_psram)");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("espidf") {
        embuild::espidf::sysenv::output();
    }
}
