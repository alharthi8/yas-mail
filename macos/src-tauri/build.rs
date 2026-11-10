fn main() {
    for name in [
        "YAS_MAIL_GOOGLE_CLIENT_ID",
        "YAS_MAIL_GOOGLE_CLIENT_SECRET",
        "YAS_MAIL_MICROSOFT_CLIENT_ID",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    tauri_build::build()
}
