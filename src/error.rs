//! Los errores del plugin.
//!
//! Son pocos a propósito: escribir en el diario no falla hacia afuera. Que no se
//! pueda anotar algo no es motivo para que falle lo que se estaba haciendo, así que
//! el camino de registro no devuelve error — se cae a `stderr` y sigue.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// El identificador que se pidió no sirve para el diario.
    #[error("identificador inválido: {0}")]
    Identificador(String),
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(
        &self,
        serializador: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializador.serialize_str(self.to_string().as_ref())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
