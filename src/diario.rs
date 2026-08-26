//! Escribir en el diario del sistema, con el nombre de la aplicación.
//!
//! El problema que resuelve: una aplicación gráfica que se abre desde el menú es
//! hija del compositor, así que lo que manda por `stderr` cae en el diario
//! atribuido a la unidad de la sesión. En la práctica eso significa que sus
//! mensajes no se pueden encontrar, y que cuando una se cae **no queda nada**:
//! sólo un volcado de núcleo sin símbolos, sin la línea del panic que diría qué
//! pasó.
//!
//! El protocolo nativo de `systemd-journald` es un datagrama a un socket unix con
//! pares `CAMPO=valor`. No hace falta ningún privilegio, no hace falta libsystemd,
//! y si el socket no está —un contenedor, una máquina sin systemd— se cae a
//! `stderr` sin molestar a nadie.

use std::io::Write;
use std::os::unix::net::UnixDatagram;
use std::sync::OnceLock;

/// El socket del diario. Datagrama, sin privilegios.
const SOCKET: &str = "/run/systemd/journal/socket";

/// Hasta dónde se recorta un mensaje.
///
/// Un datagrama no se parte en trozos: si no entra, se pierde entero. Y el
/// protocolo tiene una variante con descriptor de archivo para los mensajes
/// grandes que acá no hace falta, porque una línea de registro de dieciséis mil
/// caracteres no la lee nadie. Se recorta y se avisa en el propio texto.
pub const LIMITE_MENSAJE: usize = 16 * 1024;

/// Lo que se pone al final de un mensaje recortado, para que se vea que falta algo.
pub const MARCA_DE_RECORTE: &str = "… (recortado)";

/// De dónde salió el mensaje.
///
/// Sirve para lo que más se busca: separar lo que rompió la interfaz de lo que
/// rompió el núcleo de la aplicación, que se arreglan en archivos distintos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capa {
    Interfaz,
    Nucleo,
}

impl Capa {
    pub fn como_texto(self) -> &'static str {
        match self {
            Capa::Interfaz => "interfaz",
            Capa::Nucleo => "nucleo",
        }
    }
}

/// El nivel de syslog, que es el que entiende el diario.
///
/// Se acota en lugar de rechazarse: un nivel inventado no es motivo para perder el
/// mensaje, y perderlo sería justo lo contrario de lo que hace este plugin.
pub fn nivel_valido(nivel: u8) -> u8 {
    nivel.min(7)
}

/// Si un nombre de campo lo acepta el diario.
///
/// Mayúsculas, dígitos y `_`, sin empezar con `_` ni con un dígito: el guion bajo
/// al principio lo reserva `journald` para los campos que pone él —`_PID`, `_UID`,
/// `_COMM`— y son justamente los que no se deben poder falsificar.
pub fn nombre_de_campo_valido(nombre: &str) -> bool {
    !nombre.is_empty()
        && nombre.len() <= 64
        && !nombre.starts_with('_')
        && !nombre.starts_with(|c: char| c.is_ascii_digit())
        && nombre
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Recorta un mensaje sin partir un carácter al medio.
pub fn recortar(mensaje: &str, limite: usize) -> String {
    if mensaje.len() <= limite {
        return mensaje.to_string();
    }
    let sitio = limite.saturating_sub(MARCA_DE_RECORTE.len());
    // `floor_char_boundary` todavía no está estable, así que se busca a mano: un
    // corte en medio de un carácter deja bytes inválidos y el mensaje ilegible.
    let corte = (0..=sitio.min(mensaje.len()))
        .rev()
        .find(|i| mensaje.is_char_boundary(*i))
        .unwrap_or(0);
    format!("{}{}", &mensaje[..corte], MARCA_DE_RECORTE)
}

/// Codifica un par campo/valor con el protocolo nativo del diario.
///
/// **Acá está la parte que importa para la seguridad.** La forma corta es
/// `CAMPO=valor\n`, y con eso un valor que contenga un salto de línea puede
/// inventar campos: un mensaje con `\nPRIORITY=0\n` adentro convertiría un aviso
/// de la interfaz en una emergencia del sistema, y con `\nMESSAGE=…` se pueden
/// escribir entradas falsas a nombre de la aplicación. Por eso cualquier valor con
/// un salto de línea va en la forma larga —nombre, salto, ocho bytes de largo en
/// little endian, los datos crudos, salto— donde el largo manda y el contenido no
/// se interpreta.
pub fn codificar_campo(nombre: &str, valor: &str) -> Vec<u8> {
    let mut salida = Vec::with_capacity(nombre.len() + valor.len() + 10);
    if valor.contains('\n') {
        salida.extend_from_slice(nombre.as_bytes());
        salida.push(b'\n');
        salida.extend_from_slice(&(valor.len() as u64).to_le_bytes());
        salida.extend_from_slice(valor.as_bytes());
        salida.push(b'\n');
    } else {
        salida.extend_from_slice(nombre.as_bytes());
        salida.push(b'=');
        salida.extend_from_slice(valor.as_bytes());
        salida.push(b'\n');
    }
    salida
}

/// Arma el datagrama entero.
///
/// Los campos con nombre inválido se descartan en silencio en lugar de abortar: el
/// mensaje vale más que el campo extra, y quien los pasa es código nuestro.
pub fn armar(campos: &[(&str, String)]) -> Vec<u8> {
    let mut salida = Vec::new();
    for (nombre, valor) in campos {
        if nombre_de_campo_valido(nombre) {
            salida.extend_from_slice(&codificar_campo(nombre, valor));
        }
    }
    salida
}

/// Los campos de una entrada corriente.
pub fn campos_de(
    identificador: &str,
    capa: Capa,
    nivel: u8,
    mensaje: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("MESSAGE", recortar(mensaje, LIMITE_MENSAJE)),
        ("PRIORITY", nivel_valido(nivel).to_string()),
        ("SYSLOG_IDENTIFIER", identificador.to_string()),
        ("VSK_CAPA", capa.como_texto().to_string()),
    ]
}

/// El socket, abierto una vez.
///
/// `None` cuando no hay diario. No se reintenta: si el socket no estaba al
/// arrancar no va a aparecer, y reintentar en cada línea de registro costaría una
/// llamada al sistema fallida por mensaje.
fn socket() -> Option<&'static UnixDatagram> {
    static S: OnceLock<Option<UnixDatagram>> = OnceLock::new();
    S.get_or_init(|| {
        if !std::path::Path::new(SOCKET).exists() {
            return None;
        }
        UnixDatagram::unbound().ok()
    })
    .as_ref()
}

/// Manda una entrada al diario, o a `stderr` si no hay diario.
///
/// No devuelve error a propósito. Que falle el registro no es motivo para que falle
/// lo que se estaba haciendo, y una aplicación que se cae **por no poder anotar que
/// algo salió mal** sería una broma de mal gusto.
pub fn enviar(campos: &[(&str, String)]) {
    let datagrama = armar(campos);

    if let Some(sock) = socket() {
        if sock.send_to(&datagrama, SOCKET).is_ok() {
            return;
        }
    }

    // Sin diario, el mensaje va a `stderr`, que es donde iba antes de este plugin.
    let mensaje = campos
        .iter()
        .find(|(n, _)| *n == "MESSAGE")
        .map(|(_, v)| v.as_str())
        .unwrap_or("(sin mensaje)");
    let _ = writeln!(std::io::stderr(), "{mensaje}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_valor_con_salto_de_linea_no_puede_inventar_campos() {
        // Lo que este plugin no puede permitir: que un mensaje de la interfaz
        // escriba entradas falsas a nombre de la aplicación, o se ascienda a
        // emergencia del sistema.
        let malicioso = "parece inofensivo\nPRIORITY=0\nMESSAGE=el sistema se cae\n";
        let codificado = codificar_campo("MESSAGE", malicioso);

        // Forma larga: nombre, salto, ocho bytes de largo, datos, salto.
        assert!(codificado.starts_with(b"MESSAGE\n"));
        let largo = u64::from_le_bytes(codificado[8..16].try_into().unwrap());
        assert_eq!(largo as usize, malicioso.len());
        assert_eq!(&codificado[16..16 + malicioso.len()], malicioso.as_bytes());
        assert_eq!(*codificado.last().unwrap(), b'\n');

        // Y nunca la forma corta, que es la que dejaría inyectar.
        assert!(!codificado.starts_with(b"MESSAGE="));
    }

    #[test]
    fn un_valor_normal_va_en_la_forma_corta() {
        assert_eq!(codificar_campo("PRIORITY", "3"), b"PRIORITY=3\n".to_vec());
    }

    #[test]
    fn el_largo_de_la_forma_larga_cuenta_bytes_y_no_caracteres() {
        // Con caracteres de más de un byte, contando caracteres el diario leería
        // menos datos de los que hay y el resto se interpretaría como campos.
        let valor = "ñandú\nsigue";
        let c = codificar_campo("MESSAGE", valor);
        let largo = u64::from_le_bytes(c[8..16].try_into().unwrap());
        assert_eq!(largo as usize, valor.len());
        assert_ne!(largo as usize, valor.chars().count());
    }

    #[test]
    fn los_campos_reservados_del_diario_no_se_pueden_escribir() {
        // `_PID`, `_UID` y `_COMM` los pone `journald` y son los que hacen
        // confiable una entrada: si se pudieran falsificar, el diario dejaría de
        // servir para saber quién dijo qué.
        assert!(!nombre_de_campo_valido("_PID"));
        assert!(!nombre_de_campo_valido("_COMM"));
        assert!(!nombre_de_campo_valido("__CURSOR"));
    }

    #[test]
    fn un_nombre_de_campo_tiene_que_ser_el_que_el_diario_acepta() {
        assert!(nombre_de_campo_valido("MESSAGE"));
        assert!(nombre_de_campo_valido("VSK_CAPA"));
        assert!(nombre_de_campo_valido("CODE_LINE"));
        assert!(!nombre_de_campo_valido("mensaje"), "minúsculas");
        assert!(!nombre_de_campo_valido("MI CAMPO"), "espacio");
        assert!(!nombre_de_campo_valido("MI=CAMPO"), "igual");
        assert!(!nombre_de_campo_valido("MI\nCAMPO"), "salto");
        assert!(!nombre_de_campo_valido("1CAMPO"), "empieza con dígito");
        assert!(!nombre_de_campo_valido(""));
        assert!(!nombre_de_campo_valido(&"A".repeat(65)));
    }

    #[test]
    fn un_campo_con_nombre_invalido_no_arrastra_al_resto() {
        let d = armar(&[
            ("MESSAGE", "hola".to_string()),
            ("_PID", "1".to_string()),
            ("PRIORITY", "6".to_string()),
        ]);
        let texto = String::from_utf8_lossy(&d);
        assert!(texto.contains("MESSAGE=hola"));
        assert!(texto.contains("PRIORITY=6"));
        assert!(!texto.contains("_PID"));
    }

    #[test]
    fn el_nivel_se_acota_en_lugar_de_perder_el_mensaje() {
        assert_eq!(nivel_valido(3), 3);
        assert_eq!(nivel_valido(7), 7);
        assert_eq!(nivel_valido(9), 7);
        assert_eq!(nivel_valido(255), 7);
    }

    #[test]
    fn recortar_no_parte_un_caracter_al_medio() {
        // Un corte en medio de un carácter de varios bytes deja el mensaje
        // ilegible, que es peor que perder unas letras.
        let mensaje = "ñ".repeat(100);
        let corto = recortar(&mensaje, 20);
        assert!(corto.ends_with(MARCA_DE_RECORTE));
        assert!(corto.len() <= 20 + MARCA_DE_RECORTE.len());
        // Que sea una cadena válida es el punto de la prueba.
        assert!(corto.chars().all(|c| c == 'ñ' || MARCA_DE_RECORTE.contains(c)));
    }

    #[test]
    fn un_mensaje_que_entra_no_se_toca() {
        assert_eq!(recortar("corto", LIMITE_MENSAJE), "corto");
    }

    #[test]
    fn un_mensaje_recortado_lo_dice() {
        // Si no, un mensaje cortado parece un mensaje que termina ahí, y se busca
        // el problema en el lugar equivocado.
        let largo = "a".repeat(LIMITE_MENSAJE + 1);
        assert!(recortar(&largo, LIMITE_MENSAJE).ends_with(MARCA_DE_RECORTE));
    }

    #[test]
    fn una_entrada_corriente_lleva_lo_que_hace_falta_para_encontrarla() {
        let c = campos_de("vasak-terminal", Capa::Interfaz, 3, "algo falló");
        let texto = String::from_utf8_lossy(&armar(&c)).to_string();
        assert!(texto.contains("SYSLOG_IDENTIFIER=vasak-terminal"));
        assert!(texto.contains("PRIORITY=3"));
        assert!(texto.contains("VSK_CAPA=interfaz"));
        assert!(texto.contains("MESSAGE=algo falló"));
    }

    #[test]
    fn el_identificador_no_lo_elige_el_mensaje() {
        // Aunque el texto traiga saltos de línea, el identificador sigue siendo el
        // de la aplicación: es lo que hace que el selector de registros funcione.
        //
        // Lo que se comprueba es la **forma** del datagrama, no que el texto no
        // aparezca: en la forma larga el intento aparece igual, pero adentro del
        // bloque de datos, donde el largo manda y el diario no lo interpreta.
        let d = armar(&campos_de(
            "vasak-terminal",
            Capa::Interfaz,
            6,
            "x\nSYSLOG_IDENTIFIER=otro",
        ));
        assert!(d.starts_with(b"MESSAGE\n"), "el mensaje tiene que ir en forma larga");

        // El identificador de verdad, en forma corta, es el único que el diario va
        // a leer como campo.
        let esperado = b"SYSLOG_IDENTIFIER=vasak-terminal\n";
        assert!(
            d.windows(esperado.len()).any(|v| v == esperado),
            "falta el identificador de la aplicación"
        );

        // Y el intento queda dentro del largo declarado para MESSAGE.
        let largo = u64::from_le_bytes(d[8..16].try_into().unwrap()) as usize;
        let datos = &d[16..16 + largo];
        assert!(std::str::from_utf8(datos).unwrap().contains("SYSLOG_IDENTIFIER=otro"));
    }
}
