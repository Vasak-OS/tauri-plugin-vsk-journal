//! Que un panic deje rastro.
//!
//! Esto es lo que motivó el plugin. El 25/08/2026 `vasak-desktop` abortó y lo
//! único que quedó fue un volcado de núcleo sin símbolos: veinte marcos con `n/a`
//! y ni una palabra de qué pasó. El mensaje del panic **sí** se había escrito, pero
//! a `stderr`, y `stderr` no iba a ninguna parte que se pudiera leer después.
//!
//! Con el gancho puesto, ese mismo caso deja en el diario el mensaje, el archivo y
//! la línea, a nombre de la aplicación y con prioridad de crítico.

use crate::diario::{self, Capa};

/// Nivel de syslog para un panic: crítico.
///
/// Crítico y no error: un panic no es «una operación falló», es la aplicación
/// terminándose. Que se distinga de los errores corrientes importa cuando alguien
/// abre el diario para ver por qué el escritorio desapareció.
const NIVEL_CRITICO: u8 = 2;

/// El texto de un panic, con la ubicación si la hay.
///
/// Pura para poder probarla: el gancho corre mientras el proceso se está muriendo,
/// que es el peor momento para descubrir que el formato estaba mal.
pub fn descripcion(carga: &str, ubicacion: Option<&str>) -> String {
    match ubicacion {
        Some(donde) => format!("pánico en {donde}: {carga}"),
        None => format!("pánico: {carga}"),
    }
}

/// Saca el texto de la carga de un panic.
///
/// Los dos tipos que usa la biblioteca estándar: `&str` cuando el mensaje es una
/// literal y `String` cuando lleva formato. Cualquier otra cosa —un `panic_any`
/// con un tipo propio— no tiene texto que mostrar, pero el hecho de que hubo un
/// pánico igual se anota.
pub fn carga_de(carga: &(dyn std::any::Any + Send)) -> String {
    if let Some(texto) = carga.downcast_ref::<&str>() {
        (*texto).to_string()
    } else if let Some(texto) = carga.downcast_ref::<String>() {
        texto.clone()
    } else {
        "(sin texto)".to_string()
    }
}

/// Instala el gancho de pánico.
///
/// El gancho anterior se conserva y se llama después: sacarlo dejaría la consola
/// sin el mensaje, que es donde lo mira quien está desarrollando. Esto agrega el
/// diario, no reemplaza nada.
pub fn instalar(identificador: String) {
    let anterior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let ubicacion = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));

        let mut campos = diario::campos_de(
            &identificador,
            Capa::Nucleo,
            NIVEL_CRITICO,
            &descripcion(&carga_de(info.payload()), ubicacion.as_deref()),
        );

        // Los campos que `journald` documenta para ubicar código, así que
        // `journalctl -o verbose` los muestra donde se los espera.
        if let Some(lugar) = info.location() {
            campos.push(("CODE_FILE", lugar.file().to_string()));
            campos.push(("CODE_LINE", lugar.line().to_string()));
        }

        diario::enviar(&campos);
        anterior(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_descripcion_lleva_donde_pasó() {
        // Sin la ubicación, un `unwrap` en un proyecto con doscientos archivos no
        // se encuentra.
        assert_eq!(
            descripcion("called `Option::unwrap()` on a `None` value", Some("src/lib.rs:42")),
            "pánico en src/lib.rs:42: called `Option::unwrap()` on a `None` value"
        );
    }

    #[test]
    fn sin_ubicacion_igual_se_anota() {
        assert_eq!(descripcion("algo", None), "pánico: algo");
    }

    #[test]
    fn se_lee_la_carga_de_los_dos_tipos_de_panic() {
        // `panic!("literal")` deja un `&str`; `panic!("{}", x)` deja un `String`.
        let literal: Box<dyn std::any::Any + Send> = Box::new("una literal");
        let formateado: Box<dyn std::any::Any + Send> = Box::new(String::from("con formato"));
        assert_eq!(carga_de(literal.as_ref()), "una literal");
        assert_eq!(carga_de(formateado.as_ref()), "con formato");
    }

    #[test]
    fn una_carga_de_otro_tipo_no_pierde_el_aviso() {
        // `panic_any(42)` no tiene texto, pero que hubo un pánico importa igual.
        let raro: Box<dyn std::any::Any + Send> = Box::new(42u32);
        assert_eq!(carga_de(raro.as_ref()), "(sin texto)");
    }

    #[test]
    fn un_panic_es_critico_y_no_un_error_cualquiera() {
        // Un panic no es «una operación falló»: es la aplicación terminándose, y
        // que se distinga importa cuando alguien abre el diario para ver por qué el
        // escritorio desapareció. 2 es «crítico» en syslog; 3 es «error».
        assert_eq!(NIVEL_CRITICO, 2, "crítico, no error");
    }
}
