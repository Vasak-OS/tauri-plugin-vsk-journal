//! Lo que la interfaz puede pedir.
//!
//! Dos cosas y nada más: anotar una línea y preguntar con qué nombre se firma. El
//! identificador **no** se puede elegir desde acá — lo pone el plugin al arrancar.
//! Si la interfaz pudiera elegirlo, una página cargada en el WebView podría
//! escribir entradas a nombre de cualquier servicio del sistema.

use crate::{diario::Capa, Diario};
use tauri::State;

#[tauri::command]
pub fn registrar(diario: State<'_, Diario>, nivel: u8, mensaje: String) {
    // `Capa::Interfaz` fijo: lo que llega por aquí viene del WebView por
    // definición, y poder distinguirlo de lo que anota el núcleo es la mitad de
    // para qué sirve el campo.
    diario.registrar(Capa::Interfaz, nivel, &mensaje);
}

#[tauri::command]
pub fn identificador(diario: State<'_, Diario>) -> String {
    diario.identificador().to_string()
}
