import { deflateSync } from 'node:zlib';

type Rgb = [number, number, number];

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
	let c = n;
	for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
	return c >>> 0;
});

function crc32(buf: Buffer): number {
	let c = 0xffffffff;
	for (const byte of buf) c = (CRC_TABLE[(c ^ byte) & 0xff] as number) ^ (c >>> 8);
	return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Buffer): Buffer {
	const head = Buffer.alloc(8);
	head.writeUInt32BE(data.length, 0);
	head.write(type, 4, 'ascii');
	const crc = Buffer.alloc(4);
	crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])), 0);
	return Buffer.concat([head, data, crc]);
}

/** A flat mock-up of an app page (header, sidebar, rows, one outlined element), as a PNG. */
export function fakeScreenshot(width: number, height: number, variant: number): Buffer {
	const dark = variant % 2 === 0;
	const bg: Rgb = dark ? [24, 26, 32] : [246, 246, 244];
	const panel: Rgb = dark ? [33, 36, 44] : [255, 255, 255];
	const line: Rgb = dark ? [52, 56, 68] : [222, 222, 218];
	const ink: Rgb = dark ? [112, 118, 134] : [150, 150, 146];
	const accents: Rgb[] = [
		[94, 160, 232],
		[226, 122, 96],
		[120, 190, 132],
		[176, 142, 236],
	];
	const accent = accents[variant % accents.length] as Rgb;

	const stride = width * 3 + 1;
	const raw = Buffer.alloc(stride * height);
	const rect = (x: number, y: number, w: number, h: number, c: Rgb) => {
		const x1 = Math.min(width, Math.max(0, Math.round(x + w)));
		const y1 = Math.min(height, Math.max(0, Math.round(y + h)));
		for (let py = Math.max(0, Math.round(y)); py < y1; py++) {
			let o = py * stride + 1 + Math.max(0, Math.round(x)) * 3;
			for (let px = Math.max(0, Math.round(x)); px < x1; px++) {
				raw[o++] = c[0];
				raw[o++] = c[1];
				raw[o++] = c[2];
			}
		}
	};
	const outline = (x: number, y: number, w: number, h: number, c: Rgb, t: number) => {
		rect(x, y, w, t, c);
		rect(x, y + h - t, w, t, c);
		rect(x, y, t, h, c);
		rect(x + w - t, y, t, h, c);
	};

	const u = width / 100;
	const phone = width < 600;
	const side = phone ? 0 : 18 * u;
	const head = phone ? 12 * u : 4 * u;
	rect(0, 0, width, height, bg);
	rect(0, 0, width, head, panel);
	rect(0, head, width, 1, line);
	rect(2 * u, head * 0.35, (phone ? 24 : 8) * u, head * 0.3, ink);
	rect(width - (phone ? 14 : 5) * u, head * 0.3, (phone ? 10 : 3) * u, head * 0.4, accent);
	if (!phone) {
		rect(0, head + 1, side, height, panel);
		rect(side, head, 1, height, line);
		for (let i = 0; i < 7; i++) {
			rect(2 * u, head + (3 + i * 4.2) * u, (8 + ((i * 5 + variant) % 6)) * u, 1.1 * u, ink);
		}
		rect(0, head + (2.2 + (variant % 5) * 4.2) * u, 0.4 * u, 2.8 * u, accent);
	}

	const left = side + (phone ? 4 : 3) * u;
	const cardW = width - left - (phone ? 4 : 3) * u;
	const rowH = (phone ? 16 : 5.4) * u;
	const gap = (phone ? 3 : 1.2) * u;
	const top = head + (phone ? 6 : 3) * u;
	const rows = Math.floor((height - top) / (rowH + gap));
	const picked = variant % Math.max(1, Math.min(rows, 5));
	for (let i = 0; i < rows; i++) {
		const y = top + i * (rowH + gap);
		rect(left, y, cardW, rowH, panel);
		outline(left, y, cardW, rowH, line, 1);
		rect(
			left + 2 * u,
			y + rowH * 0.28,
			cardW * (0.25 + ((i * 7 + variant) % 5) * 0.07),
			rowH * 0.16,
			ink,
		);
		rect(
			left + 2 * u,
			y + rowH * 0.6,
			cardW * (0.4 + ((i * 3 + variant) % 4) * 0.08),
			rowH * 0.1,
			line,
		);
		rect(
			left + cardW - (phone ? 16 : 7) * u,
			y + rowH * 0.34,
			(phone ? 12 : 5) * u,
			rowH * 0.3,
			i === picked ? accent : line,
		);
		if (i === picked)
			outline(left - 0.5 * u, y - 0.5 * u, cardW + u, rowH + u, accent, Math.max(2, 0.25 * u));
	}

	const ihdr = Buffer.alloc(13);
	ihdr.writeUInt32BE(width, 0);
	ihdr.writeUInt32BE(height, 4);
	ihdr[8] = 8;
	ihdr[9] = 2;
	return Buffer.concat([
		Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
		chunk('IHDR', ihdr),
		chunk('IDAT', deflateSync(raw)),
		chunk('IEND', Buffer.alloc(0)),
	]);
}
