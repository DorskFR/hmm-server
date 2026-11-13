export type Status = 'new' | 'triaged' | 'in_progress' | 'resolved' | 'dismissed';
export type Scope = 'reports:read' | 'attachments:read' | 'reports:triage' | 'admin';
export type WebhookEvent = 'report.created' | 'report.status_changed' | 'ping';
export type DeliveryState = 'pending' | 'delivered' | 'failed';

export interface Claim {
	by: string;
	until: string;
}

export interface ReportSummary {
	project: string;
	id: string;
	kind: string;
	category: string | null;
	comment: string;
	page_url: string | null;
	client_name: string | null;
	client_version: string | null;
	created_at: string;
	received_at: string;
	status: Status;
	assignee: string | null;
	ticket: string | null;
	claim: Claim | null;
	attachments: number;
}

export interface Attachment {
	id: string;
	mime: string;
	width: number;
	height: number;
	bytes: number;
}

export interface ReportEvent {
	at: string;
	actor: string;
	type: string;
	data: Record<string, unknown>;
}

export interface Report extends Omit<ReportSummary, 'attachments'> {
	origin: string | null;
	envelope: unknown;
	attachments: Attachment[];
	events: ReportEvent[];
}

export interface ProjectCounts {
	project: string;
	counts: Record<Status, number>;
}

export interface Me {
	actor: string;
	scopes: string[];
	projects: string[];
}

export interface ApiKey {
	id: string;
	name: string;
	prefix: string;
	projects: string[];
	scopes: string[];
	created_at: string;
	expires_at: string | null;
	revoked_at: string | null;
	last_used_at: string | null;
}

export interface AccessEntry {
	at: string;
	key_id: string;
	key_name: string;
	method: string;
	path: string;
	status: number;
	remote_addr: string | null;
}

export interface Webhook {
	id: string;
	name: string;
	project: string;
	url: string;
	events: string[];
	enabled: boolean;
	created_at: string;
	last_delivery: { at: string; status_code: number | null; error: string | null } | null;
}

export interface Delivery {
	id: string;
	webhook_id: string;
	webhook_name: string;
	event: string;
	project: string | null;
	report_id: string | null;
	state: DeliveryState;
	attempts: number;
	next_attempt_at: string | null;
	last_status_code: number | null;
	last_error: string | null;
	last_duration_ms: number | null;
	created_at: string;
	updated_at: string;
}

export class ApiError extends Error {
	readonly status: number;
	readonly details: string[];

	constructor(status: number, message: string, details: string[] = []) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.details = details;
	}
}

const BASE = '/v1/admin';

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
	let res: Response;
	try {
		res = await fetch(BASE + path, {
			method,
			headers: {
				accept: 'application/json',
				...(body === undefined ? {} : { 'content-type': 'application/json' }),
			},
			body: body === undefined ? undefined : JSON.stringify(body),
		});
	} catch {
		throw new ApiError(0, 'The server could not be reached.');
	}
	if (res.status === 204) return undefined as T;
	const text = await res.text();
	let json: unknown;
	try {
		json = text ? JSON.parse(text) : undefined;
	} catch {
		json = undefined;
	}
	if (!res.ok) {
		const err = (json ?? {}) as { error?: unknown; details?: unknown };
		const message = typeof err.error === 'string' ? err.error : `Request failed (${res.status}).`;
		const details = Array.isArray(err.details) ? err.details.map(String) : [];
		throw new ApiError(res.status, message, details);
	}
	return json as T;
}

function query(params: Record<string, string | number | null | undefined>): string {
	const search = new URLSearchParams();
	for (const [key, value] of Object.entries(params)) {
		if (value !== null && value !== undefined && value !== '') search.set(key, String(value));
	}
	const s = search.toString();
	return s ? `?${s}` : '';
}

const reportPath = (project: string, id: string) =>
	`/reports/${encodeURIComponent(project)}/${encodeURIComponent(id)}`;

export interface ReportFilter {
	project?: string | null;
	status?: string | null;
	kind?: string | null;
	q?: string | null;
}

export const api = {
	me: () => request<Me>('GET', '/me'),
	projects: () => request<{ projects: ProjectCounts[] }>('GET', '/projects'),

	reports: (filter: ReportFilter, cursor: string | null, limit: number) =>
		request<{ reports: ReportSummary[]; next_cursor: string | null }>(
			'GET',
			`/reports${query({ ...filter, cursor, limit })}`,
		),
	report: (project: string, id: string) => request<Report>('GET', reportPath(project, id)),
	patchReport: (
		project: string,
		id: string,
		patch: { status?: Status; assignee?: string | null; ticket?: string | null },
	) => request<Report>('PATCH', reportPath(project, id), patch),
	releaseClaim: (project: string, id: string) =>
		request<void>('DELETE', `${reportPath(project, id)}/claim`),
	addNote: (project: string, id: string, note: string) =>
		request<ReportEvent>('POST', `${reportPath(project, id)}/events`, { note }),
	attachmentUrl: (project: string, id: string, attachment: string) =>
		`${BASE}${reportPath(project, id)}/attachments/${encodeURIComponent(attachment)}`,

	keys: () => request<{ keys: ApiKey[] }>('GET', '/keys'),
	createKey: (body: { name: string; projects: string[]; scopes: string[]; expires_at?: string }) =>
		request<ApiKey & { token: string }>('POST', '/keys', body),
	revokeKey: (id: string) => request<void>('DELETE', `/keys/${encodeURIComponent(id)}`),

	access: (key: string | null, cursor: string | null, limit: number) =>
		request<{ entries: AccessEntry[]; next_cursor: string | null }>(
			'GET',
			`/access${query({ key, cursor, limit })}`,
		),

	webhooks: () => request<{ webhooks: Webhook[] }>('GET', '/webhooks'),
	createWebhook: (body: { name: string; project: string; url: string; events: string[] }) =>
		request<Webhook & { secret: string }>('POST', '/webhooks', body),
	patchWebhook: (
		id: string,
		patch: { name?: string; url?: string; events?: string[]; enabled?: boolean },
	) => request<Webhook>('PATCH', `/webhooks/${encodeURIComponent(id)}`, patch),
	deleteWebhook: (id: string) => request<void>('DELETE', `/webhooks/${encodeURIComponent(id)}`),
	pingWebhook: (id: string) =>
		request<Delivery>('POST', `/webhooks/${encodeURIComponent(id)}/ping`),
	deliveries: (webhook: string | null, cursor: string | null, limit: number) =>
		request<{ deliveries: Delivery[]; next_cursor: string | null }>(
			'GET',
			`/deliveries${query({ webhook, cursor, limit })}`,
		),
	redeliver: (id: string) =>
		request<Delivery>('POST', `/deliveries/${encodeURIComponent(id)}/redeliver`),
};
