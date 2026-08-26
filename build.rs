const COMMANDS: &[&str] = &["registrar", "identificador"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
