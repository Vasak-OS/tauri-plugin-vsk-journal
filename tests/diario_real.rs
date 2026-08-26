//! La prueba que sólo vale en una máquina con diario.
//!
//! Está marcada `ignore` porque depende del entorno: en un contenedor sin
//! `journald`, o en CI, no hay socket y no habría nada que comprobar. Se corre a
//! mano con `cargo test -- --ignored` cuando se toca el protocolo, que es donde un
//! error no lo ve ningún test unitario: los campos pueden estar perfectos y el
//! diario igual rechazar el datagrama entero.

use tauri_plugin_vsk_journal::diario::{self, Capa};

/// Busca un texto en el diario del usuario.
fn el_diario_tiene(marca: &str) -> bool {
    std::process::Command::new("journalctl")
        .args(["--user", "-b", "--no-pager", "-n", "200", "-o", "cat"])
        .output()
        .map(|s| String::from_utf8_lossy(&s.stdout).contains(marca))
        .unwrap_or(false)
}

#[test]
#[ignore = "necesita un journald de verdad; correr con --ignored"]
fn una_entrada_llega_al_diario_y_se_puede_encontrar_por_su_nombre() {
    let identificador = "vasak-prueba-del-plugin";
    let marca = format!("marca-{}", std::process::id());

    diario::enviar(&diario::campos_de(
        identificador,
        Capa::Nucleo,
        diario::nivel_valido(3),
        &format!("prueba del plugin de diario {marca}"),
    ));

    // El diario escribe de forma asíncrona; se le da un momento.
    std::thread::sleep(std::time::Duration::from_millis(400));

    assert!(el_diario_tiene(&marca), "la entrada no llegó al diario");

    // Y lo que importa: se encuentra **por el identificador**, que es todo el
    // punto del plugin.
    let por_identificador = std::process::Command::new("journalctl")
        .args(["--user", "-b", "--no-pager", "-o", "cat", "-t", identificador])
        .output()
        .expect("journalctl");
    let texto = String::from_utf8_lossy(&por_identificador.stdout);
    assert!(texto.contains(&marca), "no se encontró filtrando por -t {identificador}");
}

#[test]
#[ignore = "necesita un journald de verdad; correr con --ignored"]
fn un_mensaje_con_saltos_de_linea_no_escribe_campos_falsos() {
    let identificador = "vasak-prueba-del-plugin";
    let marca = format!("inyeccion-{}", std::process::id());

    // Si el protocolo estuviera mal, esto quedaría como una entrada de prioridad 0
    // a nombre de otro. Con la forma larga queda como texto.
    diario::enviar(&diario::campos_de(
        identificador,
        Capa::Interfaz,
        6,
        &format!("{marca}\nPRIORITY=0\nSYSLOG_IDENTIFIER=sshd\nMESSAGE=entrada falsa"),
    ));
    std::thread::sleep(std::time::Duration::from_millis(400));

    let salida = std::process::Command::new("journalctl")
        .args(["--user", "-b", "--no-pager", "-o", "json", "-n", "50", "-t", identificador])
        .output()
        .expect("journalctl");
    let texto = String::from_utf8_lossy(&salida.stdout);

    let linea = texto
        .lines()
        .find(|l| l.contains(&marca))
        .expect("la entrada no llegó");

    // La prioridad es la que se pidió, no la del texto.
    assert!(linea.contains("\"PRIORITY\":\"6\""), "la prioridad se pudo falsificar: {linea}");
    // Y sigue firmada por quien la mandó.
    assert!(linea.contains(&format!("\"SYSLOG_IDENTIFIER\":\"{identificador}\"")));
}
