//! Registro en el diario del sistema para las aplicaciones de VasakOS.
//!
//! # Por qué existe
//!
//! Una aplicación gráfica que se abre desde el menú es hija del compositor. Lo que
//! manda por `stderr` termina en el diario **atribuido a la unidad de la sesión**,
//! así que sus mensajes se mezclan con los del compositor y no se pueden encontrar
//! por nombre. Dos consecuencias concretas, las dos comprobadas:
//!
//! 1. El selector de aplicaciones de `vasak-monitor` lista el terminal, los ajustes
//!    y la galería, y para todos dice «sin entradas» — no porque no escriban, sino
//!    porque lo que escriben no lleva su nombre.
//! 2. Cuando `vasak-desktop` abortó, lo único que quedó fue un volcado de núcleo
//!    sin símbolos. El mensaje del panic se había escrito, a un `stderr` que no se
//!    podía leer después.
//!
//! # Cómo se usa
//!
//! ```text
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_vsk_journal::init())
//! ```
//!
//! Con eso queda el identificador puesto —el nombre del ejecutable— y el gancho de
//! pánico instalado. Desde Rust:
//!
//! ```text
//! use tauri_plugin_vsk_journal::DiarioExt;
//! app.diario().error("no se pudo abrir la configuración");
//! ```
//!
//! Desde la interfaz, con el paquete `@vasakgroup/plugin-vsk-journal`.

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

mod commands;
pub mod diario;
mod error;
mod panico;

pub use diario::Capa;
pub use error::{Error, Result};

/// Niveles de syslog, que son los que entiende el diario.
pub const EMERGENCIA: u8 = 0;
pub const CRITICO: u8 = 2;
pub const ERROR: u8 = 3;
pub const AVISO: u8 = 4;
pub const INFORMATIVO: u8 = 6;
pub const DEPURACION: u8 = 7;

/// Hasta dónde puede llegar un identificador.
///
/// `journald` no lo limita, pero un identificador larguísimo rompe cualquier
/// listado y no se puede escribir a mano en `journalctl -t`.
const LIMITE_IDENTIFICADOR: usize = 64;

/// El diario de esta aplicación.
pub struct Diario {
    identificador: String,
}

impl Diario {
    /// Con quién firma sus entradas.
    pub fn identificador(&self) -> &str {
        &self.identificador
    }

    /// Anota algo, con el nivel que se le pase.
    pub fn registrar(&self, capa: Capa, nivel: u8, mensaje: &str) {
        diario::enviar(&diario::campos_de(&self.identificador, capa, nivel, mensaje));
    }

    pub fn error(&self, mensaje: &str) {
        self.registrar(Capa::Nucleo, ERROR, mensaje);
    }

    pub fn aviso(&self, mensaje: &str) {
        self.registrar(Capa::Nucleo, AVISO, mensaje);
    }

    pub fn informacion(&self, mensaje: &str) {
        self.registrar(Capa::Nucleo, INFORMATIVO, mensaje);
    }

    pub fn depuracion(&self, mensaje: &str) {
        self.registrar(Capa::Nucleo, DEPURACION, mensaje);
    }
}

pub trait DiarioExt<R: Runtime> {
    fn diario(&self) -> &Diario;
}

impl<R: Runtime, T: Manager<R>> DiarioExt<R> for T {
    fn diario(&self) -> &Diario {
        self.state::<Diario>().inner()
    }
}

/// Si un identificador sirve para firmar entradas del diario.
///
/// Sin saltos de línea ni `=`: los dos son separadores del protocolo, y aunque la
/// codificación los neutraliza, un identificador con un salto adentro no se puede
/// buscar con `journalctl -t` — o sea, no serviría para nada.
pub fn identificador_valido(identificador: &str) -> bool {
    !identificador.is_empty()
        && identificador.len() <= LIMITE_IDENTIFICADOR
        && !identificador.contains(['\n', '=', '\0'])
}

/// El nombre del ejecutable, que es con lo que se firma por omisión.
///
/// Se elige éste y no el nombre del producto porque es el que coincide con lo que
/// el diario ya guarda en `_COMM`, y así una entrada nueva y una vieja de la misma
/// aplicación caen juntas al filtrar.
pub fn identificador_por_omision() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|ruta| ruta.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|nombre| identificador_valido(nombre))
        .unwrap_or_else(|| "vasak-app".to_string())
}

/// Arranca el plugin firmando con el nombre del ejecutable.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    armar(identificador_por_omision())
}

/// Arranca el plugin con un identificador propio.
///
/// Útil cuando el ejecutable no se llama como la aplicación. Un identificador
/// inválido no aborta el arranque: se cae al del ejecutable, porque una aplicación
/// que no abre por un problema de registro sería peor que una que registra con un
/// nombre distinto del pedido.
pub fn init_con_identificador<R: Runtime>(identificador: &str) -> TauriPlugin<R> {
    if identificador_valido(identificador) {
        armar(identificador.to_string())
    } else {
        armar(identificador_por_omision())
    }
}

fn armar<R: Runtime>(identificador: String) -> TauriPlugin<R> {
    // El gancho se instala al construir el plugin y no en `setup`: un panic dentro
    // del propio `setup` —leyendo la configuración, abriendo una ventana— es de los
    // más probables y de los que menos rastro dejan.
    panico::instalar(identificador.clone());

    Builder::new("vsk-journal")
        .invoke_handler(tauri::generate_handler![
            commands::registrar,
            commands::identificador
        ])
        .setup(move |app, _api| {
            app.manage(Diario {
                identificador: identificador.clone(),
            });
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_identificador_con_separadores_del_protocolo_no_pasa() {
        // No por la inyección —la codificación ya la neutraliza— sino porque un
        // identificador con un salto adentro no se puede buscar con `journalctl -t`.
        assert!(!identificador_valido("vasak\nterminal"));
        assert!(!identificador_valido("vasak=terminal"));
        assert!(!identificador_valido("vasak\0terminal"));
        assert!(!identificador_valido(""));
        assert!(!identificador_valido(&"a".repeat(LIMITE_IDENTIFICADOR + 1)));
        assert!(identificador_valido("vasak-terminal"));
        assert!(identificador_valido("vasak-file-manager"));
    }

    #[test]
    fn siempre_hay_un_identificador_con_el_que_firmar() {
        // Sin esto, una entrada quedaría sin identificador y volvería a ser
        // imposible de encontrar, que es justo lo que este plugin arregla.
        let por_omision = identificador_por_omision();
        assert!(!por_omision.is_empty());
        assert!(identificador_valido(&por_omision));
    }

    #[test]
    fn los_niveles_son_los_de_syslog() {
        // El diario los interpreta por número: equivocarlos hace que un error se
        // muestre como información y desaparezca de los filtros.
        assert_eq!((EMERGENCIA, CRITICO, ERROR, AVISO, INFORMATIVO, DEPURACION), (0, 2, 3, 4, 6, 7));
    }
}
