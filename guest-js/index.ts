import { invoke } from '@tauri-apps/api/core';

/**
 * Registro en el diario del sistema, con el nombre de la aplicación.
 *
 * El identificador lo pone el lado Rust al arrancar y desde acá no se puede
 * elegir: si se pudiera, una página cargada en el WebView escribiría entradas a
 * nombre de cualquier servicio.
 */

/** Los niveles de syslog, que son los que entiende el diario. */
export const Level = {
	Emergency: 0,
	Alert: 1,
	Critical: 2,
	Error: 3,
	Warning: 4,
	Notice: 5,
	Info: 6,
	Debug: 7,
} as const;

export type LevelValue = (typeof Level)[keyof typeof Level];

/**
 * Hasta dónde se recorta un mensaje antes de mandarlo.
 *
 * El lado Rust recorta igual, pero mandarlo entero por el puente para que lo
 * recorten allá cuesta serializar megabytes por una línea de registro. Un objeto
 * enorme que alguien pasó por error a `console.error` es exactamente el caso.
 */
export const MESSAGE_LIMIT = 16 * 1024;

const TRIM_MARK = '… (recortado)';

/** Acota un nivel inventado en lugar de perder el mensaje. */
export function normalizeLevel(level: number): number {
	if (!Number.isFinite(level)) {
		return Level.Info;
	}
	return Math.min(7, Math.max(0, Math.trunc(level)));
}

/** Recorta un mensaje dejando claro que se recortó. */
export function truncate(message: string, limit = MESSAGE_LIMIT): string {
	if (message.length <= limit) {
		return message;
	}
	return message.slice(0, Math.max(0, limit - TRIM_MARK.length)) + TRIM_MARK;
}

/**
 * Convierte cualquier cosa en algo legible.
 *
 * Un `Error` aporta el `stack`, que es lo único que sirve para encontrar dónde
 * pasó. Un objeto suelto se serializa, y si no se puede —referencias circulares,
 * que en un evento del DOM son la norma— se cae al `String(...)` en lugar de tirar
 * una excepción dentro del propio registro.
 */
export function describe(value: unknown): string {
	if (value instanceof Error) {
		return value.stack ? `${value.name}: ${value.message}\n${value.stack}` : `${value.name}: ${value.message}`;
	}
	if (typeof value === 'string') {
		return value;
	}
	try {
		return JSON.stringify(value) ?? String(value);
	} catch {
		return String(value);
	}
}

/** El texto de un `ErrorEvent`, con el archivo y la línea si los trae. */
export function describeErrorEvent(event: {
	message?: string;
	filename?: string;
	lineno?: number;
	colno?: number;
	error?: unknown;
}): string {
	const donde = event.filename ? ` (${event.filename}:${event.lineno ?? 0}:${event.colno ?? 0})` : '';
	const detalle = event.error !== undefined && event.error !== null ? `\n${describe(event.error)}` : '';
	return `${event.message ?? 'error sin mensaje'}${donde}${detalle}`;
}

/** Anota una línea en el diario. */
export async function log(level: number, message: string): Promise<void> {
	await invoke('plugin:vsk-journal|registrar', {
		nivel: normalizeLevel(level),
		mensaje: truncate(message),
	});
}

export const error = (message: string) => log(Level.Error, message);
export const warn = (message: string) => log(Level.Warning, message);
export const info = (message: string) => log(Level.Info, message);
export const debug = (message: string) => log(Level.Debug, message);

/** Con qué nombre firma esta aplicación sus entradas. */
export async function getIdentifier(): Promise<string> {
	return await invoke('plugin:vsk-journal|identificador');
}

/** Lo mínimo que hace falta de `window` y de `console`, para poder probarlo. */
export interface CaptureTarget {
	addEventListener(tipo: string, escucha: (evento: unknown) => void): void;
	removeEventListener(tipo: string, escucha: (evento: unknown) => void): void;
}

export interface CaptureOptions {
	/** Qué escucha los eventos. Por omisión `window`. */
	target?: CaptureTarget;
	/** Si además se replica lo que va a `console.error` y `console.warn`. */
	console?: Console | null;
	/** Adónde se manda. Sólo para probar. */
	sink?: (level: number, message: string) => void;
}

/**
 * Manda al diario lo que rompe la interfaz.
 *
 * Un error de JavaScript no se ve: la pantalla queda a medias y no queda registro
 * de por qué. Esto engancha los dos que importan —`error` y
 * `unhandledrejection`— y devuelve la función que lo deshace.
 *
 * El reentrante está cuidado: si el propio registro falla y eso llama a
 * `console.error`, sin la guarda se llamaría a sí mismo hasta agotar la pila.
 */
export function captureFailures(options: CaptureOptions = {}): () => void {
	const target = options.target ?? (globalThis as unknown as CaptureTarget);
	const sink = options.sink ?? ((nivel: number, texto: string) => void log(nivel, texto));
	const consola = options.console === undefined ? globalThis.console : options.console;

	let dentro = false;
	const mandar = (nivel: number, texto: string) => {
		if (dentro) {
			return;
		}
		dentro = true;
		try {
			sink(nivel, texto);
		} finally {
			dentro = false;
		}
	};

	const alFallar = (evento: unknown) => {
		mandar(Level.Error, describeErrorEvent(evento as { message?: string }));
	};
	const alRechazar = (evento: unknown) => {
		const razon = (evento as { reason?: unknown }).reason;
		mandar(Level.Error, `promesa rechazada sin atender: ${describe(razon)}`);
	};

	target.addEventListener('error', alFallar);
	target.addEventListener('unhandledrejection', alRechazar);

	let restaurarConsola: (() => void) | null = null;
	if (consola) {
		const errorOriginal = consola.error;
		const avisoOriginal = consola.warn;
		consola.error = (...args: unknown[]) => {
			mandar(Level.Error, args.map(describe).join(' '));
			errorOriginal.apply(consola, args);
		};
		consola.warn = (...args: unknown[]) => {
			mandar(Level.Warning, args.map(describe).join(' '));
			avisoOriginal.apply(consola, args);
		};
		restaurarConsola = () => {
			consola.error = errorOriginal;
			consola.warn = avisoOriginal;
		};
	}

	return () => {
		target.removeEventListener('error', alFallar);
		target.removeEventListener('unhandledrejection', alRechazar);
		restaurarConsola?.();
	};
}
