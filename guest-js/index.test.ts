import { describe, expect, it } from 'bun:test';
import {
	captureFailures,
	type CaptureTarget,
	describe as describir,
	describeErrorEvent,
	Level,
	MESSAGE_LIMIT,
	normalizeLevel,
	truncate,
} from './index';

/** Un `window` de mentira, para poder contar qué se enganchó y qué se soltó. */
function objetivoFalso() {
	const escuchas = new Map<string, ((evento: unknown) => void)[]>();
	const objetivo: CaptureTarget = {
		addEventListener(tipo, escucha) {
			escuchas.set(tipo, [...(escuchas.get(tipo) ?? []), escucha]);
		},
		removeEventListener(tipo, escucha) {
			escuchas.set(tipo, (escuchas.get(tipo) ?? []).filter((e) => e !== escucha));
		},
	};
	return {
		objetivo,
		cuantas: (tipo: string) => (escuchas.get(tipo) ?? []).length,
		disparar: (tipo: string, evento: unknown) => {
			for (const e of escuchas.get(tipo) ?? []) e(evento);
		},
	};
}

describe('normalizeLevel', () => {
	it('acota en lugar de perder el mensaje', () => {
		// Un nivel inventado no es motivo para perder la línea: perderla sería lo
		// contrario de para qué existe este plugin.
		expect(normalizeLevel(9)).toBe(7);
		expect(normalizeLevel(-1)).toBe(0);
		expect(normalizeLevel(3)).toBe(3);
	});

	it('sobrevive a lo que no es un número', () => {
		expect(normalizeLevel(Number.NaN)).toBe(Level.Info);
		expect(normalizeLevel(Number.POSITIVE_INFINITY)).toBe(Level.Info);
		expect(normalizeLevel(3.7)).toBe(3);
	});
});

describe('truncate', () => {
	it('no toca lo que entra', () => {
		expect(truncate('corto')).toBe('corto');
	});

	it('avisa cuando recorta', () => {
		// Un mensaje cortado en silencio parece un mensaje que termina ahí, y se
		// busca el problema en el lugar equivocado.
		const recortado = truncate('a'.repeat(MESSAGE_LIMIT + 10));
		expect(recortado.endsWith('(recortado)')).toBe(true);
		expect(recortado.length).toBeLessThanOrEqual(MESSAGE_LIMIT);
	});
});

describe('describe', () => {
	it('conserva la pila de un Error, que es lo único que ubica el problema', () => {
		const e = new Error('se rompió');
		const texto = describir(e);
		expect(texto).toContain('Error: se rompió');
		expect(texto).toContain('index.test.ts');
	});

	it('no explota con referencias circulares', () => {
		// En un evento del DOM son la norma; tirar una excepción dentro del propio
		// registro tapa el error que se estaba intentando anotar.
		const a: Record<string, unknown> = {};
		a.yo = a;
		expect(() => describir(a)).not.toThrow();
	});

	it('deja las cadenas como están', () => {
		expect(describir('tal cual')).toBe('tal cual');
	});
});

describe('describeErrorEvent', () => {
	it('lleva archivo y línea', () => {
		expect(
			describeErrorEvent({ message: 'x is not a function', filename: 'app.js', lineno: 12, colno: 3 })
		).toBe('x is not a function (app.js:12:3)');
	});

	it('agrega la pila cuando el evento la trae', () => {
		const texto = describeErrorEvent({ message: 'algo', error: new Error('la causa') });
		expect(texto).toContain('algo');
		expect(texto).toContain('la causa');
	});

	it('no se queda sin texto', () => {
		expect(describeErrorEvent({})).toContain('error sin mensaje');
	});
});

describe('captureFailures', () => {
	it('manda al diario un error de la interfaz', () => {
		const { objetivo, disparar } = objetivoFalso();
		const anotado: [number, string][] = [];
		captureFailures({ target: objetivo, console: null, sink: (n, m) => anotado.push([n, m]) });

		disparar('error', { message: 'reventó', filename: 'a.js', lineno: 1, colno: 1 });
		expect(anotado).toHaveLength(1);
		expect(anotado[0][0]).toBe(Level.Error);
		expect(anotado[0][1]).toContain('reventó');
	});

	it('atrapa una promesa rechazada sin atender', () => {
		// Es el caso más común y el que menos rastro deja: la pantalla queda a
		// medias y en la consola no lo ve nadie.
		const { objetivo, disparar } = objetivoFalso();
		const anotado: string[] = [];
		captureFailures({ target: objetivo, console: null, sink: (_n, m) => anotado.push(m) });

		disparar('unhandledrejection', { reason: new Error('sin catch') });
		expect(anotado[0]).toContain('promesa rechazada sin atender');
		expect(anotado[0]).toContain('sin catch');
	});

	it('suelta todo lo que enganchó', () => {
		// Sin esto, cada vez que se llama queda un escucha más y el mismo error se
		// anota varias veces.
		const { objetivo, cuantas } = objetivoFalso();
		const soltar = captureFailures({ target: objetivo, console: null, sink: () => {} });
		expect(cuantas('error')).toBe(1);
		expect(cuantas('unhandledrejection')).toBe(1);
		soltar();
		expect(cuantas('error')).toBe(0);
		expect(cuantas('unhandledrejection')).toBe(0);
	});

	it('replica la consola sin quedársela', () => {
		const { objetivo } = objetivoFalso();
		const vistos: unknown[][] = [];
		const consola = {
			error: (...args: unknown[]) => vistos.push(args),
			warn: (...args: unknown[]) => vistos.push(args),
		} as unknown as Console;
		const errorOriginal = consola.error;

		const anotado: number[] = [];
		const soltar = captureFailures({ target: objetivo, console: consola, sink: (n) => anotado.push(n) });

		consola.error('algo');
		expect(anotado).toEqual([Level.Error]);
		// Y la consola sigue mostrando lo suyo: esto agrega el diario, no reemplaza.
		expect(vistos).toHaveLength(1);

		soltar();
		expect(consola.error).toBe(errorOriginal);
	});

	it('no se llama a sí mismo hasta agotar la pila', () => {
		// Si el propio registro falla y eso va a `console.error`, sin la guarda se
		// entra de nuevo y el proceso se muere por desborde.
		const { objetivo } = objetivoFalso();
		const consola = { error: () => {}, warn: () => {} } as unknown as Console;
		let veces = 0;
		captureFailures({
			target: objetivo,
			console: consola,
			sink: () => {
				veces++;
				consola.error('el registro falló');
			},
		});

		consola.error('el error de verdad');
		expect(veces).toBe(1);
	});
});
