import type { IncomingMessage } from 'node:http';
import type { Plugin } from 'vite';
import { createMockApi } from './api.ts';

async function readJson(req: IncomingMessage): Promise<unknown> {
	const chunks: Buffer[] = [];
	for await (const chunk of req) chunks.push(chunk as Buffer);
	const text = Buffer.concat(chunks).toString();
	try {
		return text ? JSON.parse(text) : undefined;
	} catch {
		return undefined;
	}
}

/** Serves `/v1/admin/*` from in-memory fixtures; `POST /__mock/reset` restores them. */
export function mockApi(): Plugin {
	const mock = createMockApi();
	return {
		name: 'hmm-mock-api',
		configureServer(server) {
			server.middlewares.use(async (req, res, next) => {
				const url = req.url ?? '/';
				if (!url.startsWith('/v1/') && !url.startsWith('/__mock/')) return next();
				const out = mock.handle(req.method ?? 'GET', url, await readJson(req));
				if (!out) return next();
				res.statusCode = out.status;
				if (out.png) {
					res.setHeader('content-type', 'image/png');
					res.setHeader('cache-control', 'private, max-age=3600');
					res.end(out.png);
				} else if (out.json !== undefined) {
					res.setHeader('content-type', 'application/json');
					res.end(JSON.stringify(out.json));
				} else {
					res.end();
				}
			});
		},
	};
}
