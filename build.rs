fn main() {
    slint_build::compile("ui/app.slint").expect("Could not compile the Slint interface");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("ProductName", "Tap")
            .set("FileDescription", "Tap Autoclicker")
            .compile()
            .expect("Could not embed Windows application resources");
    }
}
