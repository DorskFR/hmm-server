import { randomBytes, randomUUID } from 'node:crypto';
import type { ApiKey, Delivery, Report, ReportSummary, Status, Webhook } from '../src/lib/api.ts';
import { createState, type MockState } from './fixtures.ts';
import { fakeScreenshot } from './png.ts';

export interface MockResponse {
	status: number;
	json?: unknown;
	png?: Buffer;
}

const STATUSES: Status[] = ['new', 'triaged', 'in_progress', 'resolved', 'dismissed'];
const OPEN: Status[] = ['new', 'triaged', 'in_progress'];
const SCOPES = ['reports:read', 'attachments:read', 'reports:triage', 'admin'];
const EVENTS = ['report.created', 'report.status_changed'];
const ALPHANUM = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';

const now = () => new Date().toISOString().replace(/\.\d+Z$/, 'Z');
const token = (n: number) =>
	Array.from(randomBytes(n), (b) => ALPHANUM[b % ALPHANUM.length]).join('');
const invalid = (details: string[]): MockResponse => ({
	status: 400,
	json: { error: 'validation failed', details },
});
const notFound: MockResponse = { status: 404, json: { error: 'not found' } };

function page<T>(items: T[], params: URLSearchParams, fallback = 50) {
	const limit = Math.min(200, Math.max(1, Number(params.get('limit')) || fallback));
	const offset = Number(Buffer.from(params.get('cursor') ?? '', 'base64url').toString() || 0);
	const next = offset + limit < items.length ? offset + limit : null;
	return {
		items: items.slice(offset, offset + limit),
		next_cursor: next === null ? null : Buffer.from(String(next)).toString('base64url'),
	};
}

function summary(report: Report): ReportSummary {
	const { origin, envelope, events, attachments, ...rest } = report;
	return { ...rest, claim: liveClaim(report), attachments: attachments.length };
}

function liveClaim(report: Report) {
	return report.claim && Date.parse(report.claim.until) > Date.now() ? report.claim : null;
}

function detail(report: Report): Report {
	return {
		...report,
		claim: liveClaim(report),
		events: [...report.events].sort((a, b) => a.at.localeCompare(b.at)),
	};
}

export function createMockApi() {
	let state: MockState = createState();

	function settle(delivery: Delivery) {
		setTimeout(() => {
			const hook = state.webhooks.find((w) => w.id === delivery.webhook_id);
			if (!hook || !state.deliveries.includes(delivery) || delivery.state !== 'pending') return;
			const ok = hook.name !== 'tsumikit-notify';
			delivery.attempts += 1;
			delivery.state = ok ? 'delivered' : 'pending';
			delivery.last_status_code = ok ? 200 : 503;
			delivery.last_error = null;
			delivery.last_duration_ms = ok ? 84 : 640;
			delivery.next_attempt_at = ok ? null : new Date(Date.now() + 60_000).toISOString();
			delivery.updated_at = now();
			hook.last_delivery = {
				at: delivery.updated_at,
				status_code: delivery.last_status_code,
				error: null,
			};
		}, 400);
	}

	function reports(params: URLSearchParams): MockResponse {
		const project = params.get('project');
		const kind = params.get('kind');
		const q = params.get('q')?.toLowerCase();
		const wanted = new Set(
			params
				.getAll('status')
				.flatMap((s) => (s === 'open' ? OPEN : s.split(',').map((x) => x as Status))),
		);
		const rows = state.reports
			.filter(
				(r) =>
					(!project || r.project === project) &&
					(!kind || r.kind === kind) &&
					(wanted.size === 0 || wanted.has(r.status)) &&
					(!q ||
						r.comment.toLowerCase().includes(q) ||
						(r.page_url ?? '').toLowerCase().includes(q) ||
						r.id.toLowerCase().includes(q)),
			)
			.sort((a, b) => b.received_at.localeCompare(a.received_at));
		const { items, next_cursor } = page(rows, params);
		return { status: 200, json: { reports: items.map(summary), next_cursor } };
	}

	function patchReport(report: Report, body: Record<string, unknown>): MockResponse {
		const errors: string[] = [];
		if ('status' in body && !STATUSES.includes(body.status as Status))
			errors.push('status is unknown');
		for (const field of ['assignee', 'ticket'] as const) {
			if (!(field in body)) continue;
			if (body[field] === '') body[field] = null;
			if (body[field] !== null && typeof body[field] !== 'string') {
				errors.push(`${field} must be a string or null`);
			} else if (typeof body[field] === 'string' && body[field].length > 200) {
				errors.push(`${field} holds at most 200 characters`);
			}
		}
		if (errors.length) return invalid(errors);
		const change = (type: string, from: unknown, to: unknown) =>
			report.events.push({ at: now(), actor: state.actor, type, data: { from, to } });
		if ('status' in body && body.status !== report.status) {
			change('status', report.status, body.status);
			report.status = body.status as Status;
		}
		if ('assignee' in body && body.assignee !== report.assignee) {
			change('assigned', report.assignee, body.assignee);
			report.assignee = body.assignee as string | null;
		}
		if ('ticket' in body && body.ticket !== report.ticket) {
			change('ticket', report.ticket, body.ticket);
			report.ticket = body.ticket as string | null;
		}
		return { status: 200, json: detail(report) };
	}

	function createKey(body: Record<string, unknown>): MockResponse {
		const errors: string[] = [];
		const name = typeof body.name === 'string' ? body.name.trim() : '';
		const projects = Array.isArray(body.projects) ? body.projects.map(String) : [];
		const scopes = Array.isArray(body.scopes) ? body.scopes.map(String) : [];
		if (!name) errors.push('name is required');
		if (projects.length === 0) errors.push('projects must not be empty');
		if (scopes.length === 0) errors.push('scopes must not be empty');
		if (scopes.some((s) => !SCOPES.includes(s))) errors.push('scopes has an unknown scope');
		if (errors.length) return invalid(errors);
		if (state.keys.some((k) => k.name === name && !k.revoked_at)) {
			return { status: 409, json: { error: 'a key with this name already exists' } };
		}
		const key: ApiKey = {
			id: randomUUID(),
			name,
			prefix: token(8),
			projects,
			scopes,
			created_at: now(),
			expires_at: typeof body.expires_at === 'string' ? body.expires_at : null,
			revoked_at: null,
			last_used_at: null,
		};
		state.keys.unshift(key);
		return { status: 201, json: { ...key, token: `hmm_${key.prefix}_${token(32)}` } };
	}

	function webhookErrors(body: Record<string, unknown>, creating: boolean): string[] {
		const errors: string[] = [];
		if (creating || 'name' in body) {
			if (typeof body.name !== 'string' || !body.name.trim()) errors.push('name is required');
		}
		if (creating && (typeof body.project !== 'string' || !body.project)) {
			errors.push('project is required');
		}
		if (creating || 'url' in body) {
			if (typeof body.url !== 'string' || !/^https:\/\/[^\s/]+/.test(body.url)) {
				errors.push('url must be an https URL');
			} else if (new URL(body.url).hostname.endsWith('.invalid')) {
				errors.push('url host does not resolve');
			}
		}
		if (creating || 'events' in body) {
			const events = Array.isArray(body.events) ? body.events : [];
			if (events.length === 0) errors.push('events must not be empty');
			if (events.some((e) => !EVENTS.includes(String(e))))
				errors.push('events has an unknown event');
		}
		return errors;
	}

	function handle(method: string, url: string, body: unknown): MockResponse | null {
		const { pathname, searchParams: params } = new URL(url, 'http://mock');
		if (method === 'POST' && pathname === '/__mock/reset') {
			state = createState();
			return { status: 204 };
		}
		if (!pathname.startsWith('/v1/admin/')) return null;
		const seg = pathname.slice('/v1/admin/'.length).split('/').map(decodeURIComponent);
		const data = (body && typeof body === 'object' ? body : {}) as Record<string, unknown>;
		const route = `${method} ${seg[0]}`;

		if (route === 'GET me') {
			return { status: 200, json: { actor: state.actor, scopes: SCOPES, projects: ['*'] } };
		}

		if (route === 'GET projects') {
			const names = [...new Set([...state.policyProjects, ...state.reports.map((r) => r.project)])];
			const projects = names.map((project) => ({
				project,
				counts: Object.fromEntries(
					STATUSES.map((s) => [
						s,
						state.reports.filter((r) => r.project === project && r.status === s).length,
					]),
				),
			}));
			return { status: 200, json: { projects } };
		}

		if (seg[0] === 'reports') {
			if (seg.length === 1) return method === 'GET' ? reports(params) : notFound;
			const report = state.reports.find((r) => r.project === seg[1] && r.id === seg[2]);
			if (!report) return notFound;
			if (seg.length === 3 && method === 'GET') return { status: 200, json: detail(report) };
			if (seg.length === 3 && method === 'PATCH') return patchReport(report, data);
			if (seg[3] === 'attachments' && method === 'GET') {
				const index = report.attachments.findIndex((a) => a.id === seg[4]);
				const attachment = report.attachments[index];
				if (!attachment) return notFound;
				const variant = state.reports.indexOf(report) * 3 + index;
				return { status: 200, png: fakeScreenshot(attachment.width, attachment.height, variant) };
			}
			if (seg[3] === 'claim' && method === 'DELETE') {
				if (liveClaim(report)) {
					report.events.push({ at: now(), actor: state.actor, type: 'released', data: {} });
				}
				report.claim = null;
				return { status: 204 };
			}
			if (seg[3] === 'claim' && method === 'POST') {
				const held = liveClaim(report);
				if (held && held.by !== state.actor) {
					return { status: 409, json: { error: 'already claimed', claim: held } };
				}
				const lease = Number(data.lease_seconds ?? 1800);
				report.claim = {
					by: state.actor,
					until: new Date(Date.now() + lease * 1000).toISOString(),
				};
				report.events.push({
					at: now(),
					actor: state.actor,
					type: 'claimed',
					data: { until: report.claim.until },
				});
				return { status: 200, json: detail(report) };
			}
			if (seg[3] === 'events' && method === 'POST') {
				const note = typeof data.note === 'string' ? data.note : '';
				if (note.length < 1 || note.length > 4000)
					return invalid(['note must be 1 to 4000 characters']);
				const event = { at: now(), actor: state.actor, type: 'note', data: { note } };
				report.events.push(event);
				return { status: 201, json: event };
			}
			return notFound;
		}

		if (seg[0] === 'keys') {
			if (route === 'GET keys' && seg.length === 1)
				return { status: 200, json: { keys: state.keys } };
			if (route === 'POST keys' && seg.length === 1) return createKey(data);
			if (route === 'DELETE keys' && seg.length === 2) {
				const key = state.keys.find((k) => k.id === seg[1]);
				if (!key) return notFound;
				key.revoked_at ??= now();
				return { status: 204 };
			}
			return notFound;
		}

		if (route === 'GET access') {
			const key = params.get('key');
			const rows = state.access.filter((e) => !key || e.key_id === key);
			const { items, next_cursor } = page(rows, params);
			return { status: 200, json: { entries: items, next_cursor } };
		}

		if (seg[0] === 'webhooks') {
			if (seg.length === 1 && method === 'GET') {
				return { status: 200, json: { webhooks: state.webhooks } };
			}
			if (seg.length === 1 && method === 'POST') {
				const errors = webhookErrors(data, true);
				if (errors.length) return invalid(errors);
				const hook: Webhook = {
					id: randomUUID(),
					name: String(data.name).trim(),
					project: String(data.project),
					url: String(data.url),
					events: (data.events as unknown[]).map(String),
					enabled: true,
					created_at: now(),
					last_delivery: null,
				};
				state.webhooks.push(hook);
				return { status: 201, json: { ...hook, secret: `whsec_${token(40)}` } };
			}
			const hook = state.webhooks.find((w) => w.id === seg[1]);
			if (!hook) return notFound;
			if (seg.length === 2 && method === 'PATCH') {
				const errors = webhookErrors(data, false);
				if (errors.length) return invalid(errors);
				if (typeof data.name === 'string') hook.name = data.name.trim();
				if (typeof data.url === 'string') hook.url = data.url;
				if (Array.isArray(data.events)) hook.events = data.events.map(String);
				if (typeof data.enabled === 'boolean') hook.enabled = data.enabled;
				if (data.enabled === false) {
					for (const d of state.deliveries) {
						if (d.webhook_id === hook.id && d.state === 'pending') {
							d.state = 'failed';
							d.next_attempt_at = null;
							d.updated_at = now();
						}
					}
				}
				return { status: 200, json: hook };
			}
			if (seg.length === 2 && method === 'DELETE') {
				state.webhooks = state.webhooks.filter((w) => w !== hook);
				state.deliveries = state.deliveries.filter((d) => d.webhook_id !== hook.id);
				return { status: 204 };
			}
			if (seg[2] === 'ping' && method === 'POST') {
				const delivery: Delivery = {
					id: randomUUID(),
					webhook_id: hook.id,
					webhook_name: hook.name,
					event: 'ping',
					project: hook.project,
					report_id: null,
					state: 'pending',
					attempts: 0,
					next_attempt_at: now(),
					last_status_code: null,
					last_error: null,
					last_duration_ms: null,
					created_at: now(),
					updated_at: now(),
				};
				state.deliveries.unshift(delivery);
				settle(delivery);
				return { status: 202, json: delivery };
			}
			return notFound;
		}

		if (seg[0] === 'deliveries') {
			if (seg.length === 1 && method === 'GET') {
				const hook = params.get('webhook');
				const rows = state.deliveries.filter((d) => !hook || d.webhook_id === hook);
				const { items, next_cursor } = page(rows, params);
				return { status: 200, json: { deliveries: items, next_cursor } };
			}
			const delivery = state.deliveries.find((d) => d.id === seg[1]);
			if (!delivery) return notFound;
			if (seg[2] === 'redeliver' && method === 'POST') {
				delivery.state = 'pending';
				delivery.next_attempt_at = now();
				delivery.updated_at = now();
				settle(delivery);
				return { status: 202, json: delivery };
			}
		}

		return notFound;
	}

	return { handle };
}
