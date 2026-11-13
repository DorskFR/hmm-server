import { ApiError, type Status } from './api.ts';

type BadgeTone = 'neutral' | 'ok' | 'warn' | 'danger' | 'info' | 'muted' | 'violet';

export const STATUSES: { id: Status; label: string; tone: BadgeTone }[] = [
	{ id: 'new', label: 'New', tone: 'info' },
	{ id: 'triaged', label: 'Triaged', tone: 'violet' },
	{ id: 'in_progress', label: 'In progress', tone: 'warn' },
	{ id: 'resolved', label: 'Resolved', tone: 'ok' },
	{ id: 'dismissed', label: 'Dismissed', tone: 'muted' },
];

export const OPEN_STATUSES: Status[] = ['new', 'triaged', 'in_progress'];

export const statusLabel = (status: string) =>
	STATUSES.find((s) => s.id === status)?.label ?? sentence(status);

export const statusTone = (status: string): BadgeTone =>
	STATUSES.find((s) => s.id === status)?.tone ?? 'neutral';

export const KINDS = ['feedback', 'error', 'alert'];

export const kindTone = (kind: string): BadgeTone =>
	kind === 'error' ? 'danger' : kind === 'alert' ? 'warn' : 'neutral';

export const SCOPES = [
	{ id: 'reports:read', label: 'Read reports', help: 'List and open reports and their events.' },
	{ id: 'attachments:read', label: 'Read screenshots', help: 'Download attached screenshots.' },
	{
		id: 'reports:triage',
		label: 'Triage',
		help: 'Change status, assignee and ticket; claim and release; add notes.',
	},
	{
		id: 'admin',
		label: 'Admin',
		help: 'Manage keys and webhooks, read the access log.',
	},
];

export const WEBHOOK_EVENTS = [
	{ id: 'report.created', label: 'Report created', help: 'A new report was stored.' },
	{
		id: 'report.status_changed',
		label: 'Status changed',
		help: 'A report moved to another status.',
	},
];

export function sentence(value: string): string {
	const text = value.replaceAll('_', ' ');
	return text.charAt(0).toUpperCase() + text.slice(1);
}

export interface Actor {
	name: string;
	role: 'system' | 'key' | 'admin' | 'other';
}

export function parseActor(actor: string): Actor {
	if (actor === 'system') return { name: 'hmm', role: 'system' };
	if (actor.startsWith('key:')) return { name: actor.slice(4), role: 'key' };
	if (actor.startsWith('admin:')) return { name: actor.slice(6), role: 'admin' };
	return { name: actor, role: 'other' };
}

export function pagePath(url: string | null): string {
	if (!url) return '';
	try {
		const parsed = new URL(url);
		return parsed.pathname + parsed.search;
	} catch {
		return url;
	}
}

export function formatBytes(bytes: number): string {
	if (bytes < 1000) return `${bytes} B`;
	if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} kB`;
	return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function errorText(error: unknown): string {
	if (error instanceof ApiError) {
		const text = sentence(error.message);
		return error.details.length ? `${text}: ${error.details.join(', ')}.` : text;
	}
	return error instanceof Error ? error.message : 'Something went wrong.';
}
