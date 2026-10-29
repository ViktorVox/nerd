fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/nerd.ico");
        res.set_manifest_file("assets/nerd.manifest");
        res.set("ProductName", "Nerd");
        res.set("FileDescription", "Nerd - превращает устройство в графический планшет");
        res.compile().expect("не удалось встроить иконку и манифест");
    }
}
