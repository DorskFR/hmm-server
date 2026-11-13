import type {
	AccessEntry,
	ApiKey,
	Delivery,
	Report,
	ReportEvent,
	Status,
	Webhook,
} from '../src/lib/api.ts';

export interface MockState {
	actor: string;
	policyProjects: string[];
	reports: Report[];
	keys: ApiKey[];
	access: AccessEntry[];
	webhooks: Webhook[];
	deliveries: Delivery[];
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

function prng(seed: number): () => number {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

const HEX = '0123456789abcdef';

interface Seed {
	project: string;
	kind: string;
	category: string | null;
	comment: string;
	path: string;
	title: string;
}

const SITES: Record<string, { origin: string; version: string }> = {
	cctui: { origin: 'https://cctui.dorsk.dev', version: '0.30.0' },
	tsumikit: { origin: 'https://dorskfr.github.io', version: '0.83.0' },
	'hmm-demo': { origin: 'https://hmm-demo.dorsk.dev', version: '0.2.0' },
};

const SEEDS: Seed[] = [
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'problem',
		comment:
			'The session list jumps back to the top every time a new message arrives, so I lose my place while reading older sessions.\n\nHappens on the phone and on desktop. Scrolling down again works until the next message.',
		path: '/sessions/d3cc439d',
		title: 'Session d3cc439d',
	},
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'idea',
		comment: 'Could the usage panel show the reset time in my local timezone instead of UTC?',
		path: '/usage',
		title: 'Usage',
	},
	{
		project: 'tsumikit',
		kind: 'feedback',
		category: 'problem',
		comment:
			'FilterChips collapses to the dropdown form one chip too early at 390px: there is still room for "Dismissed".',
		path: '/tsumikit/molecules/filter-chips',
		title: 'FilterChips',
	},
	{
		project: 'cctui',
		kind: 'error',
		category: null,
		comment: "TypeError: Cannot read properties of undefined (reading 'id')",
		path: '/sessions/8b1f0c22/files',
		title: 'Files',
	},
	{
		project: 'hmm-demo',
		kind: 'feedback',
		category: 'question',
		comment: 'Is the screenshot taken before or after I close the picker overlay?',
		path: '/demo',
		title: 'hmm demo',
	},
	{
		project: 'cctui',
		kind: 'alert',
		category: null,
		comment: 'Worker heartbeat missed three times in a row for machine sakura.',
		path: '/machines/sakura',
		title: 'sakura',
	},
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'problem',
		comment: 'The attach button overlaps the send button when the composer wraps to three lines.',
		path: '/sessions/41aa07be',
		title: 'Session 41aa07be',
	},
	{
		project: 'tsumikit',
		kind: 'feedback',
		category: 'idea',
		comment: 'A `Timeline` molecule would save every app from rebuilding the same event list.',
		path: '/tsumikit/components',
		title: 'Components',
	},
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'other',
		comment: 'Love the new diff viewer. Nothing to fix, just saying thanks.',
		path: '/sessions/41aa07be/diff',
		title: 'Diff',
	},
	{
		project: 'hmm-demo',
		kind: 'feedback',
		category: 'problem',
		comment: 'Element picker selects the whole card when I click the badge inside it.',
		path: '/demo/cards',
		title: 'Cards',
	},
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'question',
		comment: 'Where did the export button go? It used to be in the session header.',
		path: '/sessions/99e0d1c4',
		title: 'Session 99e0d1c4',
	},
	{
		project: 'tsumikit',
		kind: 'feedback',
		category: 'problem',
		comment: 'DataTable stacked cards lose the row tone bar in the sepia theme.',
		path: '/tsumikit/organisms/data-table',
		title: 'DataTable',
	},
	{
		project: 'cctui',
		kind: 'error',
		category: null,
		comment: 'Unhandled rejection: WebSocket closed with code 1006 while replaying history.',
		path: '/sessions/d3cc439d',
		title: 'Session d3cc439d',
	},
	{
		project: 'cctui',
		kind: 'feedback',
		category: 'idea',
		comment: 'Keyboard shortcut to jump between pinned sessions, something like g then a digit.',
		path: '/',
		title: 'Sessions',
	},
	{
		project: 'tsumikit',
		kind: 'feedback',
		category: 'question',
		comment: 'Which token should a consumer use for a hairline between two toolbars?',
		path: '/tsumikit/tokens',
		title: 'Tokens',
	},
	{
		project: 'hmm-demo',
		kind: 'feedback',
		category: 'idea',
		comment: 'Let me redact part of the screenshot before sending it.',
		path: '/demo',
		title: 'hmm demo',
	},
];

const STATUS_BY_AGE: Status[] = [
	'new',
	'new',
	'triaged',
	'new',
	'in_progress',
	'triaged',
	'new',
	'resolved',
	'in_progress',
	'dismissed',
	'resolved',
	'triaged',
	'resolved',
	'resolved',
	'dismissed',
	'resolved',
];

export function createState(now = Date.now()): MockState {
	const rand = prng(20261113);
	const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d+Z$/, 'Z');
	const hex = (n: number) => Array.from({ length: n }, () => HEX[Math.floor(rand() * 16)]).join('');
	const uuid = () => `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;

	const webhooks: Webhook[] = [
		{
			id: uuid(),
			name: 'cctui-dispatch',
			project: 'cctui',
			url: 'https://cctui.dorsk.dev/api/hooks/hmm',
			events: ['report.created'],
			enabled: true,
			created_at: iso(now - 41 * DAY),
			last_delivery: { at: iso(now - 12 * MIN), status_code: 200, error: null },
		},
		{
			id: uuid(),
			name: 'tsumikit-notify',
			project: 'tsumikit',
			url: 'https://ntfy.dorsk.dev/hmm-tsumikit',
			events: ['report.created', 'report.status_changed'],
			enabled: true,
			created_at: iso(now - 19 * DAY),
			last_delivery: { at: iso(now - 3 * HOUR), status_code: 503, error: null },
		},
		{
			id: uuid(),
			name: 'staging-echo',
			project: 'hmm-demo',
			url: 'https://echo.staging.dorsk.dev/hooks/hmm',
			events: ['report.created'],
			enabled: false,
			created_at: iso(now - 6 * DAY),
			last_delivery: null,
		},
	];

	const reports: Report[] = [];
	const deliveries: Delivery[] = [];
	const total = 68;
	for (let i = 0; i < total; i++) {
		const seed = SEEDS[i % SEEDS.length] as Seed;
		const site = SITES[seed.project] as { origin: string; version: string };
		const age = i === 0 ? 4 * MIN : Math.round((i ** 1.7 * 37 + rand() * 30) * MIN);
		const received = now - age;
		const status: Status =
			i < STATUS_BY_AGE.length
				? (STATUS_BY_AGE[i] as Status)
				: (['resolved', 'resolved', 'dismissed', 'triaged', 'resolved'][i % 5] as Status);
		const id = i === 0 ? 'r_8f2a91' : `r_${hex(6)}`;
		const shots = seed.kind === 'feedback' ? ([2, 1, 1, 0, 1, 3][i % 6] ?? 0) : 0;
		const phone = i % 4 === 2;
		const attachments = Array.from({ length: shots }, (_, n) => ({
			id: `a${n + 1}`,
			mime: 'image/png',
			width: phone ? 390 : 1440,
			height: phone ? 844 : 900,
			bytes: 60_000 + Math.floor(rand() * 340_000),
		}));
		const pageUrl = `${site.origin}${seed.path}`;
		const events: ReportEvent[] = [
			{
				at: iso(received),
				actor: 'system',
				type: 'received',
				data: { attachments: shots, bytes: attachments.reduce((n, a) => n + a.bytes, 2_100) },
			},
		];
		const hook = webhooks.find((w) => w.project === seed.project && w.enabled);
		if (hook) {
			const ok = hook.name === 'cctui-dispatch' || i % 3 !== 0;
			events.push({
				at: iso(received + 2_000),
				actor: 'system',
				type: 'webhook',
				data: {
					webhook: hook.name,
					event: 'report.created',
					attempt: 1,
					status_code: ok ? 200 : 503,
					error: null,
				},
			});
			if (i < 40) {
				deliveries.push({
					id: uuid(),
					webhook_id: hook.id,
					webhook_name: hook.name,
					event: 'report.created',
					project: seed.project,
					report_id: id,
					state: ok ? 'delivered' : i < 20 ? 'pending' : 'failed',
					attempts: ok ? 1 : i < 20 ? 2 : 6,
					next_attempt_at: !ok && i < 20 ? iso(now + 21 * MIN) : null,
					last_status_code: ok ? 200 : i % 2 ? 503 : null,
					last_error: ok ? null : i % 2 ? null : 'connection timed out after 10 s',
					last_duration_ms: ok ? 40 + Math.floor(rand() * 180) : i % 2 ? 912 : 10_000,
					created_at: iso(received + 1_000),
					updated_at: iso(ok ? received + 2_000 : Math.min(now - MIN, received + 36 * MIN)),
				});
			}
		}
		let assignee: string | null = null;
		let ticket: string | null = null;
		let claim: Report['claim'] = null;
		let at = received + 9 * MIN;
		const step = (actor: string, type: string, data: Record<string, unknown>) => {
			events.push({ at: iso(Math.min(at, now - 30_000)), actor, type, data });
			at += 14 * MIN;
		};
		if (status !== 'new') step('admin:macbook', 'status', { from: 'new', to: 'triaged' });
		if (status === 'in_progress' || status === 'resolved') {
			const agent = seed.project === 'tsumikit' ? 'key:tsumikit-triage' : 'key:cctui-agent';
			step(agent, 'claimed', { until: iso(at + 30 * MIN) });
			step(agent, 'status', { from: 'triaged', to: 'in_progress' });
			assignee = 'claude';
			step(agent, 'assigned', { from: null, to: assignee });
			ticket = `${seed.project === 'tsumikit' ? 'TSU' : seed.project === 'cctui' ? 'CCT' : 'HMM'}-${200 + i}`;
			step(agent, 'ticket', { from: null, to: ticket });
			if (status === 'in_progress') {
				claim = { by: agent, until: iso(now + (26 - (i % 5) * 3) * MIN) };
			} else {
				step(agent, 'note', {
					note: 'Reproduced, fixed and covered by a test. The fix ships with the next release.',
				});
				step(agent, 'released', {});
				step(agent, 'status', { from: 'in_progress', to: 'resolved' });
			}
		}
		if (status === 'dismissed') {
			step('admin:macbook', 'note', { note: 'Works as intended, nothing to change here.' });
			step('admin:macbook', 'status', { from: 'triaged', to: 'dismissed' });
		}
		if (i === 0) claim = { by: 'key:cctui-agent', until: iso(now + 28 * MIN) };
		if (i === 0) {
			events.push({
				at: iso(received + MIN),
				actor: 'key:cctui-agent',
				type: 'claimed',
				data: { until: claim?.until },
			});
		}

		reports.push({
			project: seed.project,
			id,
			kind: seed.kind,
			category: seed.category,
			comment: seed.comment,
			page_url: pageUrl,
			client_name: '@dorsk/hmm',
			client_version: '0.2.0',
			created_at: iso(received - 1_000),
			received_at: iso(received),
			status,
			assignee,
			ticket,
			claim,
			origin: site.origin,
			attachments,
			events,
			envelope: {
				v: 1,
				id,
				kind: seed.kind,
				...(seed.category ? { category: seed.category } : {}),
				project: seed.project,
				createdAt: iso(received - 1_000),
				client: { name: '@dorsk/hmm', version: '0.2.0' },
				page: {
					url: pageUrl,
					title: seed.title,
					viewport: phone ? { width: 390, height: 844 } : { width: 1440, height: 900 },
					userAgent: phone
						? 'Mozilla/5.0 (iPhone; CPU iPhone OS 19_1 like Mac OS X) AppleWebKit/605.1.15 Version/19.1 Mobile/15E148 Safari/604.1'
						: 'Mozilla/5.0 (X11; Linux x86_64; rv:145.0) Gecko/20100101 Firefox/145.0',
					language: 'en-GB',
				},
				context: { app: seed.project, version: site.version, route: seed.path },
				comment: seed.comment,
				items: attachments.map((a, n) => ({
					id: `i${n + 1}`,
					type: 'screenshot',
					url: pageUrl,
					comment: '',
					attachment: a.id,
				})),
				attachments: attachments.map((a) => ({ id: a.id, mime: a.mime })),
			},
		});
	}

	const key = (
		name: string,
		projects: string[],
		scopes: string[],
		created: number,
		rest: Partial<ApiKey> = {},
	): ApiKey => ({
		id: uuid(),
		name,
		prefix: Array.from(
			{ length: 8 },
			() => 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'[Math.floor(rand() * 57)],
		).join(''),
		projects,
		scopes,
		created_at: iso(now - created),
		expires_at: null,
		revoked_at: null,
		last_used_at: null,
		...rest,
	});
	const keys: ApiKey[] = [
		key(
			'cctui-agent',
			['cctui'],
			['reports:read', 'attachments:read', 'reports:triage'],
			30 * DAY,
			{
				expires_at: iso(now + 60 * DAY),
				last_used_at: iso(now - 3 * MIN),
			},
		),
		key(
			'tsumikit-triage',
			['tsumikit'],
			['reports:read', 'attachments:read', 'reports:triage'],
			19 * DAY,
			{
				last_used_at: iso(now - 5 * HOUR),
			},
		),
		key('dashboard', ['*'], ['reports:read'], 3 * DAY),
		key(
			'summer-intern',
			['hmm-demo', 'tsumikit'],
			['reports:read', 'attachments:read'],
			120 * DAY,
			{
				expires_at: iso(now - 5 * DAY),
				last_used_at: iso(now - 9 * DAY),
			},
		),
		key('old-ci', ['*'], ['reports:read', 'admin'], 200 * DAY, {
			revoked_at: iso(now - 20 * DAY),
			last_used_at: iso(now - 21 * DAY),
		}),
	];

	const access: AccessEntry[] = [];
	const callers = [keys[0], keys[0], keys[1], keys[0], keys[3], keys[0], keys[1]] as ApiKey[];
	for (let i = 0; i < 140; i++) {
		const caller = callers[i % callers.length] as ApiKey;
		const report = reports[(i * 5) % reports.length] as Report;
		const base = `/v1/admin/reports/${report.project}/${report.id}`;
		const calls: [string, string, number][] = [
			['GET', '/v1/admin/reports?status=open&limit=50', 200],
			['GET', base, 200],
			['GET', `${base}/attachments/a1`, 200],
			['POST', `${base}/claim`, i % 9 === 4 ? 409 : 200],
			['PATCH', base, 200],
			['POST', `${base}/events`, 201],
			['GET', '/v1/admin/keys', 403],
			['GET', base, i % 8 === 7 ? 404 : 200],
		];
		const [method, path, status] = calls[i % calls.length] as [string, string, number];
		const expired = caller.name === 'summer-intern';
		access.push({
			at: iso(now - (expired ? 9 * DAY : 0) - Math.round((i ** 1.5 * 2.2 + 3) * MIN)),
			key_id: caller.id,
			key_name: caller.name,
			method,
			path,
			status,
			remote_addr: caller.name === 'cctui-agent' ? '10.42.3.17' : '10.42.1.8',
		});
	}
	access.sort((a, b) => b.at.localeCompare(a.at));
	deliveries.sort((a, b) => b.created_at.localeCompare(a.created_at));

	return {
		actor: 'admin:macbook',
		policyProjects: ['cctui', 'tsumikit', 'hmm-demo', 'nanori'],
		reports,
		keys,
		access,
		webhooks,
		deliveries,
	};
}
