# tauri-plugin-vsk-journal

Registro en el diario del sistema para las aplicaciones de VasakOS, **con el nombre
de cada aplicación**.

## El problema

Una aplicación gráfica que se abre desde el menú es hija del compositor. Lo que
manda por `stderr` termina en el diario atribuido a la unidad de la sesión, no a
ella. Dos consecuencias, las dos medidas en una máquina real:

1. El selector de aplicaciones de `vasak-monitor` lista el terminal, los ajustes y
   la galería, y para todas dice «sin entradas». No porque no escriban: porque lo
   que escriben no lleva su nombre y no hay forma de filtrarlo.
2. El 25/08/2026 `vasak-desktop` abortó. Lo único que quedó fue un volcado de
   núcleo sin símbolos: veinte marcos con `n/a`. El mensaje del panic **sí** se
   había escrito, a un `stderr` que después no se podía leer. El crash quedó sin
   diagnosticar.

## Qué hace

Habla el protocolo nativo de `systemd-journald`: un datagrama a
`/run/systemd/journal/socket` con pares `CAMPO=valor`. Sin privilegios, sin
`libsystemd`, sin dependencias nuevas. Si no hay socket —un contenedor, una máquina
sin systemd— se cae a `stderr` y no molesta a nadie.

- Cada entrada lleva `SYSLOG_IDENTIFIER` con el nombre de la aplicación, así que
  `journalctl -t vasak-terminal` y el selector del monitor la encuentran.
- `VSK_CAPA` dice si el mensaje salió de la interfaz o del núcleo, que se arreglan
  en archivos distintos.
- El gancho de pánico deja el mensaje, el archivo y la línea con prioridad de
  crítico. El caso de arriba, con esto puesto, se diagnostica leyendo el diario.

## Uso

### Rust

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_vsk_journal::init())
```

Firma con el nombre del ejecutable, que es el que coincide con lo que el diario ya
guarda en `_COMM`. Para elegir otro, `init_con_identificador("vasak-terminal")`.

```rust
use tauri_plugin_vsk_journal::DiarioExt;

app.diario().error("no se pudo abrir la configuración");
app.diario().informacion("arrancó con el tema oscuro");
```

### Interfaz

```ts
import { captureFailures, error, getIdentifier } from '@vasakgroup/plugin-vsk-journal';

// Lo que rompe la interfaz deja de ser invisible.
const soltar = captureFailures();

await error('el usuario no tiene permisos para esa carpeta');
```

`captureFailures()` engancha `error` y `unhandledrejection`, y replica lo que va a
`console.error` y `console.warn`. Devuelve la función que lo deshace. La consola
sigue mostrando lo suyo: esto agrega el diario, no reemplaza nada.

## Seguridad

Lo único que este plugin no puede permitir es que un mensaje escriba entradas
falsas. La forma corta del protocolo es `CAMPO=valor\n`, así que un valor con un
salto de línea adentro podría inventar campos: un mensaje con `\nPRIORITY=0\n`
convertiría un aviso de la interfaz en una emergencia del sistema, y con
`\nSYSLOG_IDENTIFIER=sshd\n` se escribirían entradas a nombre de otro servicio.

Cualquier valor con un salto de línea va en la forma larga —nombre, salto, ocho
bytes de largo en little endian, los datos crudos, salto— donde el largo manda y el
contenido no se interpreta. Comprobado contra el diario de verdad: el intento queda
como texto dentro del mensaje, con la prioridad y el identificador que se pidieron.

Además:

- Los campos que empiezan con `_` —`_PID`, `_UID`, `_COMM`— los pone `journald` y
  son los que hacen confiable una entrada. Se rechazan.
- La interfaz elige el nivel y el texto. **El identificador no**: lo pone el lado
  Rust al arrancar. Si se pudiera elegir desde el WebView, una página cargada ahí
  escribiría a nombre de cualquier servicio del sistema.
- El nivel se acota en lugar de rechazarse, y los mensajes se recortan avisando:
  perder la línea sería lo contrario de para qué existe esto.

## Pruebas

```bash
cargo test          # 20 unitarias
bun test            # 15 de la interfaz
cargo test -- --ignored   # contra el journald de la máquina
```

Las dos últimas necesitan un diario de verdad, así que están marcadas `ignore`: en
CI no hay socket y no habría nada que comprobar. Se corren a mano cuando se toca el
protocolo, que es donde un error no lo ve ningún test unitario — los campos pueden
estar perfectos y el diario rechazar el datagrama entero.

## Licencia

GPLv3.
